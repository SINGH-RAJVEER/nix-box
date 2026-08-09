use std::collections::BTreeMap;
use std::path::Path;
use std::{fs, process::Stdio};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tokio::process::Command;

pub const MAX_FLAKE_RESULTS: usize = 20;

#[derive(Debug, Clone)]
pub struct FlakeHit {
    pub repo: String,
    pub repo_url: String,
    pub path: String,
    pub match_fragment: Option<String>,
    content_url: String,
}

#[derive(Debug, Clone)]
pub struct FlakeDetails {
    pub repo: String,
    pub repo_url: String,
    pub path: String,
    pub description: Option<String>,
    pub stars: u64,
    pub topics: Vec<String>,
    pub homepage: Option<String>,
    pub default_branch: String,
    pub pushed_at: Option<String>,
    pub archived: bool,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

#[derive(Deserialize)]
struct CodeSearchResponse {
    items: Vec<CodeSearchItem>,
}

#[derive(Deserialize)]
struct CodeSearchItem {
    path: String,
    url: String,
    repository: SearchRepository,
    #[serde(default)]
    text_matches: Vec<TextMatch>,
}

#[derive(Deserialize)]
struct TextMatch {
    fragment: String,
}

#[derive(Deserialize)]
struct SearchRepository {
    full_name: String,
    html_url: String,
}

#[derive(Deserialize)]
struct RepositorySearchResponse {
    items: Vec<RepositorySearchItem>,
}

#[derive(Deserialize)]
struct RepositorySearchItem {
    full_name: String,
    stargazers_count: u64,
}

struct RankedHit {
    score: u64,
    hit: FlakeHit,
}

#[derive(Deserialize)]
struct Repository {
    #[serde(default)]
    description: Option<String>,
    stargazers_count: u64,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    homepage: Option<String>,
    default_branch: String,
    #[serde(default)]
    pushed_at: Option<String>,
    archived: bool,
}

/// Searches GitHub's code index for flakes at the root of their repositories.
/// Authentication is delegated to the user's existing `gh auth login` session.
pub async fn search_flakes(query: &str) -> Result<Vec<FlakeHit>> {
    let query = query.trim();
    let (code_items, repositories) =
        tokio::try_join!(search_code(query), search_repositories(query))?;
    let mut direct = Vec::new();
    let mut candidates: BTreeMap<String, u64> = BTreeMap::new();

    for item in code_items {
        let name_score = repository_name_score(query, &item.repository.full_name);
        direct.push(RankedHit {
            score: 100 + name_score,
            hit: FlakeHit {
                repo: item.repository.full_name,
                repo_url: item.repository.html_url,
                path: item.path,
                match_fragment: item
                    .text_matches
                    .first()
                    .map(|matched| compact_fragment(&matched.fragment)),
                content_url: item
                    .url
                    .strip_prefix("https://api.github.com/")
                    .unwrap_or(&item.url)
                    .to_string(),
            },
        });
        for reference in item
            .text_matches
            .iter()
            .flat_map(|matched| github_references(&matched.fragment))
        {
            let score = repository_name_score(query, &reference);
            if score > 0 {
                candidates
                    .entry(reference)
                    .and_modify(|existing| *existing = (*existing).max(1_000 + score))
                    .or_insert(1_000 + score);
            }
        }
    }

    for repository in repositories {
        let score = repository_name_score(query, &repository.full_name);
        candidates
            .entry(repository.full_name)
            .and_modify(|existing| *existing = (*existing).max(500 + score))
            .or_insert(500 + score + repository.stargazers_count.min(10_000) / 1_000);
    }

    let mut tasks = tokio::task::JoinSet::new();
    for (repo, score) in candidates {
        tasks.spawn(async move {
            root_flake_hit(repo)
                .await
                .map(|hit| RankedHit { score, hit })
        });
    }
    while let Some(result) = tasks.join_next().await {
        if let Ok(Ok(hit)) = result {
            direct.push(hit);
        }
    }

    let mut deduped: BTreeMap<String, RankedHit> = BTreeMap::new();
    for candidate in direct {
        match deduped.get(&candidate.hit.repo) {
            Some(existing) if existing.score >= candidate.score => {}
            _ => {
                deduped.insert(candidate.hit.repo.clone(), candidate);
            }
        }
    }
    let mut ranked: Vec<RankedHit> = deduped.into_values().collect();
    ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.hit.repo.cmp(&b.hit.repo))
    });
    ranked.truncate(MAX_FLAKE_RESULTS);
    Ok(ranked.into_iter().map(|candidate| candidate.hit).collect())
}

