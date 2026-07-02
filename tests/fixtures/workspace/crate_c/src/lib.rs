//! Minimal dependency-free lib. crate_c carries no regime-transition.toml, so it
//! is never gated; this source only makes `cargo metadata` see a valid member.

pub fn untracked_thing() {}
