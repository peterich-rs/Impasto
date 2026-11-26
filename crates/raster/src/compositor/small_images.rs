//! Packs compatible image runs on adapters without texture binding arrays.

use std::collections::HashMap;

use etagere::{AllocId, AllocatorOptions, AtlasAllocator, size2};

use super::{Coverage, Frame, Image, ImageSource, PassBinding};
use crate::Status;
use crate::arena::Extra;
use crate::batch::{self, ImageInstance, ImageVertex};
use crate::compiler::Affine;
use crate::gpu::Gpu;
use crate::pipelines::{self, BlendKey, PipelineKind, ViewUniform};
use crate::textures::GpuImage;

const PAGE: u32 = 1024;
const LAYERS: u32 = 2;
const MAX_IMAGE: u32 = 256;
const MIP_LEVELS: u32 = 3;
const MIP_ALIGNMENT: i32 = 1 << (MIP_LEVELS - 1);
const UV_PACK_STRIDE: u32 = 2048;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

pub(super) fn supports(gpu: &Gpu) -> bool {
    gpu.max_texture_size() >= PAGE
        && gpu.caps.max_array_layers >= LAYERS
        && std::env::var("IMPASTO_SMALL_IMAGES").as_deref() != Ok("0")
}

#[derive(Default)]
pub(super) struct SmallImages {
    atlas: Option<Atlas>,
    frame: u64,
}

struct Atlas {
    backing: wgpu::Texture,
    pages: [AtlasAllocator; LAYERS as usize],
    entries: HashMap<u64, Entry>,
    local: Vec<u64>,
    groups: [wgpu::BindGroup; 2],
}

#[derive(Clone, Copy)]
struct Entry {
    layer: u32,
    origin: [u32; 2],
    levels: u32,
    allocation: AllocId,
    frame: u64,
}

impl SmallImages {
    pub(super) fn begin(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        if let Some(atlas) = &mut self.atlas {
            // A dropped encoder leaves its freshly allocated regions uninitialized.
            for revision in atlas.local.drain(..) {
                if let Some(entry) = atlas.entries.remove(&revision) {
                    atlas.pages[entry.layer as usize].deallocate(entry.allocation);
                }
            }
        }
    }

    pub(super) fn submitted(&mut self) {
        if let Some(atlas) = &mut self.atlas {
            atlas.local.clear();
        }
    }
}

impl Atlas {
    fn new(frame: &Frame<'_>) -> Self {
        use wgpu::TextureUsages as Usage;
        let gpu = frame.gpu;
        let backing = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("impasto small images"),
            size: wgpu::Extent3d {
                width: PAGE,
                height: PAGE,
                depth_or_array_layers: LAYERS,
            },
            mip_level_count: MIP_LEVELS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: Usage::COPY_DST | Usage::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = backing.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let groups = [&frame.compositor.linear, &frame.compositor.nearest].map(|sampler| {
            pipelines::bind_group(
                gpu,
                &frame.compositor.pipelines.layouts.small_images,
                &[
                    wgpu::BindingResource::TextureView(&view),
                    wgpu::BindingResource::Sampler(sampler),
                ],
            )
        });
        Self {
            backing,
            pages: std::array::from_fn(|_| {
                AtlasAllocator::with_options(
                    size2(PAGE as i32, PAGE as i32),
                    &AllocatorOptions {
                        alignment: size2(MIP_ALIGNMENT, MIP_ALIGNMENT),
                        ..Default::default()
                    },
                )
            }),
            entries: HashMap::new(),
            local: Vec::new(),
            groups,
        }
    }

    fn place(
        &mut self,
        image: &GpuImage,
        frame: u64,
        command: &mut wgpu::CommandEncoder,
    ) -> Option<Entry> {
        if let Some(entry) = self.entries.get_mut(&image.revision) {
            entry.frame = frame;
            return Some(*entry);
        }
        let wanted = size2(image.width as i32, image.height as i32);
        for _ in 0..=16 {
            if let Some((layer, allocation)) = self
                .pages
                .iter_mut()
                .zip(0..)
                .find_map(|(page, layer)| Some((layer, page.allocate(wanted)?)))
            {
                let origin = [
                    allocation.rectangle.min.x as u32,
                    allocation.rectangle.min.y as u32,
                ];
                let entry = Entry {
                    layer,
                    origin,
                    levels: image.view().texture().mip_level_count().min(MIP_LEVELS),
                    allocation: allocation.id,
                    frame,
                };
                for level in 0..entry.levels {
                    command.copy_texture_to_texture(
                        wgpu::TexelCopyTextureInfo {
                            mip_level: level,
                            ..image.view().texture().as_image_copy()
                        },
                        wgpu::TexelCopyTextureInfo {
                            mip_level: level,
                            origin: wgpu::Origin3d {
                                x: origin[0] >> level,
                                y: origin[1] >> level,
                                z: layer,
                            },
                            ..self.backing.as_image_copy()
                        },
                        super::extent(
                            [image.width, image.height].map(|side| (side >> level).max(1)),
                        ),
                    );
                }
                self.entries.insert(image.revision, entry);
                self.local.push(image.revision);
                return Some(entry);
            }
            let (&revision, _) = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.frame != frame)
                .min_by_key(|(_, entry)| entry.frame)?;
            if let Some(entry) = self.entries.remove(&revision) {
                self.pages[entry.layer as usize].deallocate(entry.allocation);
            }
        }
        None
    }
}

