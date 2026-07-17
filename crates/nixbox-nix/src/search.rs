use std::{collections::BTreeMap, process::Stdio};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

const MAX_SEARCH_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_SEARCH_ERROR_BYTES: usize = 64 * 1024;
const MAX_SEARCH_RESULTS: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Resolves a channel shortname to a flake ref. Bare names like `nixpkgs-unstable`
/// aren't always in the user's registry, so map known ones explicitly.
fn resolve_channel(channel: &str) -> &str {
    match channel {
        "nixpkgs-unstable" => "github:NixOS/nixpkgs/nixos-unstable",
        other => other,
    }
}

pub async fn search(channel: &str, query: &str) -> Result<Vec<SearchHit>> {
    let query = if query.trim().is_empty() { "^" } else { query };
    let resolved = resolve_channel(channel);

    let mut child = Command::new("nix")
        .args([
            "search",
            "--json",
            "--quiet",
            "--extra-experimental-features",
            "nix-command flakes",
            resolved,
            query,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("invoking `nix search` (is nix installed and on PATH?)")?;

    let stdout = child.stdout.take().context("capturing nix search stdout")?;
    let stderr = child.stderr.take().context("capturing nix search stderr")?;
    let stderr_task = tokio::spawn(read_capped(stderr, MAX_SEARCH_ERROR_BYTES));

    let stdout = match read_limited(stdout, MAX_SEARCH_OUTPUT_BYTES).await? {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = stderr_task.await;
            anyhow::bail!(
                "nix search returned more than {} MiB; narrow the query",
                MAX_SEARCH_OUTPUT_BYTES / 1024 / 1024
            );
        }
    };

    let status = child.wait().await.context("waiting for nix search")?;
    let stderr = stderr_task
        .await
        .context("joining nix search stderr reader")??;

    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        anyhow::bail!("nix search failed: {}", stderr.trim());
    }

    parse_search_output(&stdout)
}

fn parse_search_output(stdout: &[u8]) -> Result<Vec<SearchHit>> {
    let parsed: BTreeMap<String, RawHit> =
        serde_json::from_slice(stdout).context("parsing nix search JSON")?;

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
    hits.truncate(MAX_SEARCH_RESULTS);
    Ok(hits)
}

async fn read_limited<R>(mut reader: R, limit: usize) -> std::io::Result<Option<Vec<u8>>>
where
    R: AsyncRead + Unpin,
{
    let mut out = Vec::new();
    let mut buf = [0; 8192];
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            return Ok(Some(out));
        }
        if out.len().saturating_add(n) > limit {
            return Ok(None);
        }
        out.extend_from_slice(&buf[..n]);
    }
}

async fn read_capped<R>(mut reader: R, limit: usize) -> std::io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut out = Vec::new();
    let mut buf = [0; 8192];
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            return Ok(out);
        }
        let remaining = limit.saturating_sub(out.len());
        out.extend_from_slice(&buf[..n.min(remaining)]);
    }
}

fn short_attr(full: &str) -> &str {
    full.rsplit_once('.').map(|(_, tail)| tail).unwrap_or(full)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn resolves_known_channel_aliases() {
        assert_eq!(
            resolve_channel("nixpkgs-unstable"),
            "github:NixOS/nixpkgs/nixos-unstable"
        );
        assert_eq!(resolve_channel("nixpkgs"), "nixpkgs");
        assert_eq!(resolve_channel("github:owner/repo"), "github:owner/repo");
    }

    #[test]
    fn short_attr_keeps_final_attr_segment() {
        assert_eq!(short_attr("legacyPackages.x86_64-linux.ripgrep"), "ripgrep");
        assert_eq!(short_attr("firefox"), "firefox");
        assert_eq!(short_attr("pkgs.python312Packages.black"), "black");
    }

    #[tokio::test]
    async fn read_limited_allows_exact_limit() {
        let bytes = b"abcdef".to_vec();
        let out = read_limited(Cursor::new(bytes), 6).await.expect("read");
        assert_eq!(out, Some(b"abcdef".to_vec()));
    }

    #[tokio::test]
    async fn read_limited_rejects_payload_over_limit() {
        let bytes = b"abcdef".to_vec();
        let out = read_limited(Cursor::new(bytes), 5).await.expect("read");
        assert_eq!(out, None);
    }

    #[tokio::test]
    async fn read_capped_drains_bytes_beyond_limit() {
        let bytes = b"abcdef".to_vec();
        let mut reader = Cursor::new(bytes);

        let out = read_capped(&mut reader, 3).await.expect("read");

        assert_eq!(out, b"abc");
        assert_eq!(reader.position(), 6);
    }

    #[test]
    fn parses_sorts_shortens_and_defaults_description() {
        let raw = br#"
        {
            "legacyPackages.x86_64-linux.zoxide": {
                "pname": "zoxide",
                "version": "1.0",
                "description": "jump around"
            },
            "legacyPackages.x86_64-linux.fd": {
                "pname": "fd",
                "version": "2.0"
            }
        }
        "#;

        let hits = parse_search_output(raw).expect("parse search output");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].attr, "fd");
        assert_eq!(hits[0].pname, "fd");
        assert_eq!(hits[0].description, "");
        assert_eq!(hits[1].attr, "zoxide");
        assert_eq!(hits[1].description, "jump around");
    }

    #[test]
    fn parse_search_output_caps_result_count() {
        let mut raw = String::from("{");
        for i in 0..(MAX_SEARCH_RESULTS + 25) {
            if i > 0 {
                raw.push(',');
            }
            raw.push_str(&format!(
                r#""legacyPackages.x86_64-linux.pkg{i:04}":{{"pname":"pkg{i:04}","version":"1","description":""}}"#
            ));
        }
        raw.push('}');

        let hits = parse_search_output(raw.as_bytes()).expect("parse search output");
        assert_eq!(hits.len(), MAX_SEARCH_RESULTS);
        assert_eq!(hits.first().unwrap().pname, "pkg0000");
        assert_eq!(hits.last().unwrap().pname, "pkg0499");
    }
}
