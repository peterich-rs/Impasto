//! Retains the pixels of expensive, stable, isolated picture subtrees.

use std::collections::HashMap;
use std::sync::Arc;

use engine::scene::{LayerId, PictureId};

use super::{BlendKey, Coverage, Frame, FullPass, Image, OutputFormat, Status, scissor};
use crate::compiler::{Affine, Clip, CompiledPicture, IDENTITY, PictureBounds};

const CACHE_BUDGET: u64 = 24 * 1024 * 1024;
const STABLE_FRAMES: u8 = 3;
const CANDIDATE_LIMIT: usize = 2048;
pub(super) const MIN_RESAMPLED_BLUR: f32 = 2.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubtreeCacheStats {
    pub hits: u32,
    pub misses: u32,
    pub evictions: u32,
    pub bytes: u64,
}

#[derive(PartialEq)]
struct Key {
    picture: PictureId,
    revision: u64,
    format: wgpu::TextureFormat,
    scale: f32,
    pixel_alignment: Option<([f32; 2], [u32; 2])>,
    blur: Option<f32>,
    image_revisions: Box<[u64]>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct CacheSlot {
    pub layer: LayerId,
    pub group: Option<usize>,
}

struct Entry {
    key: Key,
    seen: u8,
    used: u64,
    image: Option<Image>,
    baked_at: [f32; 2],
    submitted: bool,
}

pub(super) struct SubtreeCache {
    slots: HashMap<CacheSlot, Entry>,
    clock: u64,
    enabled: bool,
    output: Option<OutputFormat>,
    stats: SubtreeCacheStats,
}

impl SubtreeCache {
    pub(super) fn new() -> Self {
        Self {
            slots: HashMap::new(),
            clock: 0,
            enabled: std::env::var("IMPASTO_SUBTREE_CACHE").as_deref() != Ok("0"),
            output: None,
            stats: SubtreeCacheStats::default(),
        }
    }

    #[cfg(test)]
    pub(super) fn disable(&mut self) {
        self.enabled = false;
        self.clear();
    }

    pub(super) fn clear(&mut self) {
        self.slots.clear();
        self.stats.bytes = 0;
    }

    pub(super) fn begin(&mut self) {
        self.clock = self.clock.wrapping_add(1);
        let bytes = self.stats.bytes;
        self.stats = SubtreeCacheStats {
            bytes,
            ..Default::default()
        };
        let cutoff = self.clock.saturating_sub(STABLE_FRAMES.into());
        self.remove_if(|slot| {
            if slot.image.is_some() {
                !slot.submitted
            } else {
                slot.used < cutoff
            }
        });
    }

    pub(super) fn submitted(&mut self) {
        for slot in self.slots.values_mut() {
            slot.submitted = slot.image.is_some();
        }
    }

    pub(super) fn output(&mut self, output: OutputFormat) {
        if self.output != Some(output) {
            self.clear();
            self.output = Some(output);
        }
    }

    pub(super) fn prune(&mut self, pictures: &HashMap<PictureId, Arc<CompiledPicture>>) {
        self.remove_if(|slot| {
            pictures
                .get(&slot.key.picture)
                .is_none_or(|picture| picture.resident_id != slot.key.revision)
        });
    }

    pub(super) fn stats(&self) -> SubtreeCacheStats {
        self.stats
    }

    fn remove_if(&mut self, remove: impl Fn(&Entry) -> bool) {
        self.slots.retain(|_, slot| {
            if remove(slot) {
                if let Some(image) = &slot.image {
                    self.stats.evictions += 1;
                    self.stats.bytes -= image.bytes();
                }
                false
            } else {
                true
            }
        });
    }

    fn evict(&mut self, id: CacheSlot) {
        if let Some(entry) = self.slots.remove(&id)
            && let Some(image) = entry.image
        {
            self.stats.evictions += 1;
            self.stats.bytes -= image.bytes();
        }
    }

    fn oldest(&self, except: CacheSlot, require_image: bool) -> Option<CacheSlot> {
        self.slots
            .iter()
            .filter(|(id, entry)| **id != except && (!require_image || entry.image.is_some()))
            .min_by_key(|(id, entry)| (entry.used, id.layer.0, id.group))
            .map(|(&id, _)| id)
    }

    fn observe(&mut self, id: CacheSlot, key: Key) -> &mut Entry {
        if self.slots.get(&id).is_some_and(|entry| entry.key != key) {
            self.evict(id);
        }
        if self.slots.len() >= CANDIDATE_LIMIT
            && !self.slots.contains_key(&id)
            && let Some(oldest) = self.oldest(id, false)
        {
            self.evict(oldest);
        }
        let entry = self.slots.entry(id).or_insert(Entry {
            key,
            seen: 0,
            used: self.clock.wrapping_sub(1),
            image: None,
            baked_at: [0.0; 2],
            submitted: false,
        });
        if entry.used != self.clock {
            entry.seen = if entry.used.wrapping_add(1) == self.clock {
                entry.seen.saturating_add(1)
            } else {
                1
            };
            entry.used = self.clock;
        }
        entry
    }

