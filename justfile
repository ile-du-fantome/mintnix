# If already inside development shell, run commands directly.
#
# If nix is available but not in a development shell, prefix with
# `nix develop --command` to enter it automatically.
#
# If nix is not available at all, run commands directly.
[private]
_nix := `if [ -n "${IN_NIX_SHELL:-}" ]; then echo ""; elif command -v nix >/dev/null 2>&1; then echo "nix develop --command"; else echo ""; fi`

[private]
_hook_target := "x86_64-pc-windows-gnu"
[private]
_hook_debug := justfile_directory() + "/target/x86_64-pc-windows-gnu/debug/hook.dll"
[private]
_hook_release := justfile_directory() + "/target/x86_64-pc-windows-gnu/release/hook.dll"

# Show available recipes when invoked with no arguments
_default:
    @just --list

# Update flake.lock
update-flake:
    nix flake update

# Build the Windows hook for debug builds
hook:
    {{ _nix }} cargo build -p hook --target {{ _hook_target }}

# Build the Windows hook for release builds
hook-release:
    {{ _nix }} cargo build --release -p hook --target {{ _hook_target }}

# Build debug documentation
doc:
    {{ _nix }} cargo doc --document-private-items --workspace --all-features

# Build release documentation
doc-release:
    {{ _nix }} cargo doc --document-private-items --workspace --all-features --release

# Check debug compilation
check: hook
    {{ _nix }} env MINT_HOOK_DLL={{ _hook_debug }} cargo check

# Check release compilation
check-release: hook-release
    {{ _nix }} env MINT_HOOK_DLL={{ _hook_release }} cargo check --release

# Build debug binary
build: hook
    {{ _nix }} env MINT_HOOK_DLL={{ _hook_debug }} cargo build

# Build release binary
build-release: hook-release
    {{ _nix }} env MINT_HOOK_DLL={{ _hook_release }} cargo build --release

# Run debug binary
run: hook
    {{ _nix }} env MINT_HOOK_DLL={{ _hook_debug }} cargo run

# Run release binary
run-release: hook-release
    {{ _nix }} env MINT_HOOK_DLL={{ _hook_release }} cargo run --release

# Remove build artifacts
clean:
    {{ _nix }} cargo clean

# Check, build, and run debug binary
debug: check build run

# Check, build, and run release binary
release: check-release build-release run-release

# Run debug tests
test: hook
    {{ _nix }} env MINT_HOOK_DLL={{ _hook_debug }} cargo test

# Run release tests
test-release: hook-release
    {{ _nix }} env MINT_HOOK_DLL={{ _hook_release }} cargo test --release

# Run clippy lints
clippy: hook
    {{ _nix }} env MINT_HOOK_DLL={{ _hook_debug }} cargo clippy --all-targets

# Check code formatting
fmt:
    {{ _nix }} cargo fmt --check

# Check formatting, compilation, tests, and lints
validate: fmt check test clippy
    @echo "✓ All validations passed"
