//! Batches ordered image runs with one texture binding array per sampler.

use std::collections::HashMap;

use super::{Coverage, Frame, Image, draw_count};
use crate::{
    Status,
    arena::Extra,
    batch::{self, ImageInstance},
    compiler::Affine,
    gpu::IMAGE_ARRAY_LEN,
    pipelines::{self, BlendKey, PipelineKind, ViewUniform},
};

impl Frame<'_> {
    pub(super) fn image_array(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        blend: BlendKey,
        images: &[(&str, &[ImageInstance])],
    ) -> Result<(), Status> {
        let view = self.geometry_view(target, transform, clip, 0.0);
        self.read_mask(target, clip);
        let mask = self.mask_group(clip);
        let mut views = Vec::new();
        let mut vertices = Vec::new();
        let mut nearest = false;
        let limit = IMAGE_ARRAY_LEN as usize;
        for (source, instances) in images {
            let Some(image) = self.scene.textures.get(source) else {
                continue;
            };
            for instance in *instances {
                let next_nearest = instance.flags & batch::IMAGE_NEAREST != 0;
                let position = views.iter().position(|view| view == image.view());
                if nearest != next_nearest || (position.is_none() && views.len() == limit) {
                    self.image_array_batch(
                        target,
                        clip.scissor,
                        view,
                        blend,
                        &mask,
                        &views,
                        nearest,
                        &vertices,
                    )?;
                    views.clear();
                    vertices.clear();
                    nearest = next_nearest;
                }
                let index = views
                    .iter()
                    .position(|view| view == image.view())
                    .unwrap_or_else(|| {
                        let index = views.len();
                        views.push(image.view().clone());
                        index
                    });
                let mut vertex = instance
                    .vertex(image.premultiplied_srgb)
                    .with_opacity_proof(image.opaque);
                vertex.meta[0] |= draw_count(index)? << 8;
                vertices.push(vertex);
            }
        }
        self.image_array_batch(
            target,
            clip.scissor,
            view,
            blend,
            &mask,
            &views,
            nearest,
            &vertices,
        )
    }

    pub(super) fn frame_image_array(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        blend: BlendKey,
        source: &str,
        instances: &[ImageInstance],
    ) -> Result<bool, Status> {
        let images = self.frame_images.get_or_insert_with(|| {
            let mut sources: Vec<_> = self
                .scene
                .textures
                .active_images()
                .filter(|(source, image)| cacheable(source) && image.nv12().is_none())
                .collect();
            sources.sort_unstable_by_key(|&(key, _)| key);
            let mut images = FrameImages::default();
            for chunk in sources.chunks(IMAGE_ARRAY_LEN as usize) {
                let page = images.pages.len();
                images.pages.push(
                    chunk
                        .iter()
                        .enumerate()
                        .map(|(slot, &(key, image))| {
                            images.slots.insert(key, (page, slot));
                            image.view().clone()
                        })
                        .collect(),
                );
            }
            images
        });
        let Some(&(page, slot)) = images.slots.get(source) else {
            return Ok(false);
        };
        let views = images.pages[page].clone();
        let slot = draw_count(slot)?;
        let Some(image) = self.scene.textures.get(source) else {
            return Ok(false);
        };
        let view = self.geometry_view(target, transform, clip, 0.0);
        self.read_mask(target, clip);
        let mask = self.mask_group(clip);
        for run in instances
            .chunk_by(|a, b| a.flags & batch::IMAGE_NEAREST == b.flags & batch::IMAGE_NEAREST)
        {
            let vertices: Vec<_> = run
                .iter()
                .map(|image_instance| {
                    let mut vertex = image_instance
                        .vertex(image.premultiplied_srgb)
                        .with_opacity_proof(image.opaque);
                    vertex.meta[0] |= slot << 8;
                    vertex
                })
                .collect();
            self.image_array_batch(
                target,
                clip.scissor,
                view,
                blend,
                &mask,
                &views,
                run[0].flags & batch::IMAGE_NEAREST != 0,
                &vertices,
            )?;
        }
        Ok(true)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "a draw batch passes its shared pipeline and resources"
    )]
    fn image_array_batch(
        &mut self,
        target: &Image,
        scissor: Option<[u32; 4]>,
        view: ViewUniform,
        blend: BlendKey,
        mask: &wgpu::BindGroup,
        views: &[wgpu::TextureView],
        nearest: bool,
        vertices: &[batch::ImageVertex],
    ) -> Result<(), Status> {
        let Some(first) = views.first() else {
            return Ok(());
        };
        let compositor = &mut self.compositor;
        let key = (views.to_vec(), nearest);
        if compositor.image_groups.len() >= 64
            && !compositor.image_groups.contains_key(&key)
            && let Some(old) = compositor.image_groups.keys().next().cloned()
        {
            compositor.image_groups.remove(&old);
        }
        let layout = compositor
            .pipelines
            .layouts
            .image_array
            .as_ref()
            .ok_or(Status::NotReady)?;
        let material = compositor
            .image_groups
            .entry(key)
            .or_insert_with(|| {
                let mut padded = [first; IMAGE_ARRAY_LEN as usize];
                for (slot, view) in padded.iter_mut().zip(views) {
                    *slot = view;
                }
                let sampler = if nearest {
                    &compositor.nearest
                } else {
                    &compositor.linear
                };
                compositor.render_stats.bind_groups_created += 1;
                pipelines::bind_group(
                    self.gpu,
                    layout,
                    &[
                        wgpu::BindingResource::TextureViewArray(&padded),
                        wgpu::BindingResource::Sampler(sampler),
                    ],
                )
            })
            .clone();
        self.record_instances(
            target,
            view,
            scissor,
            PipelineKind::ImageArray,
            blend,
            Extra::None,
            material,
            Some(mask.clone()),
            vertices,
            6,
        )
    }
}

#[derive(Default)]
pub(super) struct FrameImages<'a> {
    slots: HashMap<&'a str, (usize, usize)>,
    pages: Vec<Vec<wgpu::TextureView>>,
}

pub(super) fn cacheable(source: &str) -> bool {
    !source.starts_with(engine::scene::image::EXTERNAL_TEXTURE_PREFIX)
        && !source.starts_with(engine::scene::image::GPU_VIEW_TEXTURE_PREFIX)
}
