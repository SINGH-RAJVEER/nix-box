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
    let query = if query.is_empty() {
        "filename:flake.nix path:/".to_string()
    } else {
        format!("{query} in:file filename:flake.nix path:/")
    };
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
        format!("per_page={MAX_FLAKE_RESULTS}"),
    ])
    .await?;
    let response: CodeSearchResponse =
        serde_json::from_slice(&raw).context("parsing GitHub code search response")?;
    Ok(response
        .items
        .into_iter()
        .map(|item| FlakeHit {
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
        })
        .collect())
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
    use super::{classify_inputs, classify_outputs, compact_fragment, ensure_flake_input};
    use std::fs;

    #[test]
    fn compacts_github_match_fragments_for_result_rows() {
        assert_eq!(
            compact_fragment("  inputs.nixpkgs.url =\n  \"github:NixOS/nixpkgs\";  "),
            "inputs.nixpkgs.url = \"github:NixOS/nixpkgs\";"
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
}
