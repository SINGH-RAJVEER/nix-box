use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
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
        format!("/run/wrappers/bin/{name}"),
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
        (
            bin.to_string_lossy().into_owned(),
            vec!["switch".into(), "--flake".into(), flake_ref],
        )
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

/// Checks whether the flake at `config_dir` exposes a standalone
/// `homeConfigurations.<user>` output. When false, home-manager packages must
/// be applied via `nixos-rebuild` because the user wires home-manager in as a
/// NixOS module rather than as a separate flake output.
pub async fn flake_has_home_configuration(config_dir: &Path) -> bool {
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    let nix = find_in_nix_profiles("nix").unwrap_or_else(|| PathBuf::from("nix"));
    let expr = format!(
        "let f = builtins.getFlake \"{}\"; in f ? homeConfigurations && f.homeConfigurations ? \"{}\"",
        config_dir.display(),
        user,
    );
    let output = Command::new(nix)
        .args(["eval", "--impure", "--json", "--expr", &expr])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;
    match output {
        Ok(out) if out.status.success() => {
            std::str::from_utf8(&out.stdout).map(|s| s.trim() == "true").unwrap_or(false)
        }
        _ => false,
    }
}

pub fn nixos_rebuild_switch_cmd(config_dir: &Path) -> (String, Vec<String>) {
    let flake_ref = format!("{}#nixos", config_dir.display());
    (
        "sudo".into(),
        vec![
            "nixos-rebuild".into(),
            "switch".into(),
            "--flake".into(),
            flake_ref,
        ],
    )
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

    let stdout_task = tokio::spawn(forward_output(stdout, stdout_tx));
    let stderr_task = tokio::spawn(forward_output(stderr, stderr_tx));

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

async fn forward_output<R>(stream: R, tx: mpsc::Sender<BuildEvent>)
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(stream).lines();
    while let Ok(Some(line)) = reader.next_line().await {
        let text = line.trim_end().to_string();
        if text.is_empty() {
            continue;
        }
        if tx.send(BuildEvent::Line(text)).await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BuildEvent, forward_output, nixos_rebuild_switch_cmd};
    use tokio::io::{AsyncWriteExt, duplex};
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn forwards_newline_delimited_lines() {
        let (mut writer, reader) = duplex(128);
        let (tx, mut rx) = mpsc::channel(8);
        let task = tokio::spawn(forward_output(reader, tx));

        writer.write_all(b"first line\nsecond line\n").await.unwrap();
        drop(writer);
        task.await.unwrap();

        assert!(matches!(rx.recv().await, Some(BuildEvent::Line(line)) if line == "first line"));
        assert!(matches!(rx.recv().await, Some(BuildEvent::Line(line)) if line == "second line"));
        assert!(rx.recv().await.is_none());
    }

    #[test]
    fn nixos_rebuild_uses_sudo_with_flake_ref() {
        let (cmd, args) = nixos_rebuild_switch_cmd(std::path::Path::new("/tmp/nixbox-config"));

        assert_eq!(cmd, "sudo");
        assert_eq!(args[0], "nixos-rebuild");
        assert_eq!(args[1], "switch");
        assert_eq!(args[2], "--flake");
        assert_eq!(args[3], "/tmp/nixbox-config#nixos");
    }
}
