{ lib, rustPlatform, fetchCrate }:

rustPlatform.buildRustPackage rec {
  pname = "nixbox";
  version = "0.1.0";

  src = fetchCrate {
    inherit pname version;
    hash = "sha256-+xOoEV4wUxLmeHZd7W32sVAMu6cCP4/NlSHAURkeCOM=";
  };

  cargoHash = lib.fakeHash;

  meta = with lib; {
    description = "TUI package manager for NixOS that wires selections into your flake + home-manager config";
    homepage = "https://github.com/SINGH-RAJVEER/nix-box";
    license = licenses.asl20;
    maintainers = with maintainers; [ ];
    mainProgram = "nixbox";
    platforms = platforms.linux;
  };
}
