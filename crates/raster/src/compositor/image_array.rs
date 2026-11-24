//! Batches ordered image runs with one texture binding array per sampler.

use super::{Coverage, Frame, Image, single};
use crate::{
    Status,
    arena::{DynamicGroup, Extra},
    batch::{self, ImageInstance},
    compiler::Affine,
    gpu::IMAGE_ARRAY_LEN,
    pipelines::{self, BlendKey, PipelineKind},
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
        let view = self.transform(target, transform, clip)?;
        let globals = self.globals(view, Extra::None);
        let mask = self.mask_group(clip);
        let pipeline = self.compositor.pipelines.get(
            self.gpu,
            single(target.format),
            PipelineKind::ImageArray,
            blend,
        )?;
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
                        &pipeline,
                        &globals,
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
                let mut vertex = instance.vertex(image.premultiplied_srgb);
                vertex.meta[1] = index as f32;
                vertices.push(vertex);
            }
        }
        self.image_array_batch(
            target,
            clip.scissor,
            &pipeline,
            &globals,
            &mask,
            &views,
            nearest,
            &vertices,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "a draw batch passes its shared pipeline and resources"
    )]
    fn image_array_batch(
        &mut self,
        target: &Image,
        scissor: Option<[u32; 4]>,
        pipeline: &wgpu::RenderPipeline,
        globals: &DynamicGroup,
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
        self.record(
            target,
            pipeline.clone(),
            globals.clone(),
            material,
            Some(mask.clone()),
            vertices,
            scissor,
        )
    }
}
