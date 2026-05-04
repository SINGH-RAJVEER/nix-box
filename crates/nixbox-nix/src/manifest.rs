use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const MANIFEST_HEADER: &str = "# Managed by nixbox. Do not edit by hand.";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub packages: BTreeSet<String>,
}

impl Manifest {
    pub fn add(&mut self, pname: &str) -> bool {
        self.packages.insert(pname.to_string())
    }

    pub fn remove(&mut self, pname: &str) -> bool {
        self.packages.remove(pname)
    }
}

pub struct ManagedFile {
    path: PathBuf,
}

impl ManagedFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Manifest> {
        if !self.path.exists() {
            return Ok(Manifest::default());
        }
        let raw = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        Ok(parse(&raw))
    }

    pub fn write_home_manager(&self, manifest: &Manifest) -> Result<()> {
        self.write(render_home_manager(manifest))
    }

    pub fn write_nixos(&self, manifest: &Manifest) -> Result<()> {
        self.write(render_nixos(manifest))
    }

    fn write(&self, content: String) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        fs::write(&self.path, content)
            .with_context(|| format!("writing {}", self.path.display()))?;
        Ok(())
    }
}

fn parse(raw: &str) -> Manifest {
    let mut packages = BTreeSet::new();
    let mut in_block = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("# nixbox:packages:start") {
            in_block = true;
            continue;
        }
        if trimmed.starts_with("# nixbox:packages:end") {
            in_block = false;
            continue;
        }
        if in_block && !trimmed.is_empty() && !trimmed.starts_with('#') {
            let token = trimmed.trim_end_matches(';');
            if let Some(pname) = token.strip_prefix("pkgs.") {
                packages.insert(pname.to_string());
            }
        }
    }
    Manifest { packages }
}

fn render_packages(manifest: &Manifest) -> String {
    let mut out = String::new();
    out.push_str("    # nixbox:packages:start\n");
    for pname in &manifest.packages {
        out.push_str(&format!("    pkgs.{}\n", pname));
    }
    out.push_str("    # nixbox:packages:end\n");
    out
}

fn render_home_manager(manifest: &Manifest) -> String {
    format!(
        "{header}\n{{ pkgs, ... }}:\n{{\n  home.packages = [\n{body}  ];\n}}\n",
        header = MANIFEST_HEADER,
        body = render_packages(manifest),
    )
}

fn render_nixos(manifest: &Manifest) -> String {
    format!(
        "{header}\n{{ pkgs, ... }}:\n{{\n  environment.systemPackages = [\n{body}  ];\n}}\n",
        header = MANIFEST_HEADER,
        body = render_packages(manifest),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_home_manager() {
        let mut m = Manifest::default();
        m.add("ripgrep");
        m.add("fd");
        let rendered = render_home_manager(&m);
        let parsed = parse(&rendered);
        assert_eq!(parsed.packages, m.packages);
    }

    #[test]
    fn round_trip_nixos() {
        let mut m = Manifest::default();
        m.add("htop");
        let rendered = render_nixos(&m);
        let parsed = parse(&rendered);
        assert_eq!(parsed.packages, m.packages);
    }
}
