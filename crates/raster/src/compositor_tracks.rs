//! Updates installed layers while retaining prepared pictures and GPU bundles.

use std::sync::Arc;

use engine::scene::{
    CompositorProp, CompositorSample, CompositorSnapshot, apply_compositor_samples,
};

use crate::{DamageRect, Frame, Rasterizer};

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
            frame.apply_shader_uniform_samples(&samples);
            frame.damage = DamageRect::Full;
            self.stats.damage = DamageRect::Full;
        }
        Some(CompositorSnapshot { target_ms, samples })
    }
}

impl Frame {
    fn apply_shader_uniform_samples(&mut self, samples: &[CompositorSample]) {
        self.shader_uniforms.clear();
        for (track, sample) in self.tree.tracks.iter().zip(samples) {
            if let CompositorProp::ShaderUniform(index) = track.prop {
                let values = self.shader_uniforms.entry(track.layer).or_default();
                if let Some(value) = values.get_mut(usize::from(index)) {
                    *value = Some(sample.value);
                }
            }
        }
    }
}
