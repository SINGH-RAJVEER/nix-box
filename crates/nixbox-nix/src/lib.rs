pub mod build;
pub mod manifest;
pub mod scan;
pub mod search;

pub use build::{
    BuildEvent, flake_has_home_configuration, home_manager_switch_cmd, nixos_rebuild_switch_cmd,
    rebuild,
};
pub use manifest::{ImportStatus, ManagedFile, Manifest, ensure_home_nix, ensure_imported};
pub use scan::{ExternalPackage, ScanTarget, remove_from_source, scan};
pub use search::{SearchHit, search};
