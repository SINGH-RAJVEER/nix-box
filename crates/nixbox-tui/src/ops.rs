use std::{collections::VecDeque, time::Duration};

use anyhow::Result;
use nixbox_config::Target;
use nixbox_nix::{
    build::{
        BuildEvent, flake_has_home_configuration, home_manager_switch_cmd,
        nixos_rebuild_switch_cmd, rebuild,
    },
    manifest::{ImportStatus, ManagedFile, ensure_imported},
    scan::{ScanTarget, remove_from_source},
};
use tokio::sync::{mpsc, oneshot};
use tokio::time::sleep;

use crate::app::{App, AppEvent, InstalledCursor, QueuedOp, Tab};
use crate::state::InProgress;

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
fn ensure_imported_note(main_file: &std::path::Path, managed: &ManagedFile) -> Option<String> {
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
    let (cancel_tx, mut cancel_rx) = oneshot::channel();
    app.build_cancel = Some(cancel_tx);
    app.current_op_label = Some(action_label.clone());
    app.in_progress_op = Some(InProgress {
        scope,
        label: action_label.clone(),
    });
    app.log.clear();
    app.status = format!("{}...", action_label);
    app.persist();

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
                let has_home_configuration = tokio::select! {
                    has_home = flake_has_home_configuration(&config_dir) => has_home,
                    _ = &mut cancel_rx => {
                        let _ = build_tx.send(BuildEvent::Cancelled).await;
                        drop(build_tx);
                        let _ = forwarder.await;
                        return;
                    }
                };
                let (c, a) = if has_home_configuration {
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
                (
                    cmd_owned.as_str(),
                    args_owned.iter().map(|s| s.as_str()).collect(),
                )
            }
            Target::NixosSystem => {
                let (c, a) = nixos_rebuild_switch_cmd(&config_dir);
                cmd_owned = c;
                args_owned = a;
                (
                    cmd_owned.as_str(),
                    args_owned.iter().map(|s| s.as_str()).collect(),
                )
            }
        };
        if let Err(e) = rebuild(cmd, &args, build_tx.clone(), cancel_rx).await {
            let _ = build_tx
                .send(BuildEvent::Finished(Err(e.to_string())))
                .await;
        }
        drop(build_tx);
        let _ = forwarder.await;
    });
}

pub(crate) fn cancel_build(app: &mut App) {
    if !app.build_in_progress {
        return;
    }
    let Some(cancel) = app.build_cancel.take() else {
        return;
    };
    if cancel.send(()).is_ok() {
        app.status = "Cancelling build...".into();
    }
}

pub(crate) fn drain_queue(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    if app.build_in_progress {
        return;
    }
    let Some(scope) = app.queue.front().map(QueuedOp::scope) else {
        app.persist();
        return;
    };

    let batch = take_queued_scope(app, scope);
    let mut applied = 0;
    for op in batch {
        let label = op.label();
        match apply_op_to_manifest(app, &op) {
            Ok(()) => applied += 1,
            Err(e) => app
                .log
                .push(format!("{}: failed to write manifest: {}", label, e)),
        }
    }
    if applied == 0 {
        app.status = format!("No queued {} changes could be written.", scope.label());
        app.persist();
        drain_queue(app, tx);
        return;
    }
    if !app.visible_tabs().contains(&app.tab) {
        app.tab = Tab::Search;
    }
    let label = format!("apply {} queued {} change(s)", applied, scope.label(),);
    spawn_rebuild(app, tx, scope, label);
}

