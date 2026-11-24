//! Holds single-device texture snapshots for compositor-owned moving groups.

use std::collections::HashMap;

use engine::scene::{LayerId, LayerKind, image::EXTERNAL_TEXTURE_PREFIX};

use super::{Clip, Frame, Image, ImageSource, Status, translation};
use crate::batch::ImageInstance;
use crate::compiler::{Affine, IDENTITY, PictureBounds};
use crate::pipelines::BlendKey;

const CACHE_BUDGET: u64 = 24 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq)]
pub(super) struct Key {
    pub revision: u64,
    pub size: [u32; 2],
    pub scale: f32,
    pub format: wgpu::TextureFormat,
}

#[derive(Default)]
pub(super) struct TransitionSnapshots {
    key: Option<Key>,
    images: HashMap<LayerId, Snapshot>,
    baking: Vec<LayerId>,
}

#[derive(Clone)]
struct Snapshot {
    image: Image,
    rect: [f32; 4],
}

impl TransitionSnapshots {
    pub(super) fn begin(&mut self) {
        for id in self.baking.drain(..) {
            self.images.remove(&id);
        }
    }

    pub(super) fn submitted(&mut self) {
        self.baking.clear();
    }

    pub(super) fn prepare(&mut self, key: Key) {
        if self.key != Some(key) {
            self.images.clear();
            self.key = Some(key);
        }
    }
}

impl Frame<'_> {
    pub(super) fn try_transition_snapshot(
        &mut self,
        id: LayerId,
        children: Option<LayerId>,
        target: &Image,
        transform: Affine,
        clip: &Clip,
        opacity: f32,
    ) -> Result<bool, Status> {
        let mut tracks = self
            .scene
            .tree
            .tracks
            .iter()
            .filter(|track| track.layer == id);
        let Some(first) = tracks.next() else {
            return Ok(false);
        };
        let requested = first.freeze_subtree || tracks.any(|track| track.freeze_subtree);
        if self.scene.slice.is_some()
            || self.baking_subtree
            || !clip.shapes.is_empty()
            || (self.gpu.caps.lcd && self.scene.has_glyphs)
            || !requested
                && (target.view != self.main
                    || self.damage_scissor.is_some()
                    || clip.scissor.is_some())
        {
            return Ok(false);
        }
        let key = Key {
            revision: self.scene.scene_revision,
            size: self.size,
            scale: self.scene.scale,
            format: self.working,
        };
        let existing = self
            .compositor
            .transition_snapshots
            .images
            .get(&id)
            .cloned();
        let snapshot = if let Some(snapshot) = existing {
            self.compositor.render_stats.transition_snapshot_hits += 1;
            snapshot
        } else {
            if !self.can_freeze(children) {
                return Ok(false);
            }
            let bounds = self.layer_bounds(children, IDENTITY)?;
            let PictureBounds::Rect([x, y, w, h]) = bounds else {
                return Ok(false);
            };
            let [width, height] = self.size.map(|pixels| pixels as f32 / key.scale);
            if !requested
                && (x < 0.0
                    || y < 0.0
                    || x + w > width
                    || y + h > height
                    || w * h < width * height / 8.0)
            {
                return Ok(false);
            }
            // Snap in device pixels and use a local origin so shadows and
            // off-window content stay available during the whole transition.
            let left = (x * key.scale).floor() - 1.0;
            let top = (y * key.scale).floor() - 1.0;
            let span = [
                ((x + w) * key.scale).ceil() + 1.0 - left,
                ((y + h) * key.scale).ceil() + 1.0 - top,
            ];
            let limit = self.gpu.caps.max_texture_2d;
            if span.into_iter().any(|side| side > limit as f32) {
                return Ok(false);
            }
            let Some([_, _, w, h]) = super::scissor([0.0, 0.0, span[0], span[1]], 1.0, [limit; 2])
            else {
                return Ok(false);
            };
            let bytes_per_pixel = if key.format == wgpu::TextureFormat::Rgba16Float {
                8
            } else {
                4
            };
            let bytes = u64::from(w) * u64::from(h) * bytes_per_pixel;
            let used: u64 = self
                .compositor
                .transition_snapshots
                .images
                .values()
                .map(|snapshot| snapshot.image.bytes())
                .sum();
            if used + bytes > CACHE_BUDGET {
                return Ok(false);
            }
            let rect = [left, top, w as f32, h as f32].map(|value| value / key.scale);
            let image = Image::new(self.gpu, [w, h], key.format, 1);
            self.clear(&image.view);
            self.baking_subtree = true;
            let frame_size = std::mem::replace(&mut self.size, [w, h]);
            let baked = self.layers(
                children,
                &image,
                translation(-rect[0], -rect[1]),
                &Clip::default(),
            );
            self.size = frame_size;
            self.baking_subtree = false;
            baked?;
            self.finish(&image.view);
            let snapshot = Snapshot { image, rect };
            let cache = &mut self.compositor.transition_snapshots;
            cache.images.insert(id, snapshot.clone());
            cache.baking.push(id);
            self.compositor.render_stats.transition_snapshot_bakes += 1;
            snapshot
        };
        let coverage = self.coverage(target, clip)?;
        if coverage.empty {
            return Ok(true);
        }
        self.images(
            target,
            transform,
            &coverage,
            &[ImageInstance {
                rect: snapshot.rect,
                uv: [0.0, 0.0, 1.0, 1.0],
                color: [opacity; 4],
                flags: 0,
            }],
            ImageSource::Pooled(&snapshot.image),
            BlendKey::SrcOver,
        )?;
        Ok(true)
    }

    fn can_freeze(&self, root: Option<LayerId>) -> bool {
        let mut pending: Vec<LayerId> = root.into_iter().collect();
        while let Some(id) = pending.pop() {
            if self.scene.tree.tracks.iter().any(|track| {
                track.layer == id
                    || track
                        .scroll_bindings
                        .iter()
                        .any(|binding| binding.layer == id)
            }) {
                return false;
            }
            let Some(layer) = self.scene.tree.layers.get(id.0 as usize) else {
                return false;
            };
            match layer.kind {
                LayerKind::Picture { picture } => {
                    let Some(picture) = self.scene.pictures.get(&picture) else {
                        return false;
                    };
                    if !picture.cache_safe
                        || picture.has_shader_effect
                        || picture.images.iter().any(|(source, _)| {
                            source.starts_with(EXTERNAL_TEXTURE_PREFIX)
                                || self.scene.textures.get(source).is_none()
                        })
                    {
                        return false;
                    }
                }
                LayerKind::BackdropBlur { .. }
                | LayerKind::BackdropMaterial { .. }
                | LayerKind::PlatformView { .. }
                | LayerKind::Blend { .. } => return false,
                _ => {}
            }
            pending.extend(layer.next);
            pending.extend(layer.first_child);
        }
        true
    }
}
