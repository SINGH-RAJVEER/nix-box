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

# ── Dev ────────────────────────────────────────────────────────────────────────

# Enter the devenv shell
dev:
    devenv shell

# Update pinned devenv inputs
dev-update:
    devenv update

# Evaluate the devenv configuration and run its tests
dev-test:
    devenv test

# Full pre-commit gate: format, lint, test
ci: fmt lint test
