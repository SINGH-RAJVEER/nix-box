pub mod build;
pub mod manifest;
pub mod search;

pub use build::{home_manager_switch_cmd, nixos_rebuild_switch_cmd, rebuild, BuildEvent};
pub use manifest::{ensure_home_nix, Manifest, ManagedFile};
pub use search::{search, SearchHit};
