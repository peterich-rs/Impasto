//! Stable layer state and render bundles for cached picture scopes.

use std::{
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    ops::Range,
};

use engine::scene::LayerId;

use crate::{
    Status,
    arena::BufferUpload,
    gpu::Gpu,
    pipelines::{self, ViewUniform},
};

const SLOTS_PER_PAGE: usize = 128;
const SLOT_LIMIT: usize = 4096;
const GEOMETRY_BUDGET: u64 = 32 * 1024 * 1024;
const PAINT_GROUP_LIMIT: usize = 128;
const MAX_IDLE_FRAMES: usize = 32;
const RETIRE_SLOTS_PER_FRAME: usize = SLOT_LIMIT / MAX_IDLE_FRAMES;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub picture: u64,
    pub layer: LayerId,
    pub scope: usize,
    pub draw: usize,
    pub format: wgpu::TextureFormat,
    pub revision: u64,
    pub atlas_epoch: u64,
    pub glyph_transform: [u32; 6],
}

pub struct ResidentBundle {
    slot: usize,
    bundle: wgpu::RenderBundle,
    bytes: u64,
    vertices: VertexRange,
    glyph_pages: Vec<crate::atlas::Placement>,
}

struct OccupiedSlot {
    key: Key,
    last_frame: usize,
}

struct StatePage {
    buffer: wgpu::Buffer,
    group: wgpu::BindGroup,
    data: Vec<u8>,
    dirty: u128,
}

const VERTEX_PAGE_BYTES: u64 = 256 * 1024;

struct VertexPage {
    buffer: wgpu::Buffer,
    free: Vec<Range<u64>>,
}

#[derive(Clone)]
pub struct VertexRange {
    page: usize,
    range: Range<u64>,
    pub buffer: wgpu::Buffer,
}

impl VertexRange {
    pub fn slice(&self) -> wgpu::BufferSlice<'_> {
        self.buffer.slice(self.range.clone())
    }
}

impl VertexPage {
    fn allocate(&mut self, size: u64) -> Option<Range<u64>> {
        let index = self
            .free
            .iter()
            .position(|region| region.end - region.start >= size)?;
        let start = self.free[index].start;
        let end = start + size;
        if end == self.free[index].end {
            self.free.remove(index);
        } else {
            self.free[index].start = end;
        }
        Some(start..end)
    }

    fn release(&mut self, range: Range<u64>) {
        let start = self.free.partition_point(|free| free.end < range.start);
        let count = self.free[start..]
            .iter()
            .take_while(|free| free.start <= range.end)
            .count();
        let merged = if count == 0 {
            range
        } else {
            self.free[start].start.min(range.start)..self.free[start + count - 1].end.max(range.end)
        };
        self.free.splice(start..start + count, [merged]);
    }
}

struct PaintEntry {
    state: wgpu::Buffer,
    stops: Box<[u8]>,
    paints: Box<[u8]>,
    group: wgpu::BindGroup,
}

#[derive(Default)]
pub struct ResidentPictures {
    bundles: HashMap<Key, ResidentBundle>,
    paints: HashMap<u64, PaintEntry>,
    previous: HashSet<Key>,
    seen: HashSet<Key>,
    slots: Vec<Option<OccupiedSlot>>,
    retire_cursor: usize,
    vertex_uploads: Vec<(usize, u64, Range<usize>)>,
    upload_data: Vec<u8>,
    pages: Vec<StatePage>,
    vertex_pages: Vec<VertexPage>,
    vertex_capacity: u64,
    vacant: Vec<usize>,
    next_slot: usize,
    stride: usize,
    frame: usize,
    checked_device: bool,
    bytes: u64,
}

pub struct LayerSlot {
    pub buffer: wgpu::Buffer,
    pub group: wgpu::BindGroup,
    pub offset: u32,
    index: usize,
}

