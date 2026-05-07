use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum BuildEvent {
    Line(String),
    Finished(Result<(), String>),
}

fn find_in_nix_profiles(name: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let user = std::env::var("USER").unwrap_or_default();
    let candidates = [
        format!("{home}/.nix-profile/bin/{name}"),
        format!("/etc/profiles/per-user/{user}/bin/{name}"),
        format!("/run/current-system/sw/bin/{name}"),
        format!("/nix/var/nix/profiles/default/bin/{name}"),
    ];
    candidates.into_iter().map(PathBuf::from).find(|p| p.exists())
}

/// Returns (command, args) for `home-manager switch --flake <config_dir>#<user>`.
/// Falls back to `nix run nixpkgs#home-manager -- switch` when the binary is absent.
pub fn home_manager_switch_cmd(config_dir: &Path) -> (String, Vec<String>) {
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    let flake_ref = format!("{}#{}", config_dir.display(), user);
    if let Some(bin) = find_in_nix_profiles("home-manager") {
        (bin.to_string_lossy().into_owned(), vec!["switch".into(), "--flake".into(), flake_ref])
    } else {
        (
            "nix".into(),
            vec![
                "run".into(),
                "nixpkgs#home-manager".into(),
                "--".into(),
                "switch".into(),
                "--flake".into(),
                flake_ref,
            ],
        )
    }
}

pub async fn rebuild(command: &str, args: &[&str], tx: mpsc::Sender<BuildEvent>) -> Result<()> {
    let resolved = find_in_nix_profiles(command)
        .unwrap_or_else(|| PathBuf::from(command));

    let mut child = Command::new(&resolved)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning `{}` (resolved: {})", command, resolved.display()))?;

    let stdout = child.stdout.take().context("capturing stdout")?;
    let stderr = child.stderr.take().context("capturing stderr")?;

    let stdout_tx = tx.clone();
    let stderr_tx = tx.clone();

    let stdout_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = stdout_tx.send(BuildEvent::Line(line)).await;
        }
    });
    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = stderr_tx.send(BuildEvent::Line(line)).await;
        }
    });

    let status = child.wait().await?;
    let _ = stdout_task.await;
    let _ = stderr_task.await;

    let result = if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} exited with status {}",
            command,
            status.code().map(|c| c.to_string()).unwrap_or_else(|| "<signal>".into())
        ))
    };
    let _ = tx.send(BuildEvent::Finished(result)).await;
    Ok(())
}