async fn search_code(query: &str) -> Result<Vec<CodeSearchItem>> {
    let query = format!("{query} in:file filename:flake.nix path:/");
    let raw = gh_api(vec![
        "--method".into(),
        "GET".into(),
        "-H".into(),
        "Accept: application/vnd.github.text-match+json".into(),
        "-H".into(),
        "X-GitHub-Api-Version: 2026-03-10".into(),
        "search/code".into(),
        "-f".into(),
        format!("q={query}"),
        "-f".into(),
        "per_page=20".into(),
    ])
    .await?;
    let response: CodeSearchResponse =
        serde_json::from_slice(&raw).context("parsing GitHub code search response")?;
    Ok(response.items)
}

async fn search_repositories(query: &str) -> Result<Vec<RepositorySearchItem>> {
    let raw = gh_api(vec![
        "--method".into(),
        "GET".into(),
        "-H".into(),
        "Accept: application/vnd.github+json".into(),
        "search/repositories".into(),
        "-f".into(),
        format!("q={query} in:name,description,topics archived:false"),
        "-f".into(),
        "per_page=10".into(),
    ])
    .await?;
    let response: RepositorySearchResponse =
        serde_json::from_slice(&raw).context("parsing GitHub repository search response")?;
    Ok(response.items)
}

async fn root_flake_hit(repo: String) -> Result<FlakeHit> {
    gh_api(vec![
        "-H".into(),
        "Accept: application/vnd.github.raw+json".into(),
        format!("repos/{repo}/contents/flake.nix"),
    ])
    .await?;
    let content_url = format!("repos/{repo}/contents/flake.nix");
    Ok(FlakeHit {
        repo_url: format!("https://github.com/{repo}"),
        repo,
        path: "flake.nix".into(),
        match_fragment: None,
        content_url,
    })
}