impl ResidentPictures {
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.checked_device = false;
        std::mem::swap(&mut self.previous, &mut self.seen);
        self.seen.clear();
        let count = self.slots.len();
        for _ in 0..RETIRE_SLOTS_PER_FRAME.min(count) {
            if self.retire_cursor == count {
                self.retire_cursor = 0;
            }
            let slot = self.retire_cursor;
            self.retire_cursor += 1;
            if let Some(occupied) = &self.slots[slot]
                && self.frame.wrapping_sub(occupied.last_frame) >= MAX_IDLE_FRAMES
                && let Some(entry) = self.bundles.remove(&occupied.key)
            {
                self.slots[slot] = None;
                self.vacant.push(slot);
                self.vertex_pages[entry.vertices.page].release(entry.vertices.range);
                self.bytes -= entry.bytes;
            }
        }
    }

    /// Promote only after seeing the same geometry and resources in consecutive frames.
    pub fn observe_miss(&mut self, key: Key) -> bool {
        if self.seen.len() < SLOT_LIMIT {
            self.seen.insert(key);
        }
        self.previous.contains(&key)
    }

    pub fn get(
        &mut self,
        key: Key,
        atlas: &mut crate::atlas::Atlas,
    ) -> Option<(usize, wgpu::RenderBundle)> {
        if let Some(entry) = self.bundles.get(&key)
            && !entry
                .glyph_pages
                .iter()
                .all(|&page| atlas.resident_page_valid(page))
            && let Some(entry) = self.bundles.remove(&key)
        {
            self.slots[entry.slot] = None;
            self.vacant.push(entry.slot);
            self.vertex_pages[entry.vertices.page].release(entry.vertices.range);
            self.bytes -= entry.bytes;
            return None;
        }
        let entry = self.bundles.get(&key)?;
        for &page in &entry.glyph_pages {
            atlas.pin(page);
        }
        if let Some(slot) = &mut self.slots[entry.slot] {
            slot.last_frame = self.frame;
        }
        Some((entry.slot, entry.bundle.clone()))
    }

    pub fn check_recording_device(&mut self, gpu: &Gpu) -> Result<(), Status> {
        if !self.checked_device {
            // wgpu's bundle encoder reports a lost parent device as a panic.
            // Give the loss callback a chance to run before recording.
            let _ = gpu.device.poll(wgpu::PollType::Poll);
            self.checked_device = true;
        }
        gpu.check_async_health()
    }

    pub fn allocate(
        &mut self,
        gpu: &Gpu,
        layout: &wgpu::BindGroupLayout,
        state: &ViewUniform,
        bytes: u64,
    ) -> Result<Option<(LayerSlot, bool)>, Status> {
        if self.bytes.saturating_add(bytes) > GEOMETRY_BUDGET {
            return Ok(None);
        }
        let index = match self.vacant.pop() {
            Some(index) => index,
            None if self.next_slot < SLOT_LIMIT => self.next_slot,
            None => return Ok(None),
        };
        let page = index / SLOTS_PER_PAGE;
        let created = page == self.pages.len();
        if created {
            let alignment = gpu.device.limits().min_uniform_buffer_offset_alignment as usize;
            self.stride = size_of::<ViewUniform>().next_multiple_of(alignment);
            let size = self.stride * SLOTS_PER_PAGE;
            let (buffer, group) = gpu.validation(|| {
                let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("impasto resident layer states"),
                    size: size as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
                    mapped_at_creation: false,
                });
                let group = pipelines::bind_group(gpu, layout, &[Self::binding(&buffer)]);
                Ok((buffer, group))
            })?;
            self.pages.push(StatePage {
                buffer,
                group,
                data: vec![0; size],
                dirty: 0,
            });
        }
        if index == self.next_slot {
            self.next_slot += 1;
            self.slots.push(None);
        }
        self.update(index, state);
        let page = &self.pages[page];
        let offset =
            u32::try_from(index % SLOTS_PER_PAGE * self.stride).map_err(|_| Status::OutOfMemory)?;
        Ok(Some((
            LayerSlot {
                buffer: page.buffer.clone(),
                group: page.group.clone(),
                offset,
                index,
            },
            created,
        )))
    }

    pub fn vertices(&mut self, gpu: &Gpu, contents: &[u8]) -> Option<(VertexRange, bool)> {
        let size = contents.len() as u64;
        if size == 0 {
            return None;
        }
        let reusable = self
            .vertex_pages
            .iter_mut()
            .enumerate()
            .find_map(|(page, data)| {
                let range = data.allocate(size)?;
                Some((page, range, data.buffer.clone()))
            });
        let (page, range, buffer, created) = if let Some((page, range, buffer)) = reusable {
            (page, range, buffer, false)
        } else {
            let capacity = size
                .max(VERTEX_PAGE_BYTES)
                .next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
            if capacity > gpu.caps.max_buffer_size
                || self.vertex_capacity.saturating_add(capacity) > GEOMETRY_BUDGET
            {
                return None;
            }
            let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("impasto resident picture vertices"),
                size: capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let page = self.vertex_pages.len();
            self.vertex_pages.push(VertexPage {
                buffer: buffer.clone(),
                free: if size == capacity {
                    Vec::new()
                } else {
                    std::iter::once(size..capacity).collect()
                },
            });
            self.vertex_capacity += capacity;
            (page, 0..size, buffer, true)
        };
        // Retired ranges only return to the pool at a frame boundary.
        // Queue writes execute after earlier frames that used this range.
        let start = self.upload_data.len();
        self.upload_data.extend_from_slice(contents);
        let end = self.upload_data.len();
        if let Some((last_page, offset, data)) = self.vertex_uploads.last_mut()
            && *last_page == page
            && *offset + data.len() as u64 == range.start
        {
            data.end = end;
        } else {
            self.vertex_uploads.push((page, range.start, start..end));
        }
        Some((
            VertexRange {
                page,
                range,
                buffer,
            },
            created,
        ))
    }

    pub fn release_vertices(&mut self, vertices: VertexRange) {
        self.vertex_pages[vertices.page].release(vertices.range);
    }

    pub fn paint_group(
        &mut self,
        gpu: &Gpu,
        layout: &wgpu::BindGroupLayout,
        state: &wgpu::Buffer,
        lut: &wgpu::TextureView,
        stops: &[[f32; 4]],
        paints: &[crate::batch::PaintExtra],
    ) -> Result<(wgpu::BindGroup, [u64; 2]), Status> {
        let stop_bytes: &[u8] = bytemuck::cast_slice(stops);
        let paint_bytes: &[u8] = bytemuck::cast_slice(paints);
        let mut hasher = DefaultHasher::new();
        (state, stop_bytes, paint_bytes).hash(&mut hasher);
        let key = hasher.finish();
        if let Some(entry) = self.paints.get(&key)
            && &entry.state == state
            && *entry.stops == *stop_bytes
            && *entry.paints == *paint_bytes
        {
            return Ok((entry.group.clone(), [0; 2]));
        }
        let mut stops = stop_bytes.to_vec();
        stops.resize(pipelines::MAX_STOPS * 32, 0);
        let stops = pipelines::buffer_from(
            gpu,
            "resident gradient stops",
            &stops,
            wgpu::BufferUsages::UNIFORM,
        )?;
        let paints = pipelines::buffer_from(
            gpu,
            "resident rectangle paints",
            paint_bytes,
            wgpu::BufferUsages::STORAGE,
        )?;
        let material = pipelines::bind_group(
            gpu,
            layout,
            &[
                Self::binding(state),
                stops.as_entire_binding(),
                paints.as_entire_binding(),
                wgpu::BindingResource::TextureView(lut),
            ],
        );
        // Cached bundles themselves retain their immutable bindings.
        if self.paints.len() >= PAINT_GROUP_LIMIT
            && let Some(old) = self.paints.keys().next().copied()
        {
            self.paints.remove(&old);
        }
        let bytes = [stops.size(), paints.size()];
        self.paints.insert(
            key,
            PaintEntry {
                state: state.clone(),
                stops: stop_bytes.into(),
                paints: paint_bytes.into(),
                group: material.clone(),
            },
        );
        Ok((material, bytes))
    }

    pub fn insert(
        &mut self,
        key: Key,
        slot: LayerSlot,
        bundle: wgpu::RenderBundle,
        bytes: u64,
        vertices: VertexRange,
        glyph_pages: Vec<crate::atlas::Placement>,
    ) {
        self.slots[slot.index] = Some(OccupiedSlot {
            key,
            last_frame: self.frame,
        });
        self.bundles.insert(
            key,
            ResidentBundle {
                slot: slot.index,
                bundle,
                bytes,
                vertices,
                glyph_pages,
            },
        );
        self.bytes += bytes;
    }

    pub fn abandon(&mut self, slot: LayerSlot) {
        self.vacant.push(slot.index);
    }

    pub fn uploads(&self) -> impl Iterator<Item = BufferUpload<'_>> {
        self.vertex_uploads
            .iter()
            .map(|(page, offset, range)| BufferUpload {
                buffer: &self.vertex_pages[*page].buffer,
                offset: *offset,
                data: &self.upload_data[range.clone()],
            })
            .chain(self.state_uploads())
    }

    pub fn uniform_bytes(&self) -> u64 {
        self.state_uploads()
            .map(|upload| upload.data.len() as u64)
            .sum()
    }

    fn state_uploads(&self) -> impl Iterator<Item = BufferUpload<'_>> {
        let stride = self.stride;
        self.pages.iter().flat_map(move |page| {
            let mut dirty = page.dirty;
            std::iter::from_fn(move || {
                if dirty == 0 {
                    return None;
                }
                let first = dirty.trailing_zeros();
                let count = (dirty >> first).trailing_ones();
                let next = first + count;
                dirty = if next == 128 {
                    0
                } else {
                    (dirty >> next) << next
                };
                let start = first as usize * stride;
                let end = (next as usize - 1) * stride + size_of::<ViewUniform>();
                Some(BufferUpload {
                    buffer: &page.buffer,
                    offset: start as u64,
                    data: &page.data[start..end],
                })
            })
        })
    }

    pub fn clear_uploads(&mut self) {
        self.vertex_uploads.clear();
        self.upload_data.clear();
        for page in &mut self.pages {
            page.dirty = 0;
        }
    }

    pub fn binding(buffer: &wgpu::Buffer) -> wgpu::BindingResource<'_> {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer,
            offset: 0,
            size: wgpu::BufferSize::new(size_of::<ViewUniform>() as u64),
        })
    }

    pub fn update(&mut self, slot: usize, view: &ViewUniform) {
        let page = &mut self.pages[slot / SLOTS_PER_PAGE];
        let start = self.stride * (slot % SLOTS_PER_PAGE);
        let bytes = bytemuck::bytes_of(view);
        let end = start + bytes.len();
        if bytes != &page.data[start..end] {
            page.data[start..end].copy_from_slice(bytes);
            page.dirty |= 1 << (slot % SLOTS_PER_PAGE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_requires_the_same_resources_in_consecutive_frames() {
        let mut resident = ResidentPictures::default();
        let key = Key {
            picture: 1,
            layer: LayerId(0),
            scope: 0,
            draw: 0,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            revision: 1,
            atlas_epoch: 0,
            glyph_transform: [0; 6],
        };
        let updated = Key { revision: 2, ..key };

        resident.begin_frame();
        assert!(!resident.observe_miss(key));
        assert!(
            !resident.observe_miss(key),
            "two draws in one frame count once"
        );
        resident.begin_frame();
        assert!(
            !resident.observe_miss(updated),
            "new image revision starts cold"
        );
        resident.begin_frame();
        assert!(resident.observe_miss(updated));
        assert!(
            !resident.observe_miss(key),
            "a visibility gap breaks consecutive reuse"
        );
    }
}
