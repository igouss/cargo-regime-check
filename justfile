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

# Gate EVERY workspace member carrying a regime-transition.toml in ONE run
# (discovered via `cargo metadata`; each crate uses its own regime file). This is
# the built-in replacement for a hand-maintained per-crate loop. Process mode needs
# a CLEAN tree — `cargo public-api` git-checks-out each commit in-tree — and refuses
# a dirty one. Aggregate exit = max over crates: 2 (error) > 1 (residual) > 0 (clean).
# Usage: just regime-check-workspace origin/main
regime-check-workspace base='origin/main':
    cargo-regime-check --workspace --base {{base}} --format json

# Same, but gate pre-captured diffs on STABLE — no nightly, no git checkout, no
# dirty-tree hazard. The CI two-stage pattern: a nightly job writes one
# `<crate>.diff` per gated crate into DIR, then this stable job gates them all. A
# gated crate with no diff in DIR is an error, never a silent skip.
# Usage: just regime-check-workspace-diff-dir target/regime-diffs
regime-check-workspace-diff-dir diff_dir='target/regime-diffs':
    cargo-regime-check --workspace --diff-dir {{diff_dir}} --format json

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
