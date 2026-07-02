//! Minimal dependency-free lib. The gate reads a pre-captured diff, not this
//! source, so the body only has to make `cargo metadata` see a valid lib target.

pub fn renamed_to() -> u8 {
    0
}

pub fn undeclared_feature() -> i32 {
    0
}
