# If already inside development shell, run commands directly.
#
# If nix is available but not in a development shell, prefix with
# `nix develop --command` to enter it automatically.
#
# If nix is not available at all, run commands directly.
[private]
_nix := `if [ -n "${IN_NIX_SHELL:-}" ]; then echo ""; elif command -v nix >/dev/null 2>&1; then echo "nix develop --command"; else echo ""; fi`

# Show available recipes when invoked with no arguments
_default:
    @just --list

# Update flake.lock
update-flake:
    nix flake update

# Build debug documentation
doc:
    {{ _nix }} cargo doc --document-private-items --workspace --all-features

# Build release documentation
doc-release:
    {{ _nix }} cargo doc --document-private-items --workspace --all-features --release

# Check debug compilation
check:
    {{ _nix }} cargo check

# Check release compilation
check-release:
    {{ _nix }} cargo check --release

# Build debug binary
build:
    {{ _nix }} cargo build

# Build release binary
build-release:
    {{ _nix }} cargo build --release

# Run debug binary
run:
    {{ _nix }} cargo run

# Run release binary
run-release:
    {{ _nix }} cargo run --release

# Remove build artifacts
clean:
    {{ _nix }} cargo clean

# Check, build, and run debug binary
debug: check build run

# Check, build, and run release binary
release: check-release build-release run-release

# Run debug tests
test:
    {{ _nix }} cargo test

# Run release tests
test-release:
    {{ _nix }} cargo test --release

# Run clippy lints
clippy:
    {{ _nix }} cargo clippy --all-targets

# Check code formatting
fmt:
    {{ _nix }} cargo fmt --check

# Check formatting, compilation, tests, and lints
validate: fmt check test clippy
    @echo "✓ All validations passed"
