use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Target {
    HomeManager,
    NixosSystem,
}

impl Target {
    pub fn label(self) -> &'static str {
        match self {
            Target::HomeManager => "home-manager",
            Target::NixosSystem => "nixos",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub channel: String,
    pub target: Target,
    pub config_dir: PathBuf,
    pub managed_file: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        let config_dir = default_nixos_dir();
        let managed_file = config_dir.join("nixbox-packages.nix");
        Self {
            channel: "nixpkgs".to_string(),
            target: Target::HomeManager,
            config_dir,
            managed_file,
        }
    }
}

impl Config {
    pub fn load_or_default() -> Result<Self> {
        let path = settings_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let cfg: Config = serde_json::from_str(&raw)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let path = settings_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let raw = serde_json::to_string_pretty(self)?;
        fs::write(&path, raw)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

pub fn settings_path() -> Result<PathBuf> {
    let base = BaseDirs::new().context("locating user directories")?;
    Ok(base.config_dir().join("nixbox").join("settings.json"))
}

fn default_nixos_dir() -> PathBuf {
    if Path::new("/etc/nixos").exists() {
        PathBuf::from("/etc/nixos")
    } else if let Some(base) = BaseDirs::new() {
        base.config_dir().join("nixos")
    } else {
        PathBuf::from(".")
    }
}
