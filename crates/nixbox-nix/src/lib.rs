pub mod build;
pub mod manifest;
pub mod scan;
pub mod search;

pub use build::{
    flake_has_home_configuration, home_manager_switch_cmd, nixos_rebuild_switch_cmd, rebuild,
    BuildEvent,
};
pub use manifest::{ensure_home_nix, ensure_imported, ImportStatus, ManagedFile, Manifest};
pub use scan::{remove_from_source, scan, ExternalPackage, ScanTarget};
pub use search::{search, SearchHit};
