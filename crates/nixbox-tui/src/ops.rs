use std::time::Duration;

use anyhow::Result;
use nixbox_config::Target;
use nixbox_nix::{
    build::{home_manager_switch_cmd, nixos_rebuild_switch_cmd, rebuild, BuildEvent},
    manifest::ManagedFile,
    scan::{remove_from_source, ScanTarget},
};
use tokio::sync::mpsc;
use tokio::time::sleep;

use crate::app::{App, AppEvent, InstalledCursor, QueuedOp, Tab};

pub(crate) fn write_manifest(app: &App) -> Result<ManagedFile> {
    let managed = ManagedFile::new(app.config.managed_file());
    match app.config.target {
        Target::HomeManager => managed.write_home_manager(&app.manifest)?,
        Target::NixosSystem => managed.write_nixos(&app.manifest)?,
    }
    Ok(managed)
}

pub(crate) fn spawn_rebuild(app: &mut App, tx: &mpsc::Sender<AppEvent>, action_label: String) {
    app.build_in_progress = true;
    app.current_op_label = Some(action_label.clone());
    app.log.clear();
    app.status = format!("{}...", action_label);

    let target = app.config.target;
    let config_dir = app.config.home_manager_dir();
    let app_tx = tx.clone();
    tokio::spawn(async move {
        let (build_tx, mut build_rx) = mpsc::channel::<BuildEvent>(64);
        let forward_tx = app_tx.clone();
        let forwarder = tokio::spawn(async move {
            while let Some(ev) = build_rx.recv().await {
                if forward_tx.send(AppEvent::Build(ev)).await.is_err() {
                    break;
                }
            }
        });
        let cmd_owned;
        let args_owned;
        let (cmd, args): (&str, Vec<&str>) = match target {
            Target::HomeManager => {
                let (c, a) = home_manager_switch_cmd(&config_dir);
                cmd_owned = c;
                args_owned = a;
                (cmd_owned.as_str(), args_owned.iter().map(|s| s.as_str()).collect())
            }
            Target::NixosSystem => {
                let (c, a) = nixos_rebuild_switch_cmd(&config_dir);
                cmd_owned = c;
                args_owned = a;
                (cmd_owned.as_str(), args_owned.iter().map(|s| s.as_str()).collect())
            }
        };
        if let Err(e) = rebuild(cmd, &args, build_tx.clone()).await {
            let _ = build_tx
                .send(BuildEvent::Finished(Err(e.to_string())))
                .await;
        }
        drop(build_tx);
        let _ = forwarder.await;
    });
}

pub(crate) fn drain_queue(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    if app.build_in_progress {
        return;
    }
    let Some(op) = app.queue.pop_front() else { return };
    let label = op.label();
    if let Err(e) = apply_op_to_manifest(app, &op) {
        app.status = format!("{}: failed to write manifest: {}", label, e);
        drain_queue(app, tx);
        return;
    }
    if !app.visible_tabs().contains(&app.tab) {
        app.tab = Tab::Search;
    }
    spawn_rebuild(app, tx, label);
}

fn apply_op_to_manifest(app: &mut App, op: &QueuedOp) -> Result<()> {
    match op {
        QueuedOp::Install(hit) => {
            app.manifest.add(&hit.attr);
        }
        QueuedOp::Uninstall(pname) => {
            app.manifest.remove(pname);
            let total = app.installed_total();
            if total == 0 {
                app.installed_selected = 0;
            } else if app.installed_selected >= total {
                app.installed_selected = total - 1;
            }
        }
        QueuedOp::Migrate(names) => {
            let scan_target = match app.config.target {
                Target::HomeManager => ScanTarget::HomeManager,
                Target::NixosSystem => ScanTarget::Nixos,
            };
            let source = app.config.main_file();
            let removed = remove_from_source(&source, scan_target, names)?;
            for name in &removed {
                app.manifest.add(name);
            }
            app.external_packages.retain(|p| !removed.contains(&p.name));
            let total = app.installed_total();
            if total == 0 {
                app.installed_selected = 0;
            } else if app.installed_selected >= total {
                app.installed_selected = total - 1;
            }
        }
    }
    let managed = write_manifest(app)?;
    app.log.push(format!(
        "Wrote {} ({}). {}...",
        managed.path().display(),
        app.config.target.label(),
        op.label(),
    ));
    Ok(())
}

