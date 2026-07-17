use std::fmt;
use std::process::Stdio;

use anyhow::{Context, Result};
use serde::de::{Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

pub const MAX_SEARCH_RESULTS: usize = 200;
const MAX_SEARCH_OUTPUT_BYTES: usize = 50 * 1024 * 1024;
const MAX_SEARCH_ERROR_BYTES: usize = 64 * 1024;

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
            "--quiet",
            "search",
            "--json",
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
    let stderr_task = tokio::spawn(read_truncated(stderr, MAX_SEARCH_ERROR_BYTES));

    let stdout = match read_limited(stdout, MAX_SEARCH_OUTPUT_BYTES).await {
        Ok(stdout) => stdout,
        Err(error) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = stderr_task.await;
            return Err(error);
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

    parse_hits(&stdout, query).context("parsing nix search JSON")
}

async fn read_limited<R>(mut reader: R, max_bytes: usize) -> Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut out = Vec::new();
    let mut buf = [0; 8192];
    loop {
        let n = reader
            .read(&mut buf)
            .await
            .context("reading nix search output")?;
        if n == 0 {
            break;
        }
        if out.len() + n > max_bytes {
            anyhow::bail!(
                "nix search output exceeded {} MiB; refine the query",
                max_bytes / 1024 / 1024
            );
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(out)
}

async fn read_truncated<R>(mut reader: R, max_bytes: usize) -> Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut out = Vec::new();
    let mut buf = [0; 8192];
    loop {
        let n = reader
            .read(&mut buf)
            .await
            .context("reading nix search stderr")?;
        if n == 0 {
            break;
        }
        let remaining = max_bytes.saturating_sub(out.len());
        if remaining > 0 {
            out.extend_from_slice(&buf[..n.min(remaining)]);
        }
    }
    Ok(out)
}

fn parse_hits(input: &[u8], query: &str) -> Result<Vec<SearchHit>, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    deserializer.deserialize_map(SearchHitsVisitor {
        query: ranking_term(query),
    })
}

struct SearchHitsVisitor {
    query: String,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Relevance {
    tier: u8,
    position: usize,
    name_len: usize,
    name: String,
}

struct RankedHit {
    relevance: Relevance,
    hit: SearchHit,
}

impl<'de> Visitor<'de> for SearchHitsVisitor {
    type Value = Vec<SearchHit>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a nix search JSON object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut hits: Vec<RankedHit> = Vec::new();
        while let Some(attr) = map.next_key::<String>()? {
            let raw = map.next_value::<RawHit>()?;
            let hit = SearchHit {
                attr: short_attr(&attr).to_string(),
                pname: raw.pname,
                version: raw.version,
                description: raw.description,
            };
            let ranked = RankedHit {
                relevance: relevance(&hit, &self.query),
                hit,
            };

            if hits.len() < MAX_SEARCH_RESULTS {
                hits.push(ranked);
            } else {
                let (worst_index, worst) = hits
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, ranked)| &ranked.relevance)
                    .expect("bounded result set is not empty");
                if ranked.relevance < worst.relevance {
                    hits[worst_index] = ranked;
                }
            }
        }
        hits.sort_by(|a, b| a.relevance.cmp(&b.relevance));
        Ok(hits.into_iter().map(|ranked| ranked.hit).collect())
    }
}

fn ranking_term(query: &str) -> String {
    let query = query.trim();
    let query = query.strip_prefix('^').unwrap_or(query);
    query.strip_suffix('$').unwrap_or(query).to_lowercase()
}

fn relevance(hit: &SearchHit, query: &str) -> Relevance {
    let attr = hit.attr.to_lowercase();
    let pname = hit.pname.to_lowercase();
    let description = hit.description.to_lowercase();
    let names = [&pname, &attr];

    let (tier, position) = if query.is_empty() {
        (6, usize::MAX)
    } else if names.iter().any(|name| name.as_str() == query) {
        (0, 0)
    } else if names.iter().any(|name| name.starts_with(query)) {
        (1, 0)
    } else if let Some(position) = names
        .iter()
        .filter_map(|name| token_position(name, query))
        .min()
    {
        (2, position)
    } else if let Some(position) = names.iter().filter_map(|name| name.find(query)).min() {
        (3, position)
    } else if let Some(position) = token_position(&description, query) {
        (4, position)
    } else if let Some(position) = description.find(query) {
        (5, position)
    } else {
        (6, usize::MAX)
    };

    Relevance {
        tier,
        position,
        name_len: pname.chars().count(),
        name: pname,
    }
}

