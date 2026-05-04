use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::Deserialize;
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub attr: String,
    pub pname: String,
    pub version: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
struct RawHit {
    pname: String,
    version: String,
    #[serde(default)]
    description: String,
}

pub async fn search(channel: &str, query: &str) -> Result<Vec<SearchHit>> {
    let query = if query.trim().is_empty() { "^" } else { query };

    let output = Command::new("nix")
        .args([
            "search",
            "--json",
            "--extra-experimental-features",
            "nix-command flakes",
            channel,
            query,
        ])
        .output()
        .await
        .context("invoking `nix search` (is nix installed and on PATH?)")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("nix search failed: {}", stderr.trim());
    }

    let parsed: BTreeMap<String, RawHit> =
        serde_json::from_slice(&output.stdout).context("parsing nix search JSON")?;

    let mut hits: Vec<SearchHit> = parsed
        .into_iter()
        .map(|(attr, raw)| SearchHit {
            attr: short_attr(&attr).to_string(),
            pname: raw.pname,
            version: raw.version,
            description: raw.description,
        })
        .collect();

    hits.sort_by(|a, b| a.pname.cmp(&b.pname));
    Ok(hits)
}

fn short_attr(full: &str) -> &str {
    full.rsplit_once('.').map(|(_, tail)| tail).unwrap_or(full)
}
