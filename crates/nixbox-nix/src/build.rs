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

pub async fn rebuild(command: &str, args: &[&str], tx: mpsc::Sender<BuildEvent>) -> Result<()> {
    let mut child = Command::new(command)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning `{}`", command))?;

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