fn eligible(image: &GpuImage) -> bool {
    let texture = image.view().texture();
    image.width.max(image.height) <= MAX_IMAGE
        && texture.format() == FORMAT
        && texture.usage().contains(wgpu::TextureUsages::COPY_SRC)
        && !image.premultiplied_srgb
        && image.nv12().is_none()
}

// Standalone minification can need mip levels beyond the atlas page.
pub(super) fn eligible_to_batch(
    image: &GpuImage,
    instances: &[ImageInstance],
    transform: Affine,
    scale: f32,
) -> bool {
    let [a, b, c, d, ..] = transform;
    eligible(image)
        && b == 0.0
        && c == 0.0
        && instances.iter().all(|instance| {
            let [_, _, width, height] = instance.rect;
            let [u0, v0, u1, v1] = instance.uv;
            width * a.abs() * scale >= (u1 - u0) * image.width as f32
                && height * d.abs() * scale >= (v1 - v0) * image.height as f32
        })
}

impl Frame<'_> {
    pub(super) fn small_image_batch(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        blend: BlendKey,
        images: &[(&str, &[ImageInstance])],
    ) -> Result<(), Status> {
        let view = self.geometry_view(target, transform, clip, 0.0);
        let mask = self.mask_group(clip);
        let mut vertices = Vec::new();
        let mut sampler = 0;
        for (source, instances) in images {
            let Some(image) = self.scene.textures.get(source) else {
                continue;
            };
            let entry = if eligible(image) {
                if self.compositor.small_images.atlas.is_none() {
                    self.compositor.small_images.atlas = Some(Atlas::new(self));
                    self.compositor.render_stats.bind_groups_created += 2;
                }
                let cache = &mut self.compositor.small_images;
                cache
                    .atlas
                    .as_mut()
                    .and_then(|atlas| atlas.place(image, cache.frame, &mut self.command))
            } else {
                None
            };
            let Some(entry) = entry else {
                self.flush_small_images(target, clip, blend, view, &mask, sampler, &mut vertices)?;
                self.images(
                    target,
                    transform,
                    clip,
                    *instances,
                    ImageSource::External(image),
                    blend,
                )?;
                continue;
            };
            for instance in *instances {
                let next = usize::from(instance.flags & batch::IMAGE_NEAREST != 0);
                if next != sampler {
                    self.flush_small_images(
                        target,
                        clip,
                        blend,
                        view,
                        &mask,
                        sampler,
                        &mut vertices,
                    )?;
                    sampler = next;
                }
                let pack = |[x, y]: [u32; 2]| x + y * UV_PACK_STRIDE;
                let mut vertex = instance.vertex(false);
                // The lower eight bits hold the flags and mip count.
                vertex.meta[0] = instance.flags | ((entry.levels - 1) << 3) | (entry.layer << 8);
                vertex.meta[2] = pack(entry.origin);
                vertex.meta[3] = pack([image.width, image.height]);
                vertices.push(vertex.with_opacity_proof(image.opaque));
            }
        }
        self.flush_small_images(target, clip, blend, view, &mask, sampler, &mut vertices)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "batch owns the shared transform, clip and sampler"
    )]
    fn flush_small_images(
        &mut self,
        target: &Image,
        clip: &Coverage,
        blend: BlendKey,
        view: ViewUniform,
        mask: &PassBinding,
        sampler: usize,
        vertices: &mut Vec<ImageVertex>,
    ) -> Result<(), Status> {
        if vertices.is_empty() {
            return Ok(());
        }
        let atlas = self
            .compositor
            .small_images
            .atlas
            .as_ref()
            .ok_or(Status::BadScene)?;
        let material = atlas.groups[sampler].clone();
        self.record_instances(
            target,
            view,
            clip.scissor,
            PipelineKind::SmallImages,
            blend,
            Extra::None,
            material,
            Some(mask.clone()),
            vertices,
            6,
        )?;
        vertices.clear();
        Ok(())
    }
}