pub(crate) async fn install_selected(app: &mut App, tx: &mpsc::Sender<AppEvent>) -> Result<()> {
    let Some(hit) = app.results.get(app.selected).cloned() else {
        app.status = "No selection.".into();
        return Ok(());
    };

    if app.manifest.packages.contains(&hit.attr)
        || app.queue.iter().any(|op| matches!(op, QueuedOp::Install(h) if h.attr == hit.attr))
    {
        app.status = format!("{} already tracked or queued.", hit.attr);
        return Ok(());
    }

    let attr = hit.attr.clone();
    app.queue.push_back(QueuedOp::Install(hit));
    if app.build_in_progress {
        app.status = format!("Queued install: {}.", attr);
        app.tab = Tab::Queue;
    } else {
        app.tab = Tab::Building;
        drain_queue(app, tx);
    }
    Ok(())
}

pub(crate) async fn uninstall_selected(
    app: &mut App,
    tx: &mpsc::Sender<AppEvent>,
) -> Result<()> {
    let packages = app.installed_packages();
    let Some(pname) = packages.get(app.installed_selected).cloned() else {
        app.status = "No selection.".into();
        return Ok(());
    };

    if app.queue.iter().any(|op| matches!(op, QueuedOp::Uninstall(p) if *p == pname)) {
        app.status = format!("{} already queued for removal.", pname);
        return Ok(());
    }

    app.queue.push_back(QueuedOp::Uninstall(pname.clone()));
    if app.build_in_progress {
        app.status = format!("Queued remove: {}.", pname);
        app.tab = Tab::Queue;
    } else {
        app.tab = Tab::Building;
        drain_queue(app, tx);
    }
    Ok(())
}

pub(crate) async fn migrate_selected(app: &mut App, tx: &mpsc::Sender<AppEvent>) -> Result<()> {
    let cursor = match app.installed_cursor() {
        Some(c) => c,
        None => {
            app.status = "No selection.".into();
            return Ok(());
        }
    };
    let ep = match cursor {
        InstalledCursor::External(ep) => ep,
        InstalledCursor::Managed(name) => {
            app.status =
                format!("{} is already managed — press d to uninstall.", name);
            return Ok(());
        }
    };
    if !ep.migratable {
        app.status = format!(
            "{} is on a same-line list in {}; remove it manually.",
            ep.name, ep.source_attr,
        );
        return Ok(());
    }
    let name = ep.name.clone();
    if app.queue.iter().any(|op| matches!(op, QueuedOp::Migrate(ns) if ns.contains(&name))) {
        app.status = format!("{} already queued for migration.", name);
        return Ok(());
    }
    app.queue.push_back(QueuedOp::Migrate(vec![name.clone()]));
    if app.build_in_progress {
        app.status = format!("Queued migrate: {}.", name);
        app.tab = Tab::Queue;
    } else {
        app.tab = Tab::Building;
        drain_queue(app, tx);
    }
    Ok(())
}

pub(crate) async fn migrate_all(app: &mut App, tx: &mpsc::Sender<AppEvent>) -> Result<()> {
    let names: Vec<String> = app
        .external_packages
        .iter()
        .filter(|ep| ep.migratable)
        .map(|ep| ep.name.clone())
        .collect();
    if names.is_empty() {
        app.status = "No migratable external packages found.".into();
        return Ok(());
    }
    let label = format!("migrate {} packages", names.len());
    app.queue.push_back(QueuedOp::Migrate(names));
    if app.build_in_progress {
        app.status = format!("Queued {}.", label);
        app.tab = Tab::Queue;
    } else {
        app.tab = Tab::Building;
        drain_queue(app, tx);
    }
    Ok(())
}

pub(crate) fn schedule_search(app: &mut App, tx: mpsc::Sender<AppEvent>) {
    let query = app.input.value().to_string();
    if query.is_empty() {
        return;
    }
    app.searching = true;
    app.search_epoch += 1;
    let epoch = app.search_epoch;
    let channel = app.config.channel.clone();
    app.latest_query = query.clone();

    tokio::spawn(async move {
        sleep(Duration::from_millis(180)).await;
        match nixbox_nix::search::search(&channel, &query).await {
            Ok(hits) => {
                let _ = tx.send(AppEvent::SearchDone { epoch, hits }).await;
            }
            Err(e) => {
                let _ = tx
                    .send(AppEvent::SearchFailed {
                        epoch,
                        error: e.to_string(),
                    })
                    .await;
            }
        }
    });
}
