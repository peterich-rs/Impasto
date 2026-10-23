//! Render tree. Layout goes through Taffy. This crate does not link Impeller.

mod error;
mod pointer;
mod shaper;
mod tree;

#[cfg(test)]
mod layout_test;

pub use error::Error;
pub use pointer::{Button, Modifiers, Phase, PointerEvent};
pub use shaper::{MonoShaper, SINGLE_LINE_MAX, Shaper, TextSize};
pub use tree::{Alignment, Discarded, Edges, Gesture, Justify, NodeId, Rect, TrackSize, Tree};
