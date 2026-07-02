//! `cargo-regime-check` — classify a public-API diff as an endofunctorial
//! refactor (residual = 0) or a regime transition whose residual must be
//! declared, and gate undeclared residual.
//!
//! Hexagonal:
//! - [`domain`] is pure (the classification calculus + identity parsing).
//! - [`adapters`] turn `regime-transition.toml` and `cargo public-api` text into
//!   domain values, and render the gate's decisions back into the TOML format.
//! - [`report`] is the presentation layer (the stable view-model + human/json).
//! - [`pipeline`] composes the per-crate `parse → classify → gate → build`
//!   sequence once, so the single-crate CLI and workspace mode share it.
//! - [`workspace`] is the workspace-mode orchestration use-case: the impure glue
//!   that drives the adapters + pipeline and folds the per-crate results through
//!   [`report::workspace`].
//! - the binary (`src/bin/cargo-regime-check.rs`) is the driving adapter.

pub mod adapters;
pub mod domain;
pub mod pipeline;
pub mod report;
pub mod workspace;
