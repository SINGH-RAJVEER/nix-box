---

# ArchBox

**ArchBox** is a modern, extensible command-line tool, writtend in rust, for managing a curated set of essential tools and applications on Arch Linux. It provides a unified interface for installing, searching, updating, and configuring packages from official repositories, the AUR, binaries, AppImages, Flatpaks, and more.

## Features

- Unified CLI for common Arch Linux tools and applications
- Supports multiple installation methods: pacman, AUR, binaries, AppImage, Flatpak, source, and scripts
- Dependency resolution and post-install configuration
- Search, list, info, and removal commands
- Profile and group-based installations
- Interactive and non-interactive modes
- Shell completions and progress indicators
- YAML-based package definitions for easy extension

## Installation

```sh
git clone https://github.com/yourusername/archbox.git
cd archbox
cargo build --release
sudo cp target/release/archbox /usr/local/bin/
```

## Usage

- Install packages:  
  `archbox install neovim starship`
- Search for packages:  
  `archbox search editor`
- List available or installed packages:  
  `archbox list --installed`
- Show package info:  
  `archbox info neovim`
- Remove packages:  
  `archbox remove discord`
- Update definitions and packages:  
  `archbox update`
- Manage profiles:  
  `archbox profile list`
- Get recommendations:  
  `archbox recommend`

For all options, use `archbox --help`.

## Configuration

Configuration is stored at `~/.config/archbox/config.yaml`.  
You can view and edit settings using `archbox config`.

## Package Definitions

Package definitions are YAML files located in `data/packages/` or user-specified directories.  
Refer to the provided examples to add or modify packages.

## Contributing

Contributions are welcome. Please open issues or pull requests for bug fixes, new features, or package definitions.  
All contributions should follow Rust best practices and include appropriate documentation and tests where applicable.

---

**Maintainer:** [yourusername](https://github.com/SINGH-RAJVEER)

---
