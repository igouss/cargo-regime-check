//! Driven adapters: turn external formats into framework-free domain values and
//! render domain decisions back into those formats.
//!
//! - [`public_api_diff`] — `cargo public-api diff` text -> [`crate::domain::ApiDiff`].
//! - [`regime_file`] — `regime-transition.toml` <-> [`crate::domain::RegimeTransition`],
//!   plus the template and per-action remediation rendering.

pub mod public_api_diff;
pub mod regime_file;
