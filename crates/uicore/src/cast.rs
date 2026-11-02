//! Width changes at the tree's stored integer sizes.
//!
//! A value that does not fit saturates. The previous `as` cast wrapped, which
//! aliased a huge length back to a small index.

/// `usize` length or slot index stored as `u32`.
pub(crate) fn u32_from_usize(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// `usize` index stored as `i32`, including the sticky-row sentinel domain.
pub(crate) fn i32_from_usize(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// Sample an `f64` clock or ratio into the `f32` motion and velocity state.
pub(crate) fn f32_from_f64(value: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "motion samples and pointer velocity are stored as f32"
    )]
    {
        value as f32
    }
}
