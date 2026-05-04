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
          rustToolchain = pkgs.rust-bin.stable.latest.default;
          nixbox = pkgs.rustPlatform.buildRustPackage {
            pname = "nixbox";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [ rustToolchain ];
          };
          isVmHost = system == "x86_64-linux";
          vmRunner = pkgs.writeShellApplication {
            name = "run-nixbox-vm";
            runtimeInputs = [ pkgs.nixos-rebuild ];
            text = ''
              set -euo pipefail
              nixos-rebuild build-vm --flake "${self}#nixbox-vm"
              exec ./result/bin/run-nixbox-vm-vm "$@"
            '';
          };
        in {
          packages = {
            default = nixbox;
            nixbox = nixbox;
          } // nixpkgs.lib.optionalAttrs isVmHost {
            vm = self.nixosConfigurations.nixbox-vm.config.system.build.vm;
          };

          devShells.default = pkgs.mkShell {
            packages = [
              rustToolchain
              pkgs.rust-analyzer
              pkgs.nix
            ];
          };

          apps = nixpkgs.lib.optionalAttrs isVmHost {
            vm = {
              type = "app";
              program = "${vmRunner}/bin/run-nixbox-vm";
            };
          };
        });
    in
    perSystem // {
      nixosConfigurations.nixbox-vm = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          ./nix/vm.nix
          ({ pkgs, ... }: {
            environment.systemPackages = [
              self.packages.${pkgs.system}.nixbox
            ];
          })
        ];
      };
    };
}
