default:
    @just --list

# ── Cargo ──────────────────────────────────────────────────────────────────────

# Build debug binary
build:
    cargo build --workspace

# Build release binary
release:
    cargo build --workspace --release

# Type-check without codegen
check:
    cargo check --workspace

# Run all tests
test:
    cargo test --workspace

# Run the TUI (debug build)
run:
    cargo run

# Format all crates
fmt:
    cargo fmt --all

# Lint with clippy
lint:
    cargo clippy --workspace -- -D warnings

# Format + lint
fix: fmt lint

# Remove build artifacts
clean:
    cargo clean

# ── Nix ────────────────────────────────────────────────────────────────────────

# Build the nixbox package via nix
nix-build:
    nix build --extra-experimental-features 'nix-command flakes'

# Run nixbox via nix (always builds from source)
nix-run:
    nix run --extra-experimental-features 'nix-command flakes'

# Update all flake inputs to latest
flake-update:
    nix flake update --extra-experimental-features 'nix-command flakes'

# Evaluate all flake outputs for errors
flake-check:
    nix flake check --extra-experimental-features 'nix-command flakes'

# Show the flake output tree
flake-show:
    nix flake show --extra-experimental-features 'nix-command flakes'

# ── Dev ────────────────────────────────────────────────────────────────────────

# Enter the nix dev shell
dev:
    nix develop --extra-experimental-features 'nix-command flakes'

# Full pre-commit gate: format, lint, test
ci: fmt lint test
