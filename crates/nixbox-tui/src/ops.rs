use std::time::Duration;

use anyhow::Result;
use nixbox_config::Target;
use nixbox_nix::{
    build::{
        flake_has_home_configuration, home_manager_switch_cmd, nixos_rebuild_switch_cmd, rebuild,
        BuildEvent,
    },
    manifest::{ensure_imported, ImportStatus, ManagedFile},
    scan::{remove_from_source, ScanTarget},
};
use tokio::sync::mpsc;
use tokio::time::sleep;

use crate::app::{App, AppEvent, InstalledCursor, QueuedOp, Tab};

fn scope_to_scan_target(scope: Target) -> ScanTarget {
    match scope {
        Target::HomeManager => ScanTarget::HomeManager,
        Target::NixosSystem => ScanTarget::Nixos,
    }
}

pub(crate) fn write_manifest(app: &App, scope: Target) -> Result<ManagedFile> {
    let managed = ManagedFile::new(app.config.managed_file_for(scope));
    match scope {
        Target::HomeManager => managed.write_home_manager(app.manifest_for(scope))?,
        Target::NixosSystem => managed.write_nixos(app.manifest_for(scope))?,
    }
    Ok(managed)
}

/// Runs `git add -- <path>` if `path` lives inside a git work tree.
/// Silent no-op when git isn't installed or the path isn't tracked-eligible.
/// Nix flakes refuse to evaluate files that are present on disk but untracked,
/// so this keeps newly-written managed files visible to the rebuild.
fn git_track(path: &std::path::Path) -> Option<String> {
    let parent = path.parent()?;
    let inside = std::process::Command::new("git")
        .arg("-C")
        .arg(parent)
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !inside.status.success()
        || std::str::from_utf8(&inside.stdout).map(|s| s.trim()) != Ok("true")
    {
        return None;
    }
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(parent)
        .args(["add", "--intent-to-add", "--"])
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()?;
    if out.status.success() {
        Some(format!("git add -N {}", path.display()))
    } else {
        Some(format!(
            "Warning: `git add` failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Notes describing any change that should be surfaced to the user.
fn ensure_imported_note(
    main_file: &std::path::Path,
    managed: &ManagedFile,
) -> Option<String> {
    match ensure_imported(main_file, managed.path()) {
        Ok(ImportStatus::AlreadyImported) => None,
        Ok(ImportStatus::InsertedIntoList) => Some(format!(
            "Added import of {} to {}.",
            managed.path().display(),
            main_file.display(),
        )),
        Ok(ImportStatus::CreatedList) => Some(format!(
            "Created imports list in {} and added {}.",
            main_file.display(),
            managed.path().display(),
        )),
        Ok(ImportStatus::MainFileMissing) => Some(format!(
            "Warning: {} not found; you must import {} manually.",
            main_file.display(),
            managed.path().display(),
        )),
        Err(e) => Some(format!(
            "Warning: could not auto-import {} into {}: {}",
            managed.path().display(),
            main_file.display(),
            e,
        )),
    }
}

pub(crate) fn spawn_rebuild(
    app: &mut App,
    tx: &mpsc::Sender<AppEvent>,
    scope: Target,
    action_label: String,
) {
    app.build_in_progress = true;
    app.current_op_label = Some(action_label.clone());
    app.log.clear();
    app.status = format!("{}...", action_label);

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
        let (cmd, args): (&str, Vec<&str>) = match scope {
            Target::HomeManager => {
                // If the flake doesn't expose a standalone `homeConfigurations.<user>`
                // output, the user wires home-manager in as a NixOS module — apply
                // the change via `nixos-rebuild` instead.
                let (c, a) = if flake_has_home_configuration(&config_dir).await {
                    home_manager_switch_cmd(&config_dir)
                } else {
                    let _ = app_tx
                        .send(AppEvent::Build(BuildEvent::Line(
                            "No standalone homeConfigurations found; applying via nixos-rebuild."
                                .into(),
                        )))
                        .await;
                    nixos_rebuild_switch_cmd(&config_dir)
                };
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
    let scope = op.scope();
    if let Err(e) = apply_op_to_manifest(app, &op) {
        app.status = format!("{}: failed to write manifest: {}", label, e);
        drain_queue(app, tx);
        return;
    }
    if !app.visible_tabs().contains(&app.tab) {
        app.tab = Tab::Search;
    }
    spawn_rebuild(app, tx, scope, label);
}

fn apply_op_to_manifest(app: &mut App, op: &QueuedOp) -> Result<()> {
    let scope = op.scope();
    match op {
        QueuedOp::Install { hit, .. } => {
            app.manifest_for_mut(scope).add(&hit.attr);
        }
        QueuedOp::Uninstall { name, .. } => {
            app.manifest_for_mut(scope).remove(name);
        }
        QueuedOp::Migrate { names, .. } => {
            let source = app.config.main_file_for(scope);
            let removed = remove_from_source(&source, scope_to_scan_target(scope), names)?;
            for name in &removed {
                app.manifest_for_mut(scope).add(name);
            }
            // Drop any externals that just moved into the manifest.
            app.external_packages.retain(|ep| {
                !(ep_target_eq(ep.scope, scope) && removed.contains(&ep.name))
            });
        }
    }
    let managed = write_manifest(app, scope)?;
    app.log.push(format!(
        "Wrote {} ({}). {}...",
        managed.path().display(),
        scope.label(),
        op.label(),
    ));
    let main_file = app.config.main_file_for(scope);
    if let Some(note) = ensure_imported_note(&main_file, &managed) {
        app.log.push(note);
    }
    // Flakes ignore untracked files — make sure git sees the managed file
    // (and the main config if we just touched it).
    if let Some(note) = git_track(managed.path()) {
        app.log.push(note);
    }
    if main_file.exists() {
        if let Some(note) = git_track(&main_file) {
            app.log.push(note);
        }
    }
    let total = app.installed_total();
    if total == 0 {
        app.installed_selected = 0;
    } else if app.installed_selected >= total {
        app.installed_selected = total - 1;
    }
    Ok(())
}

fn ep_target_eq(scan_scope: ScanTarget, target: Target) -> bool {
    matches!(
        (scan_scope, target),
        (ScanTarget::HomeManager, Target::HomeManager) | (ScanTarget::Nixos, Target::NixosSystem)
    )
}

pub(crate) async fn install_selected(app: &mut App, tx: &mpsc::Sender<AppEvent>) -> Result<()> {
    let Some(hit) = app.results.get(app.selected).cloned() else {
        app.status = "No selection.".into();
        return Ok(());
    };

    let scope = app.config.target;
    let already_tracked = app.manifest_for(scope).packages.contains(&hit.attr);
    let already_queued = app.queue.iter().any(|op| match op {
        QueuedOp::Install { hit: h, scope: s } => *s == scope && h.attr == hit.attr,
        _ => false,
    });
    if already_tracked || already_queued {
        app.status = format!("{} already tracked or queued.", hit.attr);
        return Ok(());
    }

    let attr = hit.attr.clone();
    app.queue.push_back(QueuedOp::Install { hit, scope });
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
    let cursor = match app.installed_cursor() {
        Some(c) => c,
        None => {
            app.status = "No selection.".into();
            return Ok(());
        }
    };
    let pkg = match cursor {
        InstalledCursor::Managed(p) => p,
        InstalledCursor::External(ep) => {
            app.status = format!(
                "{} is external (in {}) — press m to migrate first.",
                ep.name, ep.source_attr,
            );
            return Ok(());
        }
    };

    let scope = pkg.scope;
    if app.queue.iter().any(|op| match op {
        QueuedOp::Uninstall { name, scope: s } => *s == scope && *name == pkg.name,
        _ => false,
    }) {
        app.status = format!("{} already queued for removal.", pkg.name);
        return Ok(());
    }

    let name = pkg.name.clone();
    app.queue.push_back(QueuedOp::Uninstall {
        name: name.clone(),
        scope,
    });
    if app.build_in_progress {
        app.status = format!("Queued remove: {} [{}].", name, scope.tag());
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
        InstalledCursor::Managed(p) => {
            app.status = format!("{} is already managed — press d to uninstall.", p.name);
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
    let scope = match ep.scope {
        ScanTarget::HomeManager => Target::HomeManager,
        ScanTarget::Nixos => Target::NixosSystem,
    };
    let name = ep.name.clone();
    if app.queue.iter().any(|op| match op {
        QueuedOp::Migrate { names, scope: s } => *s == scope && names.contains(&name),
        _ => false,
    }) {
        app.status = format!("{} already queued for migration.", name);
        return Ok(());
    }
    app.queue.push_back(QueuedOp::Migrate {
        names: vec![name.clone()],
        scope,
    });
    if app.build_in_progress {
        app.status = format!("Queued migrate: {} [{}].", name, scope.tag());
        app.tab = Tab::Queue;
    } else {
        app.tab = Tab::Building;
        drain_queue(app, tx);
    }
    Ok(())
}

pub(crate) async fn migrate_all(app: &mut App, tx: &mpsc::Sender<AppEvent>) -> Result<()> {
    let mut hm: Vec<String> = Vec::new();
    let mut nx: Vec<String> = Vec::new();
    for ep in &app.external_packages {
        if !ep.migratable {
            continue;
        }
        match ep.scope {
            ScanTarget::HomeManager => hm.push(ep.name.clone()),
            ScanTarget::Nixos => nx.push(ep.name.clone()),
        }
    }
    if hm.is_empty() && nx.is_empty() {
        app.status = "No migratable external packages found.".into();
        return Ok(());
    }
    let hm_count = hm.len();
    let nx_count = nx.len();
    if !hm.is_empty() {
        app.queue.push_back(QueuedOp::Migrate {
            names: hm,
            scope: Target::HomeManager,
        });
    }
    if !nx.is_empty() {
        app.queue.push_back(QueuedOp::Migrate {
            names: nx,
            scope: Target::NixosSystem,
        });
    }
    let summary = format!(
        "Queued migrate-all ({} hm, {} nixos).",
        hm_count, nx_count,
    );
    if app.build_in_progress {
        app.status = summary;
        app.tab = Tab::Queue;
    } else {
        app.status = summary;
        app.tab = Tab::Building;
        drain_queue(app, tx);
    }
    Ok(())
}

pub(crate) fn schedule_search(app: &mut App, tx: mpsc::Sender<AppEvent>) {
    let query = app.input.value().to_string();
    if query.is_empty() {
        app.search_epoch += 1;
        app.searching = false;
        app.results.clear();
        app.selected = 0;
        app.latest_query = String::new();
        app.status = "Type to search packages.".into();
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
