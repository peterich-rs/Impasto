//! GPU page allocation, reclamation, and batched texture uploads.

use etagere::{BucketedAtlasAllocator, size2};

use super::{
    Atlas, AtlasRect, AtlasTexture, AtlasTiles, GUTTER, PAGE_LIMIT, PAGE_SIZE, Page, PageBacking,
    TILE_SIDE,
};
use crate::gpu::Gpu;

impl Atlas {
    pub(super) fn allocate(&mut self, gpu: &Gpu, color: bool, w: u32, h: u32) -> Option<AtlasRect> {
        let empty = AtlasRect {
            page: usize::MAX,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            allocation: None,
        };
        if w == 0 || h == 0 {
            return Some(empty);
        }
        let required = w.max(h).checked_add(2 * GUTTER)?;
        if required > gpu.max_texture_size() {
            return None;
        }
        let page_size = self.page_size;
        if required > page_size {
            let page = self.add_page(gpu, color, [w + 2 * GUTTER, h + 2 * GUTTER], true);
            return Some(AtlasRect {
                page,
                x: GUTTER,
                y: GUTTER,
                w,
                h,
                allocation: None,
            });
        }
        let wanted = size2(w as i32 + GUTTER as i32 * 2, h as i32 + GUTTER as i32 * 2);
        let place = |allocation: etagere::Allocation, page: usize| AtlasRect {
            page,
            x: allocation.rectangle.min.x as u32 + GUTTER,
            y: allocation.rectangle.min.y as u32 + GUTTER,
            w,
            h,
            allocation: Some(allocation.id),
        };
        // Try existing pages.
        {
            let pages = self.pages_for(color);
            for (index, page) in pages.iter_mut().enumerate() {
                if page.size == [page_size; 2]
                    && let Some(allocation) = page.alloc.allocate(wanted)
                {
                    return Some(place(allocation, index));
                }
            }
        }
        let pages = self.pages_for_ref(color);
        let limit = self.page_limit(gpu) as usize;
        let full = pages.iter().filter(|page| !page.temporary()).count() >= limit;
        if full {
            let oldest = pages
                .iter()
                .enumerate()
                .filter(|(index, page)| {
                    !page.temporary() && !self.pinned.contains(&(color, *index))
                })
                .min_by_key(|(_, page)| page.age)
                .map(|(index, _)| index);
            if let Some(index) = oldest
                && let Some(allocation) = self.reclaim(color, index, wanted)
            {
                return Some(place(allocation, index));
            }
        }
        let index = self.add_page(gpu, color, [page_size; 2], full);
        let allocation = self
            .pages_for(color)
            .get_mut(index)?
            .alloc
            .allocate(wanted)?;
        Some(place(allocation, index))
    }

    pub(super) fn page_limit(&self, gpu: &Gpu) -> u32 {
        // Budget each array for at most three full 2048-square layers.
        let area = self.page_size * self.page_size;
        (3 * PAGE_SIZE * PAGE_SIZE / area)
            .min(gpu.caps.max_array_layers)
            .min(PAGE_LIMIT)
    }

    fn add_page(&mut self, gpu: &Gpu, color: bool, size: [u32; 2], temporary: bool) -> usize {
        let max_layers = self.page_limit(gpu);
        let page_size = self.page_size;
        let age = self.clock;
        let atlas = if color {
            &mut self.color
        } else {
            &mut self.mask
        };
        let backing = if temporary {
            PageBacking::Temporary {
                texture: AtlasTexture::new(gpu, color, size, 1),
                material: None,
            }
        } else {
            let layer = atlas.pages.iter().filter(|page| !page.temporary()).count();
            let layer = u32::try_from(layer).unwrap_or(0);
            let current_layers = atlas.texture.as_ref().map_or(0, |texture| texture.layers);
            if layer >= current_layers {
                let layers = (layer + 1).next_power_of_two().min(max_layers);
                let next = AtlasTexture::new(gpu, color, [page_size; 2], layers);
                if let Some(old) = atlas.texture.replace(next) {
                    atlas.retired.push(old);
                }
                self.material = None;
                self.epoch = self.epoch.wrapping_add(1);
            }
            PageBacking::Resident {
                layer,
                tiles: AtlasTiles::new(size),
            }
        };
        let index = atlas.pages.len();
        atlas.pages.push(Page {
            alloc: BucketedAtlasAllocator::new(size2(size[0] as i32, size[1] as i32)),
            size,
            backing,
            age,
            generation: 0,
        });
        self.stats.bytes = self.mask.bytes(self.mask_channels as usize) + self.color.bytes(4);
        index
    }

    pub(super) fn pages_for(&mut self, color: bool) -> &mut Vec<Page> {
        if color {
            &mut self.color.pages
        } else {
            &mut self.mask.pages
        }
    }

    pub(super) fn pages_for_ref(&self, color: bool) -> &[Page] {
        if color {
            &self.color.pages
        } else {
            &self.mask.pages
        }
    }

