//! Render tree. Layout goes through Taffy. This crate does not link Impeller.

mod error;
mod pointer;
mod shaper;
mod tree;

#[cfg(test)]
mod layout_test;

pub use error::Error;
pub use pointer::{Phase, PointerEvent};
pub use shaper::{MonoShaper, SINGLE_LINE_MAX, Shaper, TextSize};
pub use tree::{Alignment, Discarded, Gesture, Justify, NodeId, Rect, Tree};
