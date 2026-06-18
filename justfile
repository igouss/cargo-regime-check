# cargo-regime-check — gate a crate's public-API diff against regime-transition.toml.
#
# The gate builds/runs on STABLE. `cargo public-api` needs NIGHTLY (rustdoc JSON),
# so it is always invoked as `cargo +nightly public-api ...` and piped in.

# List recipes.
default:
    @just --list

# Install the gate locally (stable toolchain).
install:
    cargo install --path .

# Gate a crate's public API between a base ref and HEAD, JSON output (for agents).
# Usage: just regime-check beads-writer origin/main regime-transition.toml
regime-check crate base='origin/main' regime='regime-transition.toml':
    cargo +nightly public-api -p {{crate}} diff {{base}}..HEAD \
      | cargo-regime-check --regime {{regime}} --format json

# Same, human-readable report.
regime-check-human crate base='origin/main' regime='regime-transition.toml':
    cargo +nightly public-api -p {{crate}} diff {{base}}..HEAD \
      | cargo-regime-check --regime {{regime}}

# Gate from a captured diff file instead of invoking cargo public-api.
regime-check-file diff regime='regime-transition.toml':
    cargo-regime-check --regime {{regime}} --diff {{diff}} --format json

# Print a starter regime-transition.toml.
regime-template:
    cargo run -q -- --template

# Print the model + known approximations.
explain:
    cargo run -q -- --explain

# Dev gate: tests + clippy + fmt (the same bar CI enforces).
check:
    cargo test
    cargo clippy --all-targets -- -D warnings
    cargo fmt --check
