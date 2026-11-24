//! Records and replays persistent draw bundles for compatible picture runs.

use bytemuck::Zeroable;

use super::{
    Coverage, DrawCall, Frame, GlyphTransform, Image, PictureReplay, draw_count, shadow_pipeline,
    single,
};
use crate::{
    Status,
    arena::FrameArena,
    atlas::{AtlasBinding, Placement},
    batch::{self, GlyphVertex, PaintExtra, RectVertex},
    compiler::{Affine, Draw},
    pipelines::{self, BlendKey, PipelineKind},
    resident::{Key, LayerSlot, VertexRange},
};

impl Frame<'_> {
    pub(super) fn try_resident_scope(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        draws: &[Draw],
        replay: &PictureReplay<'_>,
        position: (usize, usize),
    ) -> Result<bool, Status> {
        let (kind, blend, len, revision, paint_bytes) = match draws {
            [Draw::Rects(run)] => {
                let solid = run.paints.is_empty() && self.compositor.specialize_paint;
                let paint_bytes = if solid {
                    0
                } else {
                    (pipelines::MAX_STOPS * 32 + run.paints.len().max(1) * size_of::<PaintExtra>())
                        as u64
                };
                (
                    if solid {
                        PipelineKind::RectSolid
                    } else {
                        PipelineKind::Rect
                    },
                    run.blend,
                    run.vertices.len(),
                    0,
                    paint_bytes,
                )
            }
            [
                Draw::Images {
                    source,
                    instances,
                    blend,
                },
            ] => {
                let Some(image) = self.scene.textures.get(source) else {
                    return Ok(false);
                };
                (
                    PipelineKind::Image,
                    *blend,
                    instances.len(),
                    image.revision,
                    0,
                )
            }
            [Draw::Glyphs(glyphs)] if self.compositor.specialize_paint && !self.gpu.caps.lcd => (
                PipelineKind::TextMask,
                BlendKey::SrcOver,
                glyphs.len(),
                0,
                0,
            ),
            _ => return Ok(false),
        };
        if len == 0
            || len > FrameArena::vertex_limit::<RectVertex>(self.gpu)
            || paint_bytes > self.gpu.caps.max_storage_binding
        {
            return Ok(false);
        }
        let dither = self.gpu.dither_gradients
            && target.view == self.root
            && target.format.is_srgb()
            && blend == BlendKey::SrcOver;
        let is_text = kind == PipelineKind::TextMask;
        let atlas_scale = self.compositor.atlas.inverse_page_size();
        let [a, b, c, d, ..] = transform;
        let scale = self.scene.scale;
        let glyph_geometry = is_text.then(|| GlyphTransform::new(transform, scale));
        let mut key = Key {
            picture: replay.picture.resident_id,
            layer: replay.layer,
            scope: position.0,
            draw: position.1,
            format: target.format,
            revision,
            atlas_epoch: if is_text {
                self.compositor.atlas.epoch()
            } else {
                0
            },
            glyph_transform: glyph_geometry.map_or([0; 6], |geometry| {
                [
                    a.to_bits(),
                    b.to_bits(),
                    c.to_bits(),
                    d.to_bits(),
                    scale.to_bits(),
                    geometry.phase_x,
                ]
            }),
        };
        let compositor = &mut self.compositor;
        let hit = compositor.resident.get(key, &mut compositor.atlas);
        if hit.is_none() && !compositor.resident.observe_miss(key) {
            return Ok(false);
        }
        let state = self.geometry_view(
            target,
            glyph_geometry.map_or(transform, |geometry| geometry.matrix),
            clip,
            if is_text {
                atlas_scale
            } else {
                f32::from(dither)
            },
        );
        let bundle = if let Some((slot, bundle)) = hit {
            self.compositor.resident.update(slot, &state);
            bundle
        } else {
            self.compositor.resident.check_recording_device(self.gpu)?;
            let mut glyph_pages = Vec::<Placement>::new();
            let mut glyph_vertices = Vec::new();
            if let ([Draw::Glyphs(glyphs)], Some(geometry)) = (draws, glyph_geometry) {
                if glyphs.iter().any(|glyph| geometry.vector(glyph.size)) {
                    return Ok(false);
                }
                for glyph in glyphs {
                    let Some((vertex, binding, placement)) =
                        self.glyph_quad(glyph, geometry, false)?
                    else {
                        continue;
                    };
                    if placement.color || binding != AtlasBinding::Resident {
                        return Ok(false);
                    }
                    if glyph_pages
                        .iter()
                        .all(|page| page.rect.page != placement.rect.page)
                    {
                        glyph_pages.push(placement);
                    }
                    glyph_vertices.push(vertex);
                }
                if glyph_vertices.is_empty() {
                    return Ok(true);
                }
                key.atlas_epoch = self.compositor.atlas.epoch();
            }
            let (len, vertex_bytes) = if is_text {
                (
                    glyph_vertices.len(),
                    (glyph_vertices.len() * size_of::<GlyphVertex>()) as u64,
                )
            } else {
                (len, (len * size_of::<RectVertex>()) as u64)
            };
            let bytes = vertex_bytes + paint_bytes;
            let view_layout = &self.compositor.pipelines.layouts.view;
            let Some((slot, created)) =
                self.compositor
                    .resident
                    .allocate(self.gpu, view_layout, &state, bytes)?
            else {
                return Ok(false);
            };
            let image_vertices;
            let contents = match draws {
                [Draw::Rects(run)] => bytemuck::cast_slice(&run.vertices),
                [Draw::Glyphs(_)] => bytemuck::cast_slice(&glyph_vertices),
                [
                    Draw::Images {
                        source, instances, ..
                    },
                ] => {
                    let Some(image) = self.scene.textures.get(source) else {
                        self.compositor.resident.abandon(slot);
                        return Ok(false);
                    };
                    image_vertices = instances
                        .iter()
                        .map(|instance| instance.vertex(image.premultiplied_srgb))
                        .collect::<Vec<_>>();
                    bytemuck::cast_slice(&image_vertices)
                }
                _ => {
                    self.compositor.resident.abandon(slot);
                    return Ok(false);
                }
            };
            let Some((vertices, vertex_page)) =
                self.compositor.resident.vertices(self.gpu, contents)
            else {
                self.compositor.resident.abandon(slot);
                return Ok(false);
            };
            match self.build_resident_bundle(target, draws, kind, blend, &slot, len, &vertices) {
                Ok(bundle) => {
                    self.compositor.render_stats.buffers_created +=
                        u32::from(vertex_page) + u32::from(created);
                    self.compositor.render_stats.bind_groups_created += u32::from(created);
                    self.compositor.render_stats.vertex_bytes += vertex_bytes;
                    self.compositor.resident.insert(
                        key,
                        slot,
                        bundle.clone(),
                        bytes,
                        vertices,
                        glyph_pages,
                    );
                    bundle
                }
                Err(error) => {
                    self.compositor.resident.abandon(slot);
                    self.compositor.resident.release_vertices(vertices);
                    return Err(error);
                }
            }
        };
        self.track_opaque(target, transform, clip, &draws[0]);
        let count = match &draws[0] {
            Draw::Rects(run) => run.draws,
            _ => 1,
        };
        self.compositor.render_stats.draws += count;
        let pass = self.pending_pass(target);
        pass.draws += count;
        pass.calls.push(DrawCall::Resident {
            bundle,
            scissor: clip.scissor.map(|area| target.local(area)),
        });
        Ok(true)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "bundle creation receives an immutable scope and its target state"
    )]
    fn build_resident_bundle(
        &mut self,
        target: &Image,
        draws: &[Draw],
        kind: PipelineKind,
        blend: BlendKey,
        slot: &LayerSlot,
        len: usize,
        vertices: &VertexRange,
    ) -> Result<wgpu::RenderBundle, Status> {
        let gpu = self.gpu;
        let format = target.format;
        let compositor = &mut self.compositor;
        let target = single(format);
        let shadow_kind = match draws {
            [Draw::Rects(run)]
                if run.draws > 1 || run.vertices.first().is_some_and(batch::is_shadow) =>
            {
                Some(shadow_pipeline(&run.vertices))
            }
            _ => None,
        };
        let shadow_only =
            shadow_kind.is_some() && matches!(draws, [Draw::Rects(run)] if run.draws == 1);
        let pipeline = compositor.pipelines.get(
            gpu,
            target,
            if shadow_only {
                shadow_kind.unwrap_or(kind)
            } else {
                kind
            },
            blend,
        )?;
        let shadow = shadow_kind
            .filter(|_| !shadow_only)
            .map(|kind| compositor.pipelines.get(gpu, target, kind, blend))
            .transpose()?;
        let mut encoder =
            gpu.device
                .create_render_bundle_encoder(&wgpu::RenderBundleEncoderDescriptor {
                    label: Some("impasto picture bundle"),
                    color_formats: &[Some(format)],
                    sample_count: 1,
                    ..Default::default()
                });
        encoder.set_vertex_buffer(0, vertices.slice());
        match draws {
            [Draw::Rects(run)] => {
                let (group, offsets, count) = if kind == PipelineKind::RectSolid {
                    (slot.group.clone(), [slot.offset, 0], 1)
                } else {
                    let layout = &compositor.pipelines.layouts.paint;
                    let paints = if run.paints.is_empty() {
                        &[PaintExtra::zeroed()][..]
                    } else {
                        &run.paints
                    };
                    let (group, [stop_bytes, extra_bytes]) = compositor.resident.paint_group(
                        gpu,
                        layout,
                        &slot.buffer,
                        &compositor.arena.lut.view,
                        run.stops(),
                        paints,
                    )?;
                    if extra_bytes != 0 {
                        compositor.render_stats.buffers_created += 2;
                        compositor.render_stats.bind_groups_created += 1;
                        compositor.render_stats.stop_bytes += stop_bytes;
                        compositor.render_stats.extra_bytes += extra_bytes;
                    }
                    (group, [slot.offset, 0], 2)
                };
                // Each contiguous range keeps its original painter order. Both
                // rectangle pipelines bind the same view and immutable paints.
                encoder.set_bind_group(0, &group, &offsets[..count]);
                encoder.set_bind_group(1, &compositor.white_mask_group, &[]);
                let mut start = 0;
                for vertices in batch::rect_pipeline_runs(&run.vertices) {
                    let pipeline = if batch::is_shadow(&vertices[0]) {
                        shadow.as_ref().unwrap_or(&pipeline)
                    } else {
                        &pipeline
                    };
                    let end = start + draw_count(vertices.len())?;
                    encoder.set_pipeline(pipeline);
                    encoder.draw(0..6, start..end);
                    start = end;
                }
            }
            [
                Draw::Images {
                    source, instances, ..
                },
            ] => {
                encoder.set_pipeline(&pipeline);
                let image = self.scene.textures.get(source).ok_or(Status::BadScene)?;
                let nearest = instances[0].flags & batch::IMAGE_NEAREST != 0;
                let sampler = if nearest {
                    &compositor.nearest
                } else {
                    &compositor.linear
                };
                let (group, created) = image.material.get(
                    gpu,
                    &compositor.pipelines.layouts.material,
                    image.view(),
                    sampler,
                    nearest,
                );
                compositor.render_stats.bind_groups_created += u32::from(created);
                encoder.set_bind_group(0, &slot.group, &[slot.offset]);
                encoder.set_bind_group(1, &group, &[]);
                encoder.set_bind_group(2, &compositor.white_mask_group, &[]);
                encoder.draw(0..6, 0..draw_count(len)?);
            }
            [Draw::Glyphs(_)] => {
                encoder.set_pipeline(&pipeline);
                let (group, created) = compositor.atlas.material(
                    gpu,
                    &compositor.pipelines.layouts.atlas,
                    &compositor.linear,
                    AtlasBinding::Resident,
                );
                compositor.render_stats.bind_groups_created += u32::from(created);
                encoder.set_bind_group(0, &slot.group, &[slot.offset]);
                encoder.set_bind_group(1, &group, &[]);
                encoder.set_bind_group(2, &compositor.white_mask_group, &[]);
                encoder.draw(0..6, 0..draw_count(len)?);
            }
            _ => return Err(Status::BadScene),
        }
        Ok(encoder.finish(&wgpu::RenderBundleDescriptor {
            label: Some("impasto picture"),
        }))
    }
}