/// Removes every pending operation for `scope`, preserving the order of work
/// for any other scope that requires a separate rebuild command.
fn take_queued_scope(app: &mut App, scope: Target) -> Vec<QueuedOp> {
    let mut batch = Vec::new();
    let mut remaining = VecDeque::new();
    while let Some(op) = app.queue.pop_front() {
        if op.scope() == scope {
            batch.push(op);
        } else {
            remaining.push_back(op);
        }
    }
    app.queue = remaining;
    batch
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
            app.external_packages
                .retain(|ep| !(ep_target_eq(ep.scope, scope) && removed.contains(&ep.name)));
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
    if main_file.exists()
        && let Some(note) = git_track(&main_file)
    {
        app.log.push(note);
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
    app.persist();
    if app.build_in_progress {
        app.status = format!("Queued install: {}.", attr);
        app.tab = Tab::Queue;
    } else {
        app.tab = Tab::Building;
        drain_queue(app, tx);
    }
    Ok(())
}

pub(crate) async fn uninstall_selected(app: &mut App, tx: &mpsc::Sender<AppEvent>) -> Result<()> {
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
    app.persist();
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
    app.persist();
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
    app.persist();
    let summary = format!("Queued migrate-all ({} hm, {} nixos).", hm_count, nx_count,);
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
    if let Some(task) = app.search_task.take() {
        task.abort();
    }

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

    app.search_task = Some(tokio::spawn(async move {
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
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::vim::VimInput;
    use nixbox_config::Config;
    use nixbox_nix::{manifest::Manifest, search::SearchHit};
    use tokio::sync::{mpsc, oneshot};
    use tokio::time::timeout;

    fn hit(name: &str) -> SearchHit {
        SearchHit {
            attr: name.into(),
            pname: name.into(),
            version: "1.0".into(),
            description: String::new(),
        }
    }

    fn test_app() -> App {
        App::new(
            Config::default(),
            Manifest::default(),
            Manifest::default(),
            Vec::new(),
        )
    }

    #[tokio::test]
    async fn empty_query_clears_results_and_does_not_spawn_search() {
        let (tx, _rx) = mpsc::channel(1);
        let mut app = test_app();
        app.input = VimInput::new(String::new());
        app.results = vec![hit("ripgrep")];
        app.selected = 4;
        app.searching = true;
        app.latest_query = "ripgrep".into();

        schedule_search(&mut app, tx);

        assert_eq!(app.search_epoch, 1);
        assert!(!app.searching);
        assert!(app.results.is_empty());
        assert_eq!(app.selected, 0);
        assert_eq!(app.latest_query, "");
        assert_eq!(app.status, "Type to search packages.");
        assert!(app.search_task.is_none());
    }

    #[tokio::test]
    async fn new_query_aborts_previous_pending_search_task() {
        struct NotifyOnDrop(Option<oneshot::Sender<()>>);

        impl Drop for NotifyOnDrop {
            fn drop(&mut self) {
                if let Some(tx) = self.0.take() {
                    let _ = tx.send(());
                }
            }
        }

        let (tx, _rx) = mpsc::channel(1);
        let (dropped_tx, dropped_rx) = oneshot::channel();
        let mut app = test_app();
        app.input = VimInput::new("ripgrep".into());
        app.search_task = Some(tokio::spawn(async move {
            let _guard = NotifyOnDrop(Some(dropped_tx));
            std::future::pending::<()>().await;
        }));
        tokio::task::yield_now().await;

        schedule_search(&mut app, tx);

        timeout(Duration::from_secs(1), dropped_rx)
            .await
            .expect("old search task should be aborted")
            .expect("drop notification should be sent");
        assert_eq!(app.search_epoch, 1);
        assert!(app.searching);
        assert_eq!(app.latest_query, "ripgrep");
        assert!(app.search_task.is_some());
    }

    #[tokio::test]
    async fn cancel_build_signals_active_rebuild() {
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let mut app = test_app();
        app.build_in_progress = true;
        app.build_cancel = Some(cancel_tx);

        cancel_build(&mut app);

        cancel_rx.await.expect("build should be signalled");
        assert!(app.build_cancel.is_none());
        assert!(app.build_in_progress);
        assert_eq!(app.status, "Cancelling build...");
    }

    #[test]
    fn taking_a_scope_batches_all_of_its_pending_operations() {
        let mut app = test_app();
        app.queue.push_back(QueuedOp::Install {
            hit: hit("ripgrep"),
            scope: Target::HomeManager,
        });
        app.queue.push_back(QueuedOp::Install {
            hit: hit("fd"),
            scope: Target::NixosSystem,
        });
        app.queue.push_back(QueuedOp::Uninstall {
            name: "neovim".into(),
            scope: Target::HomeManager,
        });

        let batch = take_queued_scope(&mut app, Target::HomeManager);

        assert_eq!(batch.len(), 2);
        assert!(batch.iter().all(|op| op.scope() == Target::HomeManager));
        assert_eq!(app.queue.len(), 1);
        assert!(matches!(
            app.queue.front(),
            Some(QueuedOp::Install { hit, scope: Target::NixosSystem }) if hit.attr == "fd"
        ));
    }
}
