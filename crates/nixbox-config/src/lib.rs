use std::fs;
use std::path::PathBuf;

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

const MAX_RECENT_SEARCHES: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub channel: String,
    pub target: Target,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub recent_searches: Vec<String>,
}

fn default_theme() -> String {
    "default".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channel: "nixpkgs".to_string(),
            target: Target::NixosSystem,
            theme: default_theme(),
            recent_searches: Vec::new(),
        }
    }
}

impl Config {
    /// Pushes a query to the front of recent_searches, dedups, caps the list.
    pub fn push_recent(&mut self, query: String) {
        let trimmed = query.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        self.recent_searches.retain(|q| q != &trimmed);
        self.recent_searches.insert(0, trimmed);
        if self.recent_searches.len() > MAX_RECENT_SEARCHES {
            self.recent_searches.truncate(MAX_RECENT_SEARCHES);
        }
    }
}

impl Config {
    /// Returns the path where nixbox writes its managed packages file for the current target.
    pub fn managed_file(&self) -> PathBuf {
        match self.target {
            Target::HomeManager => nixos_config_dir().join("nixbox-home-packages.nix"),
            Target::NixosSystem => nixos_config_dir().join("nixbox-system-packages.nix"),
        }
    }

    /// Returns the directory where nixbox keeps its generated nix files.
    pub fn home_manager_dir(&self) -> PathBuf {
        nixos_config_dir()
    }

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

fn nixos_config_dir() -> PathBuf {
    if let Some(base) = BaseDirs::new() {
        base.config_dir().join("vm")
    } else {
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/vm")
    }
}
