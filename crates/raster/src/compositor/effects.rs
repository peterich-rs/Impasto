//! Offscreen effects, blur, and fullscreen composition.

use super::{
    CompositeDraw, Coverage, Frame, Image, add_corner_clip, expand_pixels, intersect_pixels,
    pass_timestamps, single,
};
use crate::Status;
use crate::arena::Extra;
use crate::batch;
use crate::compiler::{self, Affine, Clip, Effect, Group, PictureBounds};
use crate::compute_blur::{BlurParams, ComputeBlur};
use crate::pipelines::{self, BlendKey, ParamsUniform, PipelineKind, ViewUniform};
use crate::profiler::GpuPassKind;

#[derive(Clone, Copy)]
pub(super) enum FullPass {
    Copy,
    DisplayP3,
    EncodeSrgb,
    Over {
        blend: BlendKey,
        opacity: f32,
    },
    Multiply {
        opacity: f32,
    },
    Backdrop,
    BackdropMaterial {
        material: engine::scene::BackdropMaterial,
        top: f32,
        inverse_height: f32,
    },
    ColorFilter(engine::scene::ColorMatrix),
    BlurDown {
        radius: f32,
        backdrop: bool,
    },
    BlurUp {
        radius: f32,
        backdrop: bool,
    },
    Gaussian {
        sigma: f32,
        vertical: bool,
        backdrop: bool,
    },
    MaskIntersect,
}

