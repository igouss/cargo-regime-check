//! Pure domain: the classification calculus. No serde, no toml, no I/O.
//!
//! The declared renames are the functor `u` (arXiv:2606.01444); transporting old
//! items forward and comparing to the new surface leaves a *residual*; the
//! residual is what must be justified. Everything here is total and
//! deterministic so it can be tested against hand-built diffs without
//! `cargo public-api` present.

pub mod classify;
pub mod diff;
pub mod gate;
pub mod identity;
pub mod transition;

pub use classify::{classify, Class, Classified, MatchMode};
pub use diff::{ApiChange, ApiDiff, ApiItem};
pub use gate::{gate, required_action, GateResult, RequiredAction, Violation};
pub use identity::{ApiIdentity, ItemKind};
pub use transition::{Additive, Change, RegimeKind, RegimeTransition, Removal, Rename};