fn token_position(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .match_indices(needle)
        .find_map(|(position, matched)| {
            let before = haystack[..position].chars().next_back();
            let after = haystack[position + matched.len()..].chars().next();
            let starts_token = before.is_none_or(|ch| !ch.is_alphanumeric());
            let ends_token = after.is_none_or(|ch| !ch.is_alphanumeric());
            (starts_token && ends_token).then_some(position)
        })
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
        assert_eq!(out, b"abcdef".to_vec());
    }

    #[tokio::test]
    async fn read_limited_rejects_payload_over_limit() {
        let bytes = b"abcdef".to_vec();
        let err = read_limited(Cursor::new(bytes), 5)
            .await
            .expect_err("payload should exceed limit");
        assert!(err.to_string().contains("exceeded"));
    }

    #[tokio::test]
    async fn read_truncated_drains_bytes_beyond_limit() {
        let bytes = b"abcdef".to_vec();
        let mut reader = Cursor::new(bytes);

        let out = read_truncated(&mut reader, 3).await.expect("read");

        assert_eq!(out, b"abc");
        assert_eq!(reader.position(), 6);
    }

    #[test]
    fn parse_hits_shortens_attrs_and_caps_results() {
        let mut json = String::from("{");
        for i in 0..(MAX_SEARCH_RESULTS + 2) {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                r#""legacyPackages.x86_64-linux.pkg{i}":{{"pname":"pkg{i}","version":"1.0","description":"desc"}}"#
            ));
        }
        json.push('}');

        let hits = parse_hits(json.as_bytes(), "pkg").unwrap();

        assert_eq!(hits.len(), MAX_SEARCH_RESULTS);
        assert_eq!(hits[0].attr, "pkg0");
    }

    #[test]
    fn parse_hits_accepts_missing_description() {
        let hits = parse_hits(
            br#"{"legacyPackages.x86_64-linux.ripgrep":{"pname":"ripgrep","version":"14.1.1"}}"#,
            "ripgrep",
        )
        .unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].attr, "ripgrep");
        assert_eq!(hits[0].description, "");
    }

    #[test]
    fn exact_match_displaces_early_weak_matches() {
        let mut json = String::from("{");
        for i in 0..MAX_SEARCH_RESULTS {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                r#""legacyPackages.x86_64-linux.weak{i}":{{"pname":"weak{i}","version":"1.0","description":"A tool written in Go"}}"#
            ));
        }
        json.push_str(
            r#", "legacyPackages.x86_64-linux.go":{"pname":"go","version":"1.24","description":"The Go compiler"}}"#,
        );
        json.push('}');

        let hits = parse_hits(json.as_bytes(), "go").unwrap();

        assert_eq!(hits.len(), MAX_SEARCH_RESULTS);
        assert_eq!(hits[0].pname, "go");
        assert_eq!(
            hits.iter()
                .filter(|hit| hit.pname.starts_with("weak"))
                .count(),
            MAX_SEARCH_RESULTS - 1
        );
    }

    #[test]
    fn ranks_name_matches_before_description_matches() {
        let hits = parse_hits(
            br#"{
                "legacyPackages.x86_64-linux.air":{"pname":"air","version":"1","description":"Live reload for Go apps"},
                "legacyPackages.x86_64-linux.go-tools":{"pname":"go-tools","version":"1","description":"Developer tools"},
                "legacyPackages.x86_64-linux.go":{"pname":"go","version":"1","description":"Compiler"}
            }"#,
            "go",
        )
        .unwrap();

        assert_eq!(
            hits.iter()
                .map(|hit| hit.pname.as_str())
                .collect::<Vec<_>>(),
            ["go", "go-tools", "air"]
        );
    }

    #[tokio::test]
    #[ignore = "requires nix and a configured nixpkgs registry"]
    async fn live_go_search_ranks_exact_package_first() {
        let hits = search("nixpkgs", "go").await.expect("live nix search");

        assert_eq!(hits.first().map(|hit| hit.pname.as_str()), Some("go"));
    }
}