impl Frame<'_> {
    pub(super) fn paint_backdrop(
        &mut self,
        target: &Image,
        coverage: &Coverage,
        radius: f32,
        material: Option<FullPass>,
    ) -> Result<(), Status> {
        let dst = coverage.scissor.unwrap_or_else(|| target.area());
        let src = expand_pixels(dst, radius, target.area());
        let kind = material.unwrap_or_else(|| {
            if coverage.mask.is_none() && coverage.count == 0 {
                FullPass::Copy
            } else {
                FullPass::Backdrop
            }
        });
        if radius <= 0.0 {
            let snapshot = self.snapshot(target, src)?;
            self.full(&snapshot, Some(&snapshot), coverage, target, dst, kind)?;
            self.release(snapshot);
            return Ok(());
        }
        let snapshot = self.snapshot(target, src)?;
        let blurred = self.blur(&snapshot, radius, true)?;
        self.full(&blurred, Some(&snapshot), coverage, target, dst, kind)?;
        self.release(blurred);
        self.release(snapshot);
        Ok(())
    }

    pub(super) fn group(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Clip,
        group: Group,
        children: impl FnOnce(&mut Self, &Image) -> Result<(), Status>,
    ) -> Result<(), Status> {
        if group.opacity <= 0.0 {
            return Ok(());
        }
        let radius = group.sigma * self.scene.scale * compiler::effect_scale(transform);
        let valid_blur = radius.is_finite() && radius > 0.0;
        let mut group_clip = clip.clone();
        let corners = match group.effect {
            Effect::BackdropMaterial(_, corners) => corners,
            _ => None,
        };
        if let Some(bounds) = group.bounds {
            add_corner_clip(&mut group_clip, bounds, 0.0, corners, transform);
        }
        if let Effect::BackdropMaterial(material, _) = group.effect {
            if material.is_valid()
                && let Some(bounds) = group.bounds
                && let PictureBounds::Rect([_, top, _, height]) =
                    PictureBounds::rect(bounds).transform(transform)
            {
                let coverage = self.coverage(target, &group_clip)?;
                if !coverage.empty {
                    self.paint_backdrop(
                        target,
                        &coverage,
                        if valid_blur { radius } else { 0.0 },
                        Some(FullPass::BackdropMaterial {
                            material,
                            top: top * self.scene.scale,
                            inverse_height: 1.0 / (height * self.scene.scale),
                        }),
                    )?;
                }
            }
            return children(self, target);
        }
        if group.effect == Effect::BackdropBlur {
            if valid_blur {
                let coverage = self.coverage(target, &group_clip)?;
                if !coverage.empty {
                    self.paint_backdrop(target, &coverage, radius, None)?;
                }
            }
            return children(self, target);
        }

        let mut content = group.content;
        if let Some(rect) = group_clip.scissor {
            content = content.clip(rect);
        }
        let filter = group.effect == Effect::Blur && valid_blur;
        if filter {
            content = content.expand(3.0 * radius / self.scene.scale);
        }
        let Some(area) = self.area(content.expand(1.0 / self.scene.scale), target) else {
            return Ok(());
        };
        let source = self.take_area(area, self.working, 1);
        self.clear(&source.view);
        children(self, &source)?;
        self.finish(&source.view);
        let blurred = if filter {
            Some(self.blur(&source, radius, false)?)
        } else {
            None
        };

        let mode = match group.effect {
            Effect::Blend(mode) => batch::blend_mode(mode),
            Effect::Canvas(mode) => batch::canvas_composite(mode),
            _ => 0,
        };
        let blend = BlendKey::from_mode(mode);
        let snapshot = blend
            .is_none()
            .then(|| self.snapshot(target, area))
            .transpose()?;
        let coverage = if filter {
            self.coverage(target, &group_clip)?
        } else {
            Coverage::default()
        };
        if !coverage.empty {
            let visible = match coverage.scissor {
                Some(scissor) => intersect_pixels(scissor, area),
                None => Some(area),
            };
            if let Some(visible) = visible {
                self.full(
                    blurred.as_ref().unwrap_or(&source),
                    snapshot.as_ref(),
                    &coverage,
                    target,
                    visible,
                    if let Effect::ColorFilter(matrix) = group.effect {
                        FullPass::ColorFilter(matrix)
                    } else {
                        blend.map_or(
                            FullPass::Multiply {
                                opacity: group.opacity,
                            },
                            |blend| FullPass::Over {
                                blend,
                                opacity: group.opacity,
                            },
                        )
                    },
                )?;
            }
        }
        if let Some(snapshot) = snapshot {
            self.release(snapshot);
        }
        if let Some(blurred) = blurred {
            self.release(blurred);
        }
        self.release(source);
        Ok(())
    }

    pub(super) fn snapshot(&mut self, target: &Image, area: [u32; 4]) -> Result<Image, Status> {
        if self.deferred_root.is_some() && target.view == self.root {
            return Err(Status::BadScene);
        }
        self.finish(&target.view);
        let snapshot = self.take_area(area, target.format, 1);
        self.copy_attachment(target, &snapshot, area)?;
        Ok(snapshot)
    }

    pub(super) fn blur(
        &mut self,
        source: &Image,
        radius: f32,
        backdrop: bool,
    ) -> Result<Image, Status> {
        let mut levels = Vec::new();
        let mut size = source.logical;
        let mut remaining = radius;
        let origin = source.origin;
        let format = source.format;
        while remaining > 8.0 && levels.len() < 8 && size[0].max(size[1]) > 1 {
            size = size.map(|dim| dim.div_ceil(2));
            let down = self.take_area([origin[0], origin[1], size[0], size[1]], format, 1);
            let from = levels.last().unwrap_or(source);
            self.blur_pass(
                from,
                &down,
                FullPass::BlurDown {
                    radius: 1.0,
                    backdrop,
                },
            )?;
            levels.push(down);
            // Down/up sampling adds a small amount of variance.
            remaining = (remaining * remaining - 1.5).sqrt() * 0.5;
        }
        let input = levels.last().unwrap_or(source);
        // The apron fits sigma <= 8 at this pyramid level. The render path
        // is retained for larger kernels and for capabilities without storage.
        let [width, height] = input.logical;
        let hdr = format == wgpu::TextureFormat::Rgba16Float;
        let mut filtered = if (2.5..=8.0).contains(&remaining)
            && width.saturating_mul(height) >= 4096
            && let Some(pipeline) = self.compositor.compute_blur[usize::from(hdr)]
                .get_or_insert_with(|| ComputeBlur::new(self.gpu, input.format))
                .clone()
        {
            self.compute_blur(&pipeline, input, remaining)?
        } else {
            let scratch = self.take_area(input.area(), format, 1);
            let image = self.take_area(input.area(), format, 1);
            self.blur_pass(
                input,
                &scratch,
                FullPass::Gaussian {
                    sigma: remaining,
                    vertical: false,
                    backdrop,
                },
            )?;
            self.blur_pass(
                &scratch,
                &image,
                FullPass::Gaussian {
                    sigma: remaining,
                    vertical: true,
                    backdrop,
                },
            )?;
            self.release(scratch);
            image
        };
        while let Some(level) = levels.pop() {
            let [w, h] = levels.last().map_or(source.logical, |image| image.logical);
            let next = self.take_area([origin[0], origin[1], w, h], format, 1);
            self.blur_pass(
                &filtered,
                &next,
                FullPass::BlurUp {
                    radius: 0.75,
                    backdrop,
                },
            )?;
            self.release(level);
            self.release(filtered);
            filtered = next;
        }
        Ok(filtered)
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "sigma is between 2.5 and 8 at the call site"
    )]
    fn compute_blur(
        &mut self,
        kernel: &ComputeBlur,
        source: &Image,
        sigma: f32,
    ) -> Result<Image, Status> {
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING;
        let scratch = self.take_area_with_usage(source.area(), kernel.format, 1, usage);
        let output = self.take_area_with_usage(source.area(), kernel.format, 1, usage);
        let [w, h] = source.logical;
        let params = self.uniform(&BlurParams {
            size: [w, h],
            radius: (sigma * 3.0).ceil() as u32,
            sigma,
        })?;
        for (axis, (input, target)) in [(source, &scratch), (&scratch, &output)]
            .into_iter()
            .enumerate()
        {
            let pipeline = kernel.pipeline(axis);
            let (buffer, offset) = self.compositor.arena.uniform_buffer(params);
            let storage = target.storage_view.as_ref().ok_or(Status::BadScene)?;
            let key = (input.view.clone(), storage.clone(), buffer.clone());
            let group = self.compositor.blur_groups.entry(key).or_insert_with_key(
                |(input, storage, buffer)| {
                    self.compositor.render_stats.bind_groups_created += 1;
                    pipelines::bind_group(
                        self.gpu,
                        &kernel.layout,
                        &[
                            wgpu::BindingResource::TextureView(input),
                            wgpu::BindingResource::TextureView(storage),
                            wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer,
                                offset: 0,
                                size: wgpu::BufferSize::new(size_of::<BlurParams>() as u64),
                            }),
                        ],
                    )
                },
            );
            let stamps = pass_timestamps(&mut self.capture, GpuPassKind::ComputeBlur, 0);
            {
                let mut pass = self
                    .command
                    .begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("impasto compute blur"),
                        timestamp_writes: stamps.map(|writes| wgpu::ComputePassTimestampWrites {
                            query_set: writes.query_set,
                            beginning_of_pass_write_index: writes.beginning_of_pass_write_index,
                            end_of_pass_write_index: writes.end_of_pass_write_index,
                        }),
                    });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &*group, &[offset]);
                let [tw, th] = if axis == 0 { [16, 8] } else { [8, 16] };
                pass.dispatch_workgroups(w.div_ceil(tw), h.div_ceil(th), 1);
            }
            self.count_pass(&target.view);
            self.compositor.render_stats.compute_passes += 1;
            self.compositor.render_stats.blur_pixels += u64::from(w) * u64::from(h);
        }
        self.release(scratch);
        Ok(output)
    }

    fn blur_pass(&mut self, source: &Image, to: &Image, kind: FullPass) -> Result<(), Status> {
        self.clear(&to.view);
        self.full(source, None, &Coverage::default(), to, to.area(), kind)
    }

    pub(super) fn full(
        &mut self,
        source: &Image,
        backdrop: Option<&Image>,
        coverage: &Coverage,
        target: &Image,
        area: [u32; 4],
        kind: FullPass,
    ) -> Result<(), Status> {
        self.flush_draws();
        let view = self.uniform(&ViewUniform {
            transform: [
                [2.0, 0.0, 0.0, 0.0],
                [0.0, -2.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [-1.0, 1.0, 0.0, 1.0],
            ],
            ..coverage.view(target, 1.0)
        })?;
        let value = match kind {
            FullPass::Over { opacity, .. } | FullPass::Multiply { opacity } => opacity,
            FullPass::BlurDown { radius, .. } | FullPass::BlurUp { radius, .. } => radius,
            FullPass::Gaussian { sigma, .. } => sigma,
            _ => 0.0,
        };
        let params = ParamsUniform {
            values: [
                source.origin[0] as f32 + source.sample_offset[0],
                source.origin[1] as f32 + source.sample_offset[1],
                value,
                f32::from(
                    matches!(kind, FullPass::Gaussian { vertical: true, .. })
                        || (target.view == self.root && self.straight_alpha),
                ),
            ],
            uv_bounds: source.uv_bounds(),
            backdrop_origin: backdrop.map_or([0.0; 4], |image| {
                [image.origin[0] as f32, image.origin[1] as f32, 0.0, 0.0]
            }),
            backdrop_bounds: backdrop.map_or([0.0; 4], Image::uv_bounds),
        };
        let immediates = self.gpu.effect_immediates();
        let extra = if immediates {
            Extra::None
        } else {
            Extra::Params(self.uniform(&params)?)
        };
        let globals = self.globals(view, extra);
        let input = self.sample_attachment(source, false);
        let composite = match kind {
            FullPass::Over { .. }
            | FullPass::Multiply { .. }
            | FullPass::Backdrop
            | FullPass::BackdropMaterial { .. } => Some(self.composite_group(
                backdrop.map(|image| image.view.clone()),
                coverage.mask.as_ref().map(|image| image.view.clone()),
            )),
            _ => None,
        };
        let effect_values = match kind {
            FullPass::ColorFilter(matrix) => {
                let columns =
                    std::array::from_fn(|col| std::array::from_fn(|row| matrix[row][col]));
                Some((2, self.effect_values(&columns)?))
            }
            FullPass::BackdropMaterial {
                material,
                top,
                inverse_height,
            } => Some((
                3,
                self.effect_values(&[
                    material.tint,
                    [
                        material.saturation,
                        material.brightness,
                        material.taper,
                        inverse_height,
                    ],
                    [top, 0.0, 0.0, 0.0],
                    [0.0; 4],
                    [0.0; 4],
                ])?,
            )),
            _ => None,
        };
        self.compositor.render_stats.draws += 1;
        let blend = if let FullPass::Over { blend, .. } = kind {
            blend
        } else {
            BlendKey::SrcOver
        };
        let pipeline_kind = match kind {
            FullPass::DisplayP3 if target.format.is_srgb() => PipelineKind::P3Linear,
            FullPass::DisplayP3 => PipelineKind::P3Encoded,
            FullPass::EncodeSrgb => PipelineKind::CopySrgb,
            FullPass::Copy => PipelineKind::Copy,
            FullPass::BlurDown { .. } => PipelineKind::BlurDown,
            FullPass::BlurUp { .. } => PipelineKind::BlurUp,
            FullPass::Gaussian { .. } => PipelineKind::Gaussian,
            FullPass::Over { .. } => PipelineKind::Over,
            FullPass::Multiply { .. } => PipelineKind::Multiply,
            FullPass::Backdrop => PipelineKind::Backdrop,
            FullPass::BackdropMaterial { .. } => PipelineKind::BackdropMaterial,
            FullPass::ColorFilter(_) => PipelineKind::ColorFilter,
            FullPass::MaskIntersect => PipelineKind::MaskIntersect,
        };
        let pipeline =
            self.compositor
                .pipelines
                .get(self.gpu, single(target.format), pipeline_kind, blend)?;
        let [x, y, w, h] = target.local(area);
        let kind = match kind {
            FullPass::BlurDown { backdrop, .. }
            | FullPass::BlurUp { backdrop, .. }
            | FullPass::Gaussian { backdrop, .. } => {
                self.compositor.render_stats.blur_pixels += u64::from(w) * u64::from(h);
                if backdrop {
                    GpuPassKind::BackdropBlur
                } else {
                    GpuPassKind::OffscreenEffect
                }
            }
            FullPass::Backdrop | FullPass::BackdropMaterial { .. } => GpuPassKind::BackdropBlur,
            FullPass::MaskIntersect => GpuPassKind::ClipMask,
            FullPass::Copy | FullPass::EncodeSrgb | FullPass::DisplayP3 => {
                self.compositor.render_stats.copy_pixels += u64::from(w) * u64::from(h);
                GpuPassKind::Copy
            }
            _ => GpuPassKind::OffscreenEffect,
        };
        let call = CompositeDraw {
            pipeline,
            globals,
            source: self.resolve_binding(input)?,
            composite,
            effect_values,
            immediates: immediates.then_some(params),
            viewport: [x, y, w, h],
        };
        if target.view == self.main {
            self.main_inputs.read(source.view.clone());
            if let Some(backdrop) = backdrop {
                self.main_inputs.read(backdrop.view.clone());
            }
            if let Some(mask) = &coverage.mask {
                self.main_inputs.read(mask.view.clone());
            }
            let pass = self.pending_pass(target);
            pass.draws += 1;
            pass.list.composite(call)?;
        } else {
            self.composite_pass(target, kind, call)?;
        }
        Ok(())
    }
}
