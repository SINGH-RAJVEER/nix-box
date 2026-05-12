# NixBox

A NixOS TUI package manager. Search a nixpkgs channel, pick a package, and NixBox writes it into your flake/home-manager config and runs the rebuild — all without leaving the terminal.

## What it does

- Live search against `nix search --json` over a configurable flake input (default `nixpkgs`).
- Maintains a managed file (`nixbox-packages.nix`) in your NixOS config dir. NixBox owns this file end-to-end; it never edits hand-written config.
- On select, appends the package, writes the file, then runs `home-manager switch` or `sudo nixos-rebuild switch` and streams the output into the TUI.
- Toggle target (home-manager / nixos) with `Tab`. Setting persists in `~/.config/nixbox/settings.json`.

## Wiring the managed file

Add a single import to your existing config so NixBox's file is picked up.

For **home-manager** (`~/.config/nixos/home.nix` or wherever your home config lives):

```nix
{
  imports = [ ./nixbox-packages.nix ];
}
```

For **NixOS system** (`/etc/nixos/configuration.nix`):

```nix
{
  imports = [ ./nixbox-packages.nix ];
}
```

After that, NixBox manages the package list inside `nixbox-packages.nix` between the `# nixbox:packages:start` / `# nixbox:packages:end` markers. Adjust nothing else in the file.

## Build

```sh
nix build           # via the flake
# or
cargo build --release
```

## Run

```sh
nix run            # via the flake
# or
./target/release/nixbox
```

## Layout

Cargo workspace:

- `crates/nixbox` — binary entrypoint
- `crates/nixbox-tui` — ratatui app, search/build views
- `crates/nixbox-nix` — `nix search` wrapper, managed file writer, rebuild runner
- `crates/nixbox-config` — persisted user settings (channel, target, paths)

## Keys

| key       | action                       |
| --------- | ---------------------------- |
| type      | search                       |
| ↑ / ↓     | move selection               |
| Enter     | install selected             |
| Tab       | toggle home-manager / nixos  |
| Esc / ^C  | quit                         |
| q         | leave build view             |
