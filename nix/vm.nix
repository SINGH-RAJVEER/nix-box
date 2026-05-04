{ pkgs, lib, modulesPath, ... }:
{
  imports = [ "${modulesPath}/virtualisation/qemu-vm.nix" ];

  system.stateVersion = "24.11";

  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = false;

  networking.hostName = "nixbox-vm";
  networking.firewall.enable = false;

  nix.settings.experimental-features = [ "nix-command" "flakes" ];
  nix.settings.trusted-users = [ "@wheel" ];

  users.mutableUsers = false;
  users.users.tester = {
    isNormalUser = true;
    extraGroups = [ "wheel" ];
    password = "tester";
    home = "/home/tester";
  };
  security.sudo.wheelNeedsPassword = false;

  services.getty.autologinUser = "tester";

  users.motd = ''
    ── NixBox test VM ──────────────────────────────────────────
    Run the TUI:        nixbox

    NixOS-target test:
      sudo install -d -o tester /etc/nixos
      printf '{ pkgs, ... }: {\n  imports = [ ./nixbox-packages.nix ];\n}\n' \
        | sudo tee /etc/nixos/configuration.nix > /dev/null
      # In nixbox: Tab to set target=nixos, type a query, Enter to install.

    home-manager-target test:
      home-manager init --switch
      printf '\n  imports = [ ./nixbox-packages.nix ];\n' \
        >> ~/.config/home-manager/home.nix
      # In nixbox: leave target=home-manager (default), Enter on a package.

    Quit TUI: Esc.   Shut down VM: sudo poweroff.
    ────────────────────────────────────────────────────────────
  '';

  environment.systemPackages = with pkgs; [
    git
    home-manager
    vim
  ];

  virtualisation = {
    memorySize = 2048;
    cores = 2;
    diskSize = 8192;
    graphics = false;
    qemu.options = [ "-nographic" ];
  };

  # Persist /etc/nixos and the tester home across rebuilds inside the VM.
  systemd.tmpfiles.rules = [
    "d /etc/nixos 0755 tester users -"
  ];
}