    fn reclaim(
        &mut self,
        color: bool,
        index: usize,
        wanted: etagere::Size,
    ) -> Option<etagere::Allocation> {
        // A bucket is reused once its last glyph leaves. Bound the work per
        // attempt to leave enough time for a temporary page in this frame.
        const RECLAIM_BATCH: usize = 32;
        const RECLAIM_LIMIT: usize = 512;
        let mut victims: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(key, slot)| {
                let entry = self.slots.get(*slot)?;
                let placement = entry.placement;
                (placement.color == color && placement.rect.page == index)
                    .then(|| Some((entry.used, key.clone(), placement.rect.allocation?)))
                    .flatten()
            })
            .collect();
        victims.sort_unstable_by_key(|(used, _, _)| *used);
        if victims.is_empty() {
            return None;
        }
        let page = self.pages_for(color).get_mut(index)?;
        page.generation = page.generation.wrapping_add(1);
        for batch in victims[..victims.len().min(RECLAIM_LIMIT)].chunks(RECLAIM_BATCH) {
            for (_, key, allocation) in batch {
                if let Some(slot) = self.entries.remove(key) {
                    self.slots.remove(slot);
                }
                self.pages_for(color)[index].alloc.deallocate(*allocation);
            }
            self.stats.evictions += u32::try_from(batch.len()).unwrap_or(u32::MAX);
            self.stats.region_reclaims += 1;
            self.stats.entries = self.entries.len();
            if let Some(allocation) = self.pages_for(color)[index].alloc.allocate(wanted) {
                return Some(allocation);
            }
        }
        None
    }

    pub(super) fn upload(
        &mut self,
        gpu: &Gpu,
        color: bool,
        rect: AtlasRect,
        data: &[u8],
    ) -> Option<()> {
        let channels = if color { 4 } else { self.mask_channels };
        let row = (rect.w as usize).checked_mul(channels as usize)?;
        let atlas = if color {
            &mut self.color
        } else {
            &mut self.mask
        };
        let page = atlas.pages.get_mut(rect.page)?;
        if data.len() < row.checked_mul(rect.h as usize)? {
            return None;
        }
        let w = rect.w.checked_add(2 * GUTTER)?;
        let h = rect.h.checked_add(2 * GUTTER)?;
        let stride = zero_scratch(&mut self.scratch, [w, h], channels)?;
        for (y, source) in data.chunks_exact(row).take(rect.h as usize).enumerate() {
            let start = (y + GUTTER as usize) * stride + (GUTTER * channels) as usize;
            self.scratch[start..start + row].copy_from_slice(source);
        }
        let origin = [rect.x - GUTTER, rect.y - GUTTER];
        match &mut page.backing {
            PageBacking::Resident { tiles, .. } => {
                tiles.write(origin, [w, h], channels, &self.scratch);
            }
            PageBacking::Temporary { texture, .. } => {
                let [x, y] = origin;
                self.stats.upload_bytes += write_texture(
                    gpu,
                    texture,
                    [x, y, 0],
                    [w, h],
                    channels,
                    w * channels,
                    &self.scratch,
                );
                self.stats.upload_calls += 1;
            }
        }
        Some(())
    }

    /// Upload the changed rectangle of each dirty tile to every referenced array.
    pub fn flush(&mut self, gpu: &Gpu, prelude: &mut Option<wgpu::CommandEncoder>) {
        for (atlas, channels) in [(&mut self.mask, self.mask_channels), (&mut self.color, 4)] {
            let Some(texture) = &atlas.texture else {
                continue;
            };
            for page in &mut atlas.pages {
                let PageBacking::Resident { layer, tiles } = &mut page.backing else {
                    continue;
                };
                let row_stride = TILE_SIDE * channels;
                for (index, slot) in (0..).zip(&mut tiles.tiles) {
                    let Some(tile) = slot else {
                        continue;
                    };
                    let Some([x0, y0, x1, y1]) = tile.dirty.take() else {
                        continue;
                    };
                    let x = index % tiles.columns * TILE_SIDE + x0;
                    let y = index / tiles.columns * TILE_SIDE + y0;
                    let start = (y0 * row_stride + x0 * channels) as usize;
                    for target in std::iter::once(texture).chain(&atlas.retired) {
                        if *layer < target.layers {
                            self.stats.upload_bytes += write_texture(
                                gpu,
                                target,
                                [x, y, *layer],
                                [x1 - x0, y1 - y0],
                                channels,
                                row_stride,
                                &tile.pixels[start..],
                            );
                            self.stats.upload_calls += 1;
                        }
                    }
                }
            }
        }
        for atlas in [&mut self.mask, &mut self.color] {
            for (old, next) in atlas
                .retired
                .iter()
                .zip(atlas.retired.iter().skip(1).chain(&atlas.texture))
            {
                // Writes are mirrored to both arrays before submit, so the copy
                // preserves new glyphs and brings previously uploaded layers forward.
                crate::arena::upload_encoder(gpu, prelude).copy_texture_to_texture(
                    old.texture.as_image_copy(),
                    next.texture.as_image_copy(),
                    wgpu::Extent3d {
                        width: self.page_size,
                        height: self.page_size,
                        depth_or_array_layers: old.layers,
                    },
                );
                self.stats.copy_pixels += u64::from(self.page_size).pow(2) * u64::from(old.layers);
            }
            atlas.retired.clear();
        }
    }
}

fn zero_scratch(scratch: &mut Vec<u8>, size: [u32; 2], channels: u32) -> Option<usize> {
    let stride = (size[0] as usize).checked_mul(channels as usize)?;
    let length = stride.checked_mul(size[1] as usize)?;
    scratch.clear();
    scratch.try_reserve_exact(length).ok()?;
    scratch.resize(length, 0);
    Some(stride)
}

fn write_texture(
    gpu: &Gpu,
    texture: &AtlasTexture,
    origin: [u32; 3],
    size: [u32; 2],
    channels: u32,
    row_stride: u32,
    data: &[u8],
) -> u64 {
    let [x, y, z] = origin;
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture.texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x, y, z },
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(row_stride),
            rows_per_image: Some(size[1]),
        },
        crate::compositor::extent(size),
    );
    u64::from(size[0]) * u64::from(size[1]) * u64::from(channels)
}