    fn reserve(&mut self, id: CacheSlot, bytes: u64) -> bool {
        if bytes > CACHE_BUDGET {
            return false;
        }
        while self.stats.bytes + bytes > CACHE_BUDGET {
            let Some(oldest) = self.oldest(id, true) else {
                return false;
            };
            self.evict(oldest);
        }
        true
    }
}

/// Facts needed to key both a picture and a surrounding layer effect.
pub(super) struct CacheCandidate<'a> {
    pub slot: CacheSlot,
    pub picture_id: PictureId,
    pub picture: &'a CompiledPicture,
    pub origin: Affine,
    pub bounds: PictureBounds,
    pub blur: Option<f32>,
}

impl Frame<'_> {
    pub(super) fn try_subtree_cache(
        &mut self,
        candidate: CacheCandidate<'_>,
        target: &Image,
        clip: &Clip,
        render: impl FnOnce(&mut Self, &Image) -> Result<(), Status>,
    ) -> Result<bool, Status> {
        let CacheCandidate {
            slot,
            picture_id,
            picture,
            origin,
            bounds,
            blur,
        } = candidate;
        if !self.compositor.subtrees.enabled
            || self.scene.slice.is_some()
            || self.damage_scissor.is_some()
            || self.baking_subtree
            || !picture.cache_safe
            || (!picture.cache_expensive && blur.is_none())
            || origin[..4] != IDENTITY[..4]
            || !clip.shapes.is_empty()
            || (target.view == self.root && self.gpu.caps.lcd && picture.has_glyphs)
        {
            return Ok(false);
        }
        let scale = self.scene.scale;
        let PictureBounds::Rect(bounds) = bounds.expand(1.0 / scale) else {
            return Ok(false);
        };
        let Some(area) = scissor(bounds, scale, self.size) else {
            return Ok(false);
        };
        // A cropped filter would bake the wrong source pixels. Wait for the
        // full subtree to be visible before admitting or sampling the image.
        let [x, y, w, h] = bounds;
        let contains = |[a, b, width, height]: [f32; 4]| {
            x >= a && y >= b && x + w <= a + width && y + h <= b + height
        };
        if !contains(target.area().map(|value| value as f32 / scale))
            || clip.scissor.is_some_and(|clip| !contains(clip))
        {
            return Ok(false);
        }
        let [x, y, width, height] = area;
        let logical = [width, height];
        let position = [origin[4] * scale, origin[5] * scale];
        let resample = blur.is_some_and(|sigma| {
            let pixels = sigma * scale;
            pixels.is_finite() && pixels >= MIN_RESAMPLED_BLUR
        });
        let image_revisions = picture
            .images
            .iter()
            .map(|(source, _)| {
                if source.starts_with(engine::scene::image::EXTERNAL_TEXTURE_PREFIX) {
                    return None;
                }
                self.scene.textures.get(source).map(|image| image.revision)
            })
            .collect::<Option<Box<_>>>();
        let Some(image_revisions) = image_revisions else {
            return Ok(false);
        };
        let key = Key {
            picture: picture_id,
            revision: picture.resident_id,
            format: target.format,
            scale,
            pixel_alignment: (!resample)
                .then(|| (position.map(|value| value.rem_euclid(1.0)), logical)),
            blur,
            image_revisions,
        };
        let cache = &mut self.compositor.subtrees;
        let entry = cache.observe(slot, key);
        let mut source = if let Some(image) = &entry.image {
            let mut image = image.clone();
            if resample {
                // The floor/ceil of the destination bounds may change in
                // addition to the fractional translation of the content.
                image.sample_offset = std::array::from_fn(|axis| {
                    (position[axis] - entry.baked_at[axis])
                        + (image.origin[axis] as f32 - area[axis] as f32)
                });
            }
            cache.stats.hits += 1;
            image
        } else {
            let stable = entry.seen >= STABLE_FRAMES;
            cache.stats.misses += 1;
            let texel = if target.format == wgpu::TextureFormat::Rgba16Float {
                8
            } else {
                4
            };
            let bytes = u64::from(width) * u64::from(height) * texel;
            if !stable || !cache.reserve(slot, bytes) {
                return Ok(false);
            }
            let mut image = Image::new(self.gpu, logical, target.format, 1);
            image.origin = [x, y];
            self.clear(&image.view);
            self.baking_subtree = true;
            let rendered = render(self, &image);
            self.baking_subtree = false;
            rendered?;
            self.finish(&image.view);
            let cache = &mut self.compositor.subtrees;
            if let Some(entry) = cache.slots.get_mut(&slot) {
                cache.stats.bytes += bytes;
                entry.image = Some(image.clone());
                entry.baked_at = position;
            }
            image
        };
        source.origin = [x, y];
        self.full(
            &source,
            None,
            &Coverage::default(),
            target,
            area,
            FullPass::Over {
                blend: BlendKey::SrcOver,
                opacity: 1.0,
            },
        )?;
        Ok(true)
    }
}
