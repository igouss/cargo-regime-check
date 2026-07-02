//! Driven adapters: turn external formats into framework-free domain values and
//! render domain decisions back into those formats.
//!
//! - [`cargo_metadata`] — `cargo metadata` output -> the workspace's gated
//!   crates (the crates carrying a `regime-transition.toml`).
//! - [`diff_dir`] — a directory of pre-captured `<crate>.diff` files -> diff
//!   text: the `--diff-dir` source, which avoids the in-tree checkout
//!   `cargo public-api`'s `base..HEAD` form performs. A gated crate with no diff
//!   file is an error, never a silent skip.
//! - [`git`] — the two git queries workspace mode needs: working-tree dirtiness
//!   (default process mode refuses a dirty tree) and `base..HEAD` changed files
//!   mapped to the crates they touch (`--changed-only` skips untouched crates).
//! - [`public_api_diff`] — `cargo public-api diff` text -> [`crate::domain::ApiDiff`].
//! - [`public_api_process`] — shell out to `cargo +nightly public-api diff
//!   <base>..HEAD` -> diff text: the default diff source, split into a pure
//!   `args` seam and the process boundary.
//! - [`regime_file`] — `regime-transition.toml` <-> [`crate::domain::RegimeTransition`],
//!   plus the template and per-action remediation rendering.

pub mod cargo_metadata;
pub mod diff_dir;
pub mod git;
pub mod public_api_diff;
pub mod public_api_process;
pub mod regime_file;