fn repository_name_score(query: &str, repo: &str) -> u64 {
    let query = normalize(query);
    let name = repo.rsplit('/').next().map(normalize).unwrap_or_default();
    if query.is_empty() || name.is_empty() {
        0
    } else if name == query {
        500
    } else if name.starts_with(&query) {
        400
    } else if name.contains(&query) {
        300
    } else {
        0
    }
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn github_references(fragment: &str) -> Vec<String> {
    let mut references = Vec::new();
    let mut remaining = fragment;
    while let Some(start) = remaining.find("github:") {
        remaining = &remaining[start + "github:".len()..];
        let path: String = remaining
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/'))
            .collect();
        let mut segments = path.split('/');
        let Some(owner) = segments.next() else {
            continue;
        };
        let Some(repo) = segments.next() else {
            continue;
        };
        if !owner.is_empty() && !repo.is_empty() {
            let reference = format!("{owner}/{repo}");
            if !references.contains(&reference) {
                references.push(reference);
            }
        }
    }
    references
}

fn compact_fragment(fragment: &str) -> String {
    fragment.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Fetches the selected repository and its flake source concurrently, then
/// extracts the public metadata and common flake input/output declarations.
pub async fn fetch_flake_details(hit: &FlakeHit) -> Result<FlakeDetails> {
    let metadata = gh_api(vec![
        "-H".into(),
        "Accept: application/vnd.github+json".into(),
        format!("repos/{}", hit.repo),
    ]);
    let source = gh_api(vec![
        "-H".into(),
        "Accept: application/vnd.github.raw+json".into(),
        hit.content_url.clone(),
    ]);
    let (metadata, source) = tokio::try_join!(metadata, source)?;
    let repository: Repository =
        serde_json::from_slice(&metadata).context("parsing GitHub repository response")?;
    let source = String::from_utf8(source).context("flake.nix was not valid UTF-8")?;

    Ok(FlakeDetails {
        repo: hit.repo.clone(),
        repo_url: hit.repo_url.clone(),
        path: hit.path.clone(),
        description: repository
            .description
            .filter(|value| !value.trim().is_empty()),
        stars: repository.stargazers_count,
        topics: repository.topics,
        homepage: repository.homepage.filter(|value| !value.trim().is_empty()),
        default_branch: repository.default_branch,
        pushed_at: repository.pushed_at,
        archived: repository.archived,
        inputs: classify_inputs(&source),
        outputs: classify_outputs(&source),
    })
}

/// Adds a GitHub flake as a root input and makes `inputs` available to the
/// selected configuration constructor's modules.
pub fn ensure_flake_input(
    flake_file: &Path,
    repo: &str,
    special_args: &str,
    constructor: &str,
) -> Result<()> {
    let source = fs::read_to_string(flake_file)
        .with_context(|| format!("reading {}", flake_file.display()))?;
    if !source.contains("outputs = inputs@") {
        bail!(
            "{} must bind `inputs` in its outputs function before nixbox can import flake modules",
            flake_file.display()
        );
    }

    let mut updated = source;
    let input = format!("\"{repo}\"");
    if !updated.contains(&format!("{input}.url")) {
        let inputs_pos = updated
            .find("inputs = {")
            .context("could not find an `inputs = { ... };` block")?;
        let open = inputs_pos + "inputs = ".len();
        let close = matching_brace(&updated, open)
            .context("could not find the end of the flake inputs block")?;
        updated.insert_str(close, &format!("  {input}.url = \"github:{repo}\";\n"));
    }

    if !updated.contains(special_args) {
        let constructor_pos = updated
            .find(constructor)
            .with_context(|| format!("could not find `{constructor}`"))?;
        let open = updated[constructor_pos..]
            .find('{')
            .map(|offset| constructor_pos + offset)
            .context("could not find configuration arguments")?;
        updated.insert_str(
            open + 1,
            &format!("\n    {special_args} = {{ inherit inputs; }};"),
        );
    }

    fs::write(flake_file, updated).with_context(|| format!("writing {}", flake_file.display()))
}

fn matching_brace(source: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

async fn gh_api(args: Vec<String>) -> Result<Vec<u8>> {
    let output = Command::new("gh")
        .arg("api")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .context("invoking `gh api` (run `gh auth login` to enable flake browsing)")?;
    if !output.status.success() {
        bail!(
            "GitHub API request failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

fn classify_inputs(source: &str) -> Vec<String> {
    let Some(start) = source.find("inputs") else {
        return Vec::new();
    };
    let Some(open) = source[start..].find('{').map(|offset| start + offset) else {
        return Vec::new();
    };
    let Some(block) = balanced_block(source, open) else {
        return Vec::new();
    };

    let mut inputs = Vec::new();
    for line in block.lines() {
        let line = line.trim_start();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let name: String = line
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
            .collect();
        if !name.is_empty() && !inputs.contains(&name) {
            inputs.push(name);
        }
    }
    inputs
}

fn balanced_block(source: &str, open: usize) -> Option<&str> {
    let mut depth = 0;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[open + 1..open + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

fn classify_outputs(source: &str) -> Vec<String> {
    const OUTPUTS: [(&str, &str); 8] = [
        ("packages", "packages"),
        ("nixosModules", "NixOS modules"),
        ("homeManagerModules", "Home Manager modules"),
        ("homeModules", "Home Manager modules"),
        ("overlays", "overlays"),
        ("devShells", "dev shells"),
        ("apps", "apps"),
        ("formatter", "formatter"),
    ];
    let mut outputs = Vec::new();
    for (name, label) in OUTPUTS {
        if contains_identifier(source, name) && !outputs.iter().any(|output| output == label) {
            outputs.push(label.to_string());
        }
    }
    outputs
}

fn contains_identifier(source: &str, name: &str) -> bool {
    source.match_indices(name).any(|(start, _)| {
        let before = source[..start].chars().next_back();
        let after = source[start + name.len()..].chars().next();
        before.is_none_or(|ch| !(ch.is_ascii_alphanumeric() || ch == '_'))
            && after.is_none_or(|ch| !(ch.is_ascii_alphanumeric() || ch == '_'))
    })
}

#[cfg(test)]
mod tests {
    use super::{
        classify_inputs, classify_outputs, compact_fragment, ensure_flake_input, github_references,
        repository_name_score,
    };
    use std::fs;

    #[test]
    fn compacts_github_match_fragments_for_result_rows() {
        assert_eq!(
            compact_fragment("  inputs.nixpkgs.url =\n  \"github:NixOS/nixpkgs\";  "),
            "inputs.nixpkgs.url = \"github:NixOS/nixpkgs\";"
        );
    }

    #[test]
    fn extracts_upstream_github_references_without_branch_suffixes() {
        assert_eq!(
            github_references(
                "zen.url = \"github:0xc000022070/zen-browser-flake/beta\"; other = \"github:NixOS/nixpkgs\";"
            ),
            ["0xc000022070/zen-browser-flake", "NixOS/nixpkgs"]
        );
    }

    #[test]
    fn ranks_canonical_repository_names_above_consumer_repositories() {
        assert!(
            repository_name_score("zen-browser", "0xc000022070/zen-browser-flake")
                > repository_name_score("zen-browser", "Baitinq/nixos-config")
        );
    }

    #[test]
    fn classifies_common_flake_properties() {
        let source = r#"
            {
              inputs = {
                nixpkgs.url = "github:NixOS/nixpkgs";
                home-manager.url = "github:nix-community/home-manager";
              };
              outputs = { nixpkgs, ... }: {
                packages.x86_64-linux.default = nixpkgs.legacyPackages.x86_64-linux.hello;
                nixosModules.default = { };
                homeManagerModules.default = { };
                devShells.x86_64-linux.default = { };
              };
            }
        "#;

        assert_eq!(classify_inputs(source), ["nixpkgs", "home-manager"]);
        assert_eq!(
            classify_outputs(source),
            [
                "packages",
                "NixOS modules",
                "Home Manager modules",
                "dev shells"
            ]
        );
    }

    #[test]
    fn adds_input_and_home_manager_special_args() {
        let dir = std::env::temp_dir().join(format!("nixbox-flake-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("flake.nix");
        fs::write(
            &file,
            "{\n  inputs = { nixpkgs.url = \"github:NixOS/nixpkgs\"; };\n  outputs = inputs@{ self, nixpkgs, ... }: {\n    homeConfigurations.user = inputs.home-manager.lib.homeManagerConfiguration {\n      modules = [ ./home.nix ];\n    };\n  };\n}\n",
        )
        .unwrap();

        ensure_flake_input(
            &file,
            "owner/module",
            "extraSpecialArgs",
            "homeManagerConfiguration",
        )
        .unwrap();

        let updated = fs::read_to_string(&file).unwrap();
        assert!(updated.contains("\"owner/module\".url = \"github:owner/module\";"));
        assert!(updated.contains("extraSpecialArgs = { inherit inputs; };"));
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    #[ignore = "requires authenticated gh access and consumes GitHub search quota"]
    async fn live_zen_browser_search_promotes_the_upstream_flake() {
        let hits = super::search_flakes("zen-browser").await.unwrap();

        assert_eq!(
            hits.first().map(|hit| hit.repo.as_str()),
            Some("0xc000022070/zen-browser-flake")
        );
    }
}
