//! Minimal dependency-free lib. The gate reads a pre-captured diff, not this
//! source, so the body only has to make `cargo metadata` see a valid lib target.

pub fn new_name() -> u8 {
    0
}
