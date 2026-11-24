//! Updates installed layers while retaining prepared pictures and GPU bundles.

use std::sync::Arc;

use engine::scene::{CompositorSnapshot, apply_compositor_samples};

use crate::{DamageRect, Rasterizer};

impl Rasterizer {
    /// Samples a route's installed scene without validation, tessellation, or mesh uploads.
    /// `None` requests a UI frame when a track ends, reaches an edge, or becomes stale.
    pub fn sample_compositor_tracks(
        &mut self,
        epoch: u64,
        target_ms: f64,
    ) -> Option<CompositorSnapshot> {
        let frame = self
            .frame
            .as_mut()
            .filter(|frame| frame.tree.epoch == epoch)?;
        let mut samples = Vec::with_capacity(frame.tree.tracks.len());
        for track in frame.tree.tracks.iter() {
            samples.push(track.sample(target_ms)?);
        }
        if !samples.is_empty() {
            let tree = Arc::make_mut(&mut frame.tree);
            if !apply_compositor_samples(&mut tree.layers, &tree.tracks, &samples) {
                return None;
            }
            frame.damage = DamageRect::Full;
            self.stats.damage = DamageRect::Full;
        }
        Some(CompositorSnapshot { target_ms, samples })
    }
}
