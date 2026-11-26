//! Replays runtime effects alongside normal draws using the scene's coverage state.

use std::sync::Arc;

use bytemuck::Zeroable;
use engine::scene::{BACKDROP_TEXTURE, MAX_SHADER_TEXTURES, ShaderProgramId};

use super::{Coverage, Frame, Image, pipelines};
use crate::Status;
use crate::arena::Extra;
use crate::compiler::{Affine, PictureBounds};
use crate::runtime_effect::EffectParams;

pub(super) struct TextureGroup {
    group: wgpu::BindGroup,
    pub used: bool,
    pub backdrops: [bool; MAX_SHADER_TEXTURES],
}

impl Frame<'_> {
    #[expect(
        clippy::too_many_arguments,
        reason = "each recorded effect owns its inputs and scene coverage"
    )]
    pub(super) fn shader_effect(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        bounds: [f32; 4],
        id: ShaderProgramId,
        uniforms: &[u8],
        textures: &[Arc<str>],
        overrides: Option<&[Option<f32>; 32]>,
    ) -> Result<(), Status> {
        let Some(area) = self.area(PictureBounds::rect(bounds).transform(transform), target) else {
            return Ok(());
        };
        let Some(program) = self.scene.tree.shader_program(id) else {
            return Ok(());
        };
        let Some(pipeline) = self.compositor.runtime_effects.pipeline(
            self.gpu,
            &self.compositor.pipelines.layouts,
            program,
            target.format,
        ) else {
            return Ok(());
        };
        let backdrop = textures
            .iter()
            .take(MAX_SHADER_TEXTURES)
            .any(|source| source.as_ref() == BACKDROP_TEXTURE)
            .then(|| self.snapshot(target, area))
            .transpose()?;
        let mut params = EffectParams::zeroed();
        let (uniform_words, _) = uniforms.as_chunks::<4>();
        for (value, bytes) in params.uniforms.iter_mut().flatten().zip(uniform_words) {
            let parsed = f32::from_le_bytes(*bytes);
            if parsed.is_finite() {
                *value = parsed;
            }
        }
        if let Some(overrides) = overrides {
            for (value, animated) in params.uniforms.iter_mut().flatten().zip(overrides) {
                if let Some(animated) = animated {
                    *value = *animated;
                }
            }
        }
        let compositor = &mut self.compositor;
        let mut views = [&compositor.white; MAX_SHADER_TEXTURES];
        let mut backdrops = [false; MAX_SHADER_TEXTURES];
        for (index, source) in textures.iter().take(MAX_SHADER_TEXTURES).enumerate() {
            if source.as_ref() == BACKDROP_TEXTURE {
                if let Some(backdrop) = &backdrop {
                    views[index] = &backdrop.view;
                    backdrops[index] = true;
                    params.mappings[index] = backdrop.uv_bounds();
                    params.sample_modes[index] = 1;
                }
            } else if let Some(image) = self.scene.textures.get(source)
                && image.nv12().is_none()
            {
                views[index] = image.view();
                params.mappings[index] = [0.0, 0.0, 1.0, 1.0];
                params.sample_modes[index] = if image.premultiplied_srgb { 2 } else { 1 };
            }
        }
        let cached = compositor
            .runtime_texture_groups
            .entry(views.map(Clone::clone))
            .or_insert_with_key(|views| {
                compositor.render_stats.bind_groups_created += 1;
                TextureGroup {
                    group: pipelines::bind_group(
                        self.gpu,
                        &compositor.pipelines.layouts.runtime_textures,
                        &[
                            wgpu::BindingResource::TextureView(&views[0]),
                            wgpu::BindingResource::TextureView(&views[1]),
                            wgpu::BindingResource::TextureView(&views[2]),
                            wgpu::BindingResource::TextureView(&views[3]),
                            wgpu::BindingResource::Sampler(&compositor.linear),
                        ],
                    ),
                    used: false,
                    backdrops,
                }
            });
        cached.used = true;
        let material = cached.group.clone();
        let time = if program.animated {
            self.compositor.effect_seconds
        } else {
            0.0
        };
        let view = self.geometry_uniform(target, transform, clip, time)?;
        let uniforms = self.uniform(&params)?;
        let globals = self.globals(view, Extra::RuntimeEffect(uniforms));
        let mask = self.mask_group(clip);

        // Normal draws share the enclosing render pass. During a capture the
        // effect gets timestamp boundaries so its cost can be attributed.
        let measuring = self.capture.is_some();
        if measuring {
            self.flush_target(&target.view);
        }
        self.record(
            target,
            pipeline,
            globals,
            material,
            Some(mask),
            &[bounds],
            clip.scissor,
            6,
        )?;
        if measuring {
            self.pending_pass(target).shader = Some(id);
        }
        if measuring || backdrop.is_some() {
            self.flush_target(&target.view);
        }
        if let Some(backdrop) = backdrop {
            self.release(backdrop);
        }
        Ok(())
    }
}
