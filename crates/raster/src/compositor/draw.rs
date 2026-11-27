//! Ordered draw dispatch and adjacent batches.

use std::{iter::Peekable, slice::Iter};

use engine::scene::{CanvasComposite, LayerId};

use super::{Coverage, Frame, Image, ImageSource};
use crate::Status;
use crate::arena::{Extra, FrameArena, PaintPage};
use crate::batch::{self, ImageInstance, Mesh, RectInstance, RectVertex};
use crate::compiler::{self, Affine, Draw, IDENTITY, RectRun};
use crate::pipelines::{BlendKey, PipelineKind, ViewUniform};

type Draws<'a> = Peekable<Iter<'a, Draw>>;

impl Frame<'_> {
    pub(super) fn draws(
        &mut self,
        layer: LayerId,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        draws: &[Draw],
    ) -> Result<(), Status> {
        let mut draws = draws.iter().peekable();
        while let Some(draw) = draws.next() {
            match draw {
                Draw::Rects(run) => self.draw_rect_runs(target, transform, clip, run, &mut draws),
                Draw::Fill(color) => self.draw_fill(target, clip, *color),
                Draw::Glyphs(glyphs) => {
                    self.draw_glyph_runs(target, transform, clip, glyphs, &mut draws)
                }
                Draw::Images {
                    source,
                    instances,
                    blend,
                } => self.draw_image_runs(
                    target,
                    transform,
                    clip,
                    (source, instances),
                    *blend,
                    &mut draws,
                ),
                Draw::ShaderEffect {
                    bounds,
                    shader,
                    uniforms,
                    textures,
                } => self.shader_effect(
                    target,
                    transform,
                    clip,
                    *bounds,
                    *shader,
                    uniforms,
                    textures,
                    self.scene.shader_uniforms.get(&layer),
                ),
                Draw::Mesh(mesh) => self.draw_mesh_runs(target, transform, clip, mesh, &mut draws),
            }?;
        }
        Ok(())
    }

    fn draw_glyph_runs(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        glyphs: &[compiler::GlyphDraw],
        draws: &mut Draws<'_>,
    ) -> Result<(), Status> {
        let next = std::iter::from_fn(|| {
            let Draw::Glyphs(glyphs) = *draws.peek()? else {
                return None;
            };
            draws.next();
            Some(glyphs.as_slice())
        });
        self.glyphs(target, transform, clip, glyphs.iter().chain(next.flatten()))
    }

    fn draw_rect_runs(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        run: &RectRun,
        draws: &mut Draws<'_>,
    ) -> Result<(), Status> {
        let blend = run.blend;
        let mut first = Some(run);
        let runs = std::iter::from_fn(|| {
            if first.is_some() {
                return first.take();
            }
            let Draw::Rects(next) = *draws.peek()? else {
                return None;
            };
            if next.blend != blend {
                return None;
            }
            draws.next();
            Some(next)
        });
        self.prepared_rects(target, transform, clip, blend, runs)
    }

    fn draw_fill(
        &mut self,
        target: &Image,
        clip: &Coverage,
        color: [f32; 4],
    ) -> Result<(), Status> {
        if clip.mask.is_none()
            && clip.scissor.is_none()
            && clip.count == 0
            && if target.view == self.main {
                self.main_pass.is_none()
            } else {
                self.pending.is_none()
            }
            && let Some(clear) = self.clears.get_mut(&target.view)
            && (clear.a == 0.0 || color[3] == 1.0)
        {
            *clear = wgpu::Color {
                r: color[0].into(),
                g: color[1].into(),
                b: color[2].into(),
                a: color[3].into(),
            };
            return Ok(());
        }
        let scale = self.scene.scale;
        let rect = compiler::solid_rect(
            [
                0.0,
                0.0,
                self.size[0] as f32 / scale,
                self.size[1] as f32 / scale,
            ],
            color,
        );
        self.rects(target, IDENTITY, clip, &[rect], &[])
    }

    fn draw_image_runs(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        (source, instances): (&str, &[ImageInstance]),
        blend: BlendKey,
        draws: &mut Draws<'_>,
    ) -> Result<(), Status> {
        let next = std::iter::from_fn(|| {
            if let Some(Draw::Images {
                source: next_source,
                instances,
                blend: next_blend,
            }) = draws.peek()
                && next_source.as_ref() == source
                && *next_blend == blend
            {
                let instances = instances.as_slice();
                draws.next();
                Some(instances)
            } else {
                None
            }
        });
        if let Some(image) = self.scene.textures.get(source) {
            self.images(
                target,
                transform,
                clip,
                instances.iter().chain(next.flatten()),
                ImageSource::External(image),
                blend,
            )?;
        }
        Ok(())
    }

    fn draw_mesh_runs(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        mesh: &Mesh,
        draws: &mut Draws<'_>,
    ) -> Result<(), Status> {
        let mut meshes = vec![mesh];
        if mesh.composite == CanvasComposite::SourceOver {
            while let Some(Draw::Mesh(next)) = draws.peek() {
                if next.composite != mesh.composite {
                    break;
                }
                meshes.push(next);
                draws.next();
            }
        }
        self.meshes(target, transform, clip, &meshes)
    }

    pub(super) fn rects(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        rects: &[RectInstance],
        stops: &[[f32; 4]],
    ) -> Result<(), Status> {
        self.rect_batch_from_paint(target, transform, clip, BlendKey::SrcOver, (rects, stops))
    }

    fn prepared_rects<'a>(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        blend: BlendKey,
        runs: impl IntoIterator<Item = &'a RectRun>,
    ) -> Result<(), Status> {
        let view = self.rect_view(target, transform, clip, blend);
        let mut batch = Vec::new();
        let mut active: Option<PaintPage> = None;
        let mut addresses = Vec::new();
        for run in runs {
            let stops = self.stops(run.stops())?;
            let empty = self.paint(stops, None)?;
            addresses.clear();
            addresses.resize(run.paints.len(), None);
            for &vertex in &run.vertices {
                let (page, index) = if vertex.paint == u32::MAX {
                    if active.is_some_and(|page| page.same_bindings(empty.0)) {
                        (active.unwrap_or(empty.0), u32::MAX)
                    } else {
                        empty
                    }
                } else {
                    let offset = vertex.paint as usize;
                    if let Some(address) = addresses[offset] {
                        address
                    } else {
                        let mut paint = run.paints[offset];
                        paint.gradient[1] = FrameArena::stop_index(stops);
                        let address = if self.gpu.caps.paint_storage {
                            self.compositor
                                .arena
                                .compiled_paint(self.gpu, stops, paint)?
                        } else {
                            self.paint(stops, Some(paint))?
                        };
                        addresses[offset] = Some(address);
                        address
                    }
                };
                if let Some(previous) = active
                    && !page.same_bindings(previous)
                {
                    self.rect_batch(target, clip, view, previous, &batch, blend)?;
                    batch.clear();
                }
                active = Some(page);
                batch.push(RectVertex {
                    paint: index,
                    ..vertex
                });
            }
        }
        if let Some(page) = active {
            self.rect_batch(target, clip, view, page, &batch, blend)?;
        }
        Ok(())
    }

    fn rect_view(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        blend: BlendKey,
    ) -> ViewUniform {
        let dither =
            target.view == self.root && target.format.is_srgb() && blend == BlendKey::SrcOver;
        self.geometry_view(target, transform, clip, f32::from(dither))
    }

    pub(super) fn rect_batch_from_paint(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        blend: BlendKey,
        (rects, stops): (&[RectInstance], &[[f32; 4]]),
    ) -> Result<(), Status> {
        let view = self.rect_view(target, transform, clip, blend);
        let mut batch = Vec::new();
        let mut active = None;
        let stops = self.stops(stops)?;
        for rect in rects {
            let (page, index) = self.paint(stops, rect.extra(FrameArena::stop_index(stops)))?;
            if let Some(previous) = active
                && !page.same_bindings(previous)
            {
                self.rect_batch(target, clip, view, previous, &batch, blend)?;
                batch.clear();
            }
            active = Some(page);
            batch.push(rect.vertex(index));
        }
        if let Some(page) = active {
            self.rect_batch(target, clip, view, page, &batch, blend)?;
        }
        Ok(())
    }

    fn rect_batch(
        &mut self,
        target: &Image,
        clip: &Coverage,
        view: ViewUniform,
        page: PaintPage,
        vertices: &[RectVertex],
        blend: BlendKey,
    ) -> Result<(), Status> {
        for run in batch::rect_pipeline_runs(vertices) {
            let solid = self.compositor.specialize_paint
                && run
                    .iter()
                    .all(|rect| rect.shape[2] == 0.0 && rect.paint == u32::MAX);
            let kind = if batch::is_shadow(&run[0]) {
                shadow_pipeline(run)
            } else {
                self.rect_pipeline(target, clip, run, blend, solid)
            };
            self.read_mask(target, clip);
            let mask = self.mask_group(clip);
            let vertex_count = if batch::is_shadow(&run[0]) { 24 } else { 6 };
            self.record_instances(
                target,
                view,
                clip.scissor,
                kind,
                blend,
                if solid {
                    Extra::None
                } else {
                    Extra::Paint(page)
                },
                mask,
                None,
                run,
                vertex_count,
            )?;
        }
        Ok(())
    }

    pub(super) fn rect_pipeline(
        &self,
        target: &Image,
        clip: &Coverage,
        vertices: &[RectVertex],
        blend: BlendKey,
        solid: bool,
    ) -> PipelineKind {
        let unclipped = clip.count == 0 && clip.mask.is_none();
        if solid {
            return if unclipped {
                PipelineKind::RectSolidUnclipped
            } else {
                PipelineKind::RectSolid
            };
        }
        let dither = target.view == self.root
            && target.format.is_srgb()
            && blend == BlendKey::SrcOver
            && vertices.iter().any(batch::has_gradient);
        match (unclipped, dither) {
            (false, false) => PipelineKind::Rect,
            (true, false) => PipelineKind::RectUnclipped,
            (false, true) => PipelineKind::RectDither,
            (true, true) => PipelineKind::RectDitherUnclipped,
        }
    }

    pub(super) fn images<'a>(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        images: impl IntoIterator<Item = &'a ImageInstance>,
        image: ImageSource<'_>,
        blend: BlendKey,
    ) -> Result<(), Status> {
        let view = self.geometry_view(target, transform, clip, 0.0);
        let mask = self.mask_group(clip);
        let kind = if image.nv12().is_some() {
            PipelineKind::ExternalImage
        } else if clip.count == 0 && clip.mask.is_none() {
            PipelineKind::ImageUnclipped
        } else {
            PipelineKind::Image
        };
        let mut vertices = Vec::new();
        let mut images = images.into_iter().peekable();
        while let Some(first) = images.next() {
            let nearest = first.flags & batch::IMAGE_NEAREST;
            vertices.push(first.vertex(image.premultiplied_srgb()));
            while let Some(next) =
                images.next_if(|next| next.flags & batch::IMAGE_NEAREST == nearest)
            {
                vertices.push(next.vertex(image.premultiplied_srgb()));
            }
            if target.view == self.main
                && let ImageSource::Pooled(input) = image
            {
                self.main_inputs.read(input.view.clone());
            }
            self.read_mask(target, clip);
            let material = image.material(self, nearest != 0);
            self.record_instances(
                target,
                view,
                clip.scissor,
                kind,
                blend,
                Extra::None,
                material,
                Some(mask.clone()),
                &vertices,
                6,
            )?;
            vertices.clear();
        }
        Ok(())
    }
}

pub(super) fn shadow_pipeline(rects: &[RectVertex]) -> PipelineKind {
    if batch::has_shaped_shadow(rects) {
        PipelineKind::RectShadow
    } else {
        PipelineKind::RectCircularShadow
    }
}
