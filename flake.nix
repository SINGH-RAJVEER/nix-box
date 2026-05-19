{
  description = "NixBox — TUI package manager that wires NixOS / home-manager configs";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    let
      perSystem = flake-utils.lib.eachDefaultSystem (system:
        let
          overlays = [ (import rust-overlay) ];
          pkgs = import nixpkgs { inherit system overlays; };
          rustToolchain = pkgs.rust-bin.stable.latest.default.override {
            extensions = [ "rust-src" "rust-analyzer" ];
          };
          nixbox = pkgs.rustPlatform.buildRustPackage {
            pname = "nixbox";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [ rustToolchain ];
          };
        in {
          packages = {
            default = nixbox;
            nixbox = nixbox;
            nixbox-crate = pkgs.callPackage ./nix/package.nix {};
          };

          devShells.default = pkgs.mkShell {
            packages = [
              rustToolchain
              pkgs.nix
            ];

            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          };
        });
    in
    perSystem;
}
