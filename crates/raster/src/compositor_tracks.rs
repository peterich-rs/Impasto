//! Updates installed layers while retaining prepared pictures and GPU bundles.

use engine::scene::{
    CompositorProp, CompositorSample, CompositorSnapshot, ScrollTargetProp, ScrollValue,
};

use crate::{Frame, Rasterizer, damage};

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
            frame.damage = damage::sample(frame, &samples)?;
            frame.apply_shader_uniform_samples(&samples);
            frame.previous_revision = frame.revision;
            frame.revision = frame.revision.wrapping_add(1);
            self.stats.damage = frame.damage;
        }
        Some(CompositorSnapshot { target_ms, samples })
    }
}

impl Frame {
    fn apply_shader_uniform_samples(&mut self, samples: &[CompositorSample]) {
        self.shader_uniforms.clone_from(&self.tree.shader_uniforms);
        for (track, sample) in self.tree.tracks.iter().zip(samples) {
            for binding in track.scroll_bindings.iter() {
                if let (ScrollTargetProp::ShaderUniform(index), ScrollValue::Scalar(value)) =
                    (binding.prop, binding.curve.sample(sample.value))
                {
                    let values = self.shader_uniforms.entry(binding.layer).or_default();
                    if let Some(slot) = values.get_mut(usize::from(index)) {
                        *slot = Some(value);
                    }
                }
            }
            if let CompositorProp::ShaderUniform(index) = track.prop {
                let values = self.shader_uniforms.entry(track.layer).or_default();
                if let Some(value) = values.get_mut(usize::from(index)) {
                    *value = Some(sample.value);
                }
            }
        }
    }
}
