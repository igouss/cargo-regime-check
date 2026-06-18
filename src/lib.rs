//! `cargo-regime-check` — classify a public-API diff as an endofunctorial
//! refactor (residual = 0) or a regime transition whose residual must be
//! declared, and gate undeclared residual.
//!
//! Hexagonal:
//! - [`domain`] is pure (the classification calculus + identity parsing).
//! - [`adapters`] turn `regime-transition.toml` and `cargo public-api` text into
//!   domain values, and render the gate's decisions back into the TOML format.
//! - [`report`] is the presentation layer (the stable view-model + human/json).
//! - the binary (`src/bin/cargo-regime-check.rs`) is the driving adapter.

pub mod adapters;
pub mod domain;
pub mod report;
