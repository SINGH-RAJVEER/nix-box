# NixBox

A NixOS TUI package manager. Search a nixpkgs channel, pick a package, and NixBox writes it into your home-manager or NixOS config and runs the rebuild — without ever leaving the terminal.

## What it does

- Live search against `nix search --json` over a configurable flake input (default `nixpkgs`).
- Maintains two managed files in your config directory — `nixbox-home-packages.nix` and `nixbox-system-packages.nix` — and owns them end-to-end. Your hand-written config is never touched outside of a single `imports` line.
- On install/uninstall it updates the managed file, makes sure it's imported by your `home.nix` / `configuration.nix`, then runs the appropriate rebuild and streams the output into the TUI.
- Works whether your home-manager is exposed as a standalone `homeConfigurations.<user>` flake output, or wired in as a NixOS module — NixBox auto-detects which one you have and picks the right rebuild command.
- Scans your existing config for externally-declared packages and lets you "migrate" them into the managed file with `m` (or `M` for all of them).
- Settings (channel, target, theme, paths) persist in `~/.config/nixbox/settings.json`.

## How it wires itself in

The first time you install or migrate a package, NixBox does three things automatically:

1. Writes the managed file (`nixbox-home-packages.nix` for home-manager, `nixbox-system-packages.nix` for NixOS).
2. Inserts `./nixbox-home-packages.nix` (or `…-system-…`) into the `imports = [ … ]` list of your `home.nix` / `configuration.nix`. Existing imports-list style is preserved, and the insertion is idempotent.
3. Stages the managed file with `git add -N` if your config dir is a git work tree, so flakes (which ignore untracked files) can actually evaluate it.

If you want to override where NixBox looks for the "main" config file, set `home_manager_main_file` or `nixos_main_file` in `~/.config/nixbox/settings.json`.

Inside the managed file, NixBox owns everything between `# nixbox:packages:start` and `# nixbox:packages:end`. Don't edit those by hand.

## Install

```sh
cargo install nixbox
```

Or build from source:

```sh
nix build       # via the flake
cargo build --release
```

## Run

```sh
nix run         # via the flake
nixbox          # if cargo-installed
just run        # from a checkout
```

## Layout

Cargo workspace:

- `crates/nixbox` — binary entrypoint
- `crates/nixbox-tui` — ratatui app, search / installed / build views
- `crates/nixbox-nix` — `nix search` wrapper, managed-file writer, import inserter, rebuild runner
- `crates/nixbox-config` — persisted user settings (channel, target, theme, path overrides)

## Keys

| key                | action                                  |
| ------------------ | --------------------------------------- |
| type / `/` / `i`   | enter search                            |
| `↑` `↓` / `k` `j`  | move selection                          |
| Enter              | install selected package                |
| `d` / Delete       | uninstall selected (Installed tab)      |
| `m`                | migrate selected external package       |
| `M`                | migrate all migratable externals        |
| Tab / `l`          | next tab (Search → Installed → Build)   |
| Shift-Tab / `h`    | previous tab                            |
| Ctrl-T             | toggle home-manager / nixos target      |
| Ctrl-N             | cycle theme                             |
| Esc / Ctrl-C       | quit                                    |
