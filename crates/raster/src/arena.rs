//! Frame uploads shared by every command buffer in a presentation.
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::{
    Status,
    batch::{PAINT_EXTRA_BYTES, PaintExtra},
    gpu::Gpu,
    pipelines::{EXTRA_BYTES, Layouts, ParamsUniform, ViewUniform},
};

const MAX_SLAB: usize = 16 * 1024 * 1024;
const MAX_GLOBAL_GROUPS: usize = 1024;
const STOPS_BYTES: usize = crate::pipelines::MAX_STOPS * 32;

struct Slab {
    buffer: wgpu::Buffer,
    data: Vec<u8>,
    capacity: usize,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Address {
    slab: usize,
    offset: u32,
}

pub struct VertexSlice {
    buffer: wgpu::Buffer,
    start: u64,
    end: u64,
}

impl VertexSlice {
    pub fn slice(&self) -> wgpu::BufferSlice<'_> {
        self.buffer.slice(self.start..self.end)
    }
}

struct Pool {
    slabs: Vec<Slab>,
    active: usize,
    block: usize,
    initial: usize,
    peak: usize,
    usage: wgpu::BufferUsages,
    max_slab: usize,
}

impl Pool {
    fn new(block: usize, usage: wgpu::BufferUsages) -> Self {
        Self {
            slabs: Vec::new(),
            active: 0,
            block,
            initial: block,
            peak: 0,
            usage,
            max_slab: MAX_SLAB,
        }
    }

    fn reset(&mut self) -> bool {
        let consolidate =
            self.block < self.max_slab && self.peak > self.block && self.slabs.len() > 1;
        if consolidate {
            self.block = self.peak.next_multiple_of(self.block).min(self.max_slab);
            self.slabs.clear();
        }
        self.peak = 0;
        self.active = 0;
        for slab in &mut self.slabs {
            slab.data.clear();
        }
        consolidate
    }

    fn push(
        &mut self,
        gpu: &Gpu,
        data: &[u8],
        alignment: usize,
    ) -> Result<(Address, bool), Status> {
        let length = data.len();
        let limit = Self::limit(gpu).min(self.max_slab);
        if length > limit {
            return Err(Status::OutOfMemory);
        }
        let mut created = false;
        loop {
            if self.active == self.slabs.len() {
                let capacity = self.block.max(length).next_multiple_of(4).min(limit);
                let mut staging = Vec::new();
                staging
                    .try_reserve_exact(capacity)
                    .map_err(|_| Status::OutOfMemory)?;
                self.slabs.push(Slab {
                    buffer: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("impasto frame slab"),
                        size: capacity as u64,
                        usage: self.usage | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    data: staging,
                    capacity,
                });
                created = true;
            }
            let slab = &mut self.slabs[self.active];
            let offset = slab.data.len().next_multiple_of(alignment);
            if offset <= slab.capacity && length <= slab.capacity - offset {
                let consumed = offset + length - slab.data.len();
                slab.data.resize(offset, 0);
                slab.data.extend_from_slice(data);
                self.peak = self.peak.saturating_add(consumed).min(self.max_slab);
                return Ok((
                    Address {
                        slab: self.active,
                        offset: u32::try_from(offset).map_err(|_| Status::OutOfMemory)?,
                    },
                    created,
                ));
            }
            self.active += 1;
        }
    }

    fn limit(gpu: &Gpu) -> usize {
        usize::try_from(gpu.caps.max_buffer_size)
            .unwrap_or(MAX_SLAB)
            .min(MAX_SLAB)
            / 4
            * 4
    }
}

pub struct BufferUpload<'a> {
    pub buffer: &'a wgpu::Buffer,
    pub offset: u64,
    pub data: &'a [u8],
}

pub fn upload_encoder<'a>(
    gpu: &Gpu,
    prelude: &'a mut Option<wgpu::CommandEncoder>,
) -> &'a mut wgpu::CommandEncoder {
    prelude.get_or_insert_with(|| {
        gpu.device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("impasto frame uploads"),
            })
    })
}

struct UploadSlot {
    belt: wgpu::util::StagingBelt,
    busy: Arc<AtomicBool>,
}

struct FrameRing {
    slots: [UploadSlot; 3],
    next: usize,
    submitted: Option<usize>,
}

impl FrameRing {
    fn new(gpu: &Gpu) -> Self {
        Self {
            slots: std::array::from_fn(|_| UploadSlot {
                belt: wgpu::util::StagingBelt::new(gpu.device.clone(), 256 * 1024),
                busy: Arc::default(),
            }),
            next: 0,
            submitted: None,
        }
    }

    fn encode<'a>(
        &mut self,
        gpu: &Gpu,
        prelude: &mut Option<wgpu::CommandEncoder>,
        uploads: impl Iterator<Item = BufferUpload<'a>>,
    ) -> Result<(), Status> {
        let slots = self.slots.len();
        let ready = (0..slots)
            .map(|offset| (self.next + offset) % slots)
            .find(|&index| !self.slots[index].busy.load(Ordering::Acquire));
        if let Some(index) = ready {
            let slot = &mut self.slots[index];
            let mut any = false;
            for upload in uploads {
                if let Some(size) = wgpu::BufferSize::new(upload.data.len() as u64) {
                    let slice = slot.belt.allocate(size, wgpu::BufferSize::MIN);
                    let mapped = slice.get_mapped_range_mut();
                    if let Ok(mut mapped) = mapped {
                        mapped.copy_from_slice(upload.data);
                        drop(mapped);
                        upload_encoder(gpu, prelude).copy_buffer_to_buffer(
                            slice.buffer(),
                            slice.offset(),
                            upload.buffer,
                            upload.offset,
                            size.get(),
                        );
                        any = true;
                    } else {
                        slot.belt = wgpu::util::StagingBelt::new(gpu.device.clone(), 256 * 1024);
                        return Err(Status::PresentFailed);
                    }
                }
            }
            if any {
                slot.belt
                    .finish_and_recall_on_submit(upload_encoder(gpu, prelude));
                self.submitted = Some(index);
                self.next = (index + 1) % slots;
            }
        } else {
            // A saturated queue can still present without waiting for a slot.
            for upload in uploads.filter(|upload| !upload.data.is_empty()) {
                gpu.queue
                    .write_buffer(upload.buffer, upload.offset, upload.data);
            }
        }
        Ok(())
    }

    fn submitted(&mut self, queue: &wgpu::Queue) {
        if let Some(index) = self.submitted.take() {
            let busy = Arc::clone(&self.slots[index].busy);
            busy.store(true, Ordering::Release);
            queue.on_submitted_work_done(move || busy.store(false, Ordering::Release));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Globals {
    View,
    Paint { stops: usize, extra: usize },
    Params(usize),
}

#[derive(Clone, Copy)]
pub enum Extra {
    None,
    Paint(PaintPage),
    Params(Address),
}

/// Both pages are part of a paint batch's binding key.
#[derive(Clone, Copy)]
pub struct PaintPage {
    pub stops: Address,
    pub extra: Address,
}

impl PaintPage {
    pub fn same_bindings(self, other: Self) -> bool {
        self.stops.slab == other.stops.slab && self.extra.slab == other.extra.slab
    }
}

#[derive(Clone)]
pub struct DynamicGroup {
    pub group: wgpu::BindGroup,
    pub offsets: [u32; 2],
    pub count: usize,
}

impl DynamicGroup {
    pub fn offsets(&self) -> &[u32] {
        &self.offsets[..self.count]
    }
}

pub struct FrameArena {
    ring: FrameRing,
    vertices: Pool,
    uniforms: Pool,
    stops: Pool,
    paint: Pool,
    pub lut: crate::gradient_lut::GradientLut,
    paint_entries: HashMap<[u32; size_of::<PaintExtra>() / 4], Address>,
    groups: HashMap<(usize, Globals), wgpu::BindGroup>,
    pub buffers_created: u32,
    pub bind_groups_created: u32,
}

impl FrameArena {
    pub fn new(gpu: &Gpu) -> Self {
        let mut paint = Pool::new(
            EXTRA_BYTES,
            if gpu.caps.paint_storage {
                wgpu::BufferUsages::STORAGE
            } else {
                wgpu::BufferUsages::UNIFORM
            },
        );
        paint.max_slab = if gpu.caps.paint_storage {
            usize::try_from(gpu.caps.max_storage_binding)
                .unwrap_or(MAX_SLAB)
                .min(MAX_SLAB)
                / 16
                * 16
        } else {
            EXTRA_BYTES
        };
        if gpu.caps.paint_storage {
            paint.block = (64 * 1024).min(paint.max_slab);
            paint.initial = paint.block;
        }

        Self {
            ring: FrameRing::new(gpu),
            vertices: Pool::new(256 * 1024, wgpu::BufferUsages::VERTEX),
            uniforms: Pool::new(64 * 1024, wgpu::BufferUsages::UNIFORM),
            stops: Pool {
                max_slab: STOPS_BYTES,
                ..Pool::new(STOPS_BYTES, wgpu::BufferUsages::UNIFORM)
            },
            paint,
            paint_entries: HashMap::new(),
            lut: crate::gradient_lut::GradientLut::new(gpu),
            groups: HashMap::new(),
            buffers_created: 0,
            bind_groups_created: 0,
        }
    }

    pub fn reset(&mut self) {
        self.vertices.reset();
        let uniforms_changed = self.uniforms.reset();
        let paint_changed = self.paint.reset();
        if uniforms_changed || paint_changed {
            self.groups.clear();
        }
        self.stops.reset();
        self.paint_entries.clear();
        self.lut.reset();
        self.buffers_created = 0;
        self.bind_groups_created = 0;
    }

    pub fn trim(&mut self) {
        for pool in [
            &mut self.vertices,
            &mut self.uniforms,
            &mut self.stops,
            &mut self.paint,
        ] {
            pool.block = pool.initial;
            pool.peak = 0;
            pool.active = 0;
            if pool
                .slabs
                .first()
                .is_some_and(|slab| slab.capacity == pool.initial)
            {
                pool.slabs.truncate(1);
                pool.slabs[0].data.clear();
            } else {
                pool.slabs.clear();
            }
        }
        self.groups.clear();
        self.paint_entries.clear();
        self.lut.trim();
    }

    pub fn flush<'a>(
        &'a mut self,
        gpu: &Gpu,
        prelude: &mut Option<wgpu::CommandEncoder>,
        resident: impl Iterator<Item = BufferUpload<'a>>,
    ) -> Result<(), Status> {
        let immediate = [&self.vertices, &self.uniforms, &self.stops, &self.paint]
            .into_iter()
            .flat_map(|pool| &pool.slabs)
            .map(|slab| BufferUpload {
                buffer: &slab.buffer,
                offset: 0,
                data: &slab.data,
            });
        self.ring.encode(gpu, prelude, immediate.chain(resident))
    }

    pub fn submitted(&mut self, gpu: &Gpu) {
        self.ring.submitted(&gpu.queue);
    }

    pub fn bytes_uploaded(&self) -> [u64; 4] {
        [&self.vertices, &self.uniforms, &self.stops, &self.paint]
            .map(|pool| pool.slabs.iter().map(|slab| slab.data.len() as u64).sum())
    }

    pub fn vertex_limit<T: bytemuck::Pod>(gpu: &Gpu) -> usize {
        Pool::limit(gpu) / size_of::<T>()
    }

    pub fn vertices<T: bytemuck::Pod>(
        &mut self,
        gpu: &Gpu,
        data: &[T],
    ) -> Result<VertexSlice, Status> {
        let bytes = bytemuck::cast_slice(data);
        let (address, created) = self.vertices.push(gpu, bytes, 4)?;
        self.buffers_created += u32::from(created);
        let start = u64::from(address.offset);
        Ok(VertexSlice {
            buffer: self.vertices.slabs[address.slab].buffer.clone(),
            start,
            end: start + bytes.len() as u64,
        })
    }

    pub fn uniform<T: bytemuck::Pod>(&mut self, gpu: &Gpu, value: &T) -> Result<Address, Status> {
        let (address, created) = self.uniforms.push(
            gpu,
            bytemuck::bytes_of(value),
            gpu.caps.min_uniform_align.max(256) as usize,
        )?;
        self.buffers_created += u32::from(created);
        Ok(address)
    }

    pub fn uniform_buffer(&self, address: Address) -> (&wgpu::Buffer, u32) {
        (&self.uniforms.slabs[address.slab].buffer, address.offset)
    }

    pub fn owns_uniform_buffer(&self, buffer: &wgpu::Buffer) -> bool {
        self.uniforms
            .slabs
            .iter()
            .any(|slab| &slab.buffer == buffer)
    }

    pub fn stops(&mut self, gpu: &Gpu, values: &[[f32; 4]]) -> Result<Address, Status> {
        if values.is_empty() && !self.stops.slabs.is_empty() {
            return Ok(Address {
                slab: self.stops.active,
                offset: 0,
            });
        }
        let (address, created) = self.stops.push(gpu, bytemuck::cast_slice(values), 16)?;
        self.buffers_created += u32::from(created);
        Ok(address)
    }

    pub fn stop_index(address: Address) -> f32 {
        (address.offset / 16) as f32
    }

    pub fn paint(
        &mut self,
        gpu: &Gpu,
        stops: Address,
        value: Option<PaintExtra>,
    ) -> Result<(PaintPage, u32), Status> {
        let (extra, index) = if let Some(value) = value {
            let key = bytemuck::cast(value);
            let extra = if let Some(&address) = self.paint_entries.get(&key) {
                address
            } else {
                let address = self.push_paint(gpu, &value)?;
                self.paint_entries.insert(key, address);
                address
            };
            (extra, extra.offset / PAINT_EXTRA_BYTES)
        } else {
            if self.paint.slabs.is_empty() {
                let (_, created) = self.paint.push(gpu, &[], 16)?;
                self.buffers_created += u32::from(created);
            }
            (
                Address {
                    slab: self.paint.active,
                    offset: 0,
                },
                u32::MAX,
            )
        };
        Ok((PaintPage { stops, extra }, index))
    }

    fn push_paint(&mut self, gpu: &Gpu, value: &PaintExtra) -> Result<Address, Status> {
        let (address, created) = self.paint.push(gpu, bytemuck::bytes_of(value), 16)?;
        self.buffers_created += u32::from(created);
        Ok(address)
    }

    pub fn compiled_paint(
        &mut self,
        gpu: &Gpu,
        stops: Address,
        value: PaintExtra,
    ) -> Result<(PaintPage, u32), Status> {
        let extra = self.push_paint(gpu, &value)?;
        Ok((PaintPage { stops, extra }, extra.offset / PAINT_EXTRA_BYTES))
    }

    pub fn group(
        &mut self,
        gpu: &Gpu,
        layouts: &Layouts,
        view: Address,
        extra: Extra,
    ) -> DynamicGroup {
        let (kind, offset) = match extra {
            Extra::Paint(page) => (
                Globals::Paint {
                    stops: page.stops.slab,
                    extra: page.extra.slab,
                },
                0,
            ),
            Extra::Params(params) => (Globals::Params(params.slab), params.offset),
            Extra::None => (Globals::View, 0),
        };
        let key = (view.slab, kind);
        if self.groups.len() >= MAX_GLOBAL_GROUPS
            && !self.groups.contains_key(&key)
            && let Some(evicted) = self.groups.keys().next().copied()
        {
            // Encoded and pending draws own their handles independently of this cache.
            self.groups.remove(&evicted);
        }
        let group = self.groups.entry(key).or_insert_with(|| {
            self.bind_groups_created += 1;
            let binding = |binding, buffer, size| wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(size),
                }),
            };
            let view = binding(
                0,
                &self.uniforms.slabs[view.slab].buffer,
                size_of::<ViewUniform>() as u64,
            );
            let mut entries = vec![view];
            let layout = match kind {
                Globals::View => &layouts.view,
                Globals::Paint { stops, extra } => {
                    entries.push(binding(
                        1,
                        &self.stops.slabs[stops].buffer,
                        STOPS_BYTES as u64,
                    ));
                    entries.push(binding(
                        2,
                        &self.paint.slabs[extra].buffer,
                        self.paint.slabs[extra].capacity as u64,
                    ));
                    entries.push(wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&self.lut.view),
                    });
                    &layouts.paint
                }
                Globals::Params(slab) => {
                    entries.push(binding(
                        1,
                        &self.uniforms.slabs[slab].buffer,
                        size_of::<ParamsUniform>() as u64,
                    ));
                    &layouts.effects
                }
            };
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("impasto frame globals"),
                layout,
                entries: &entries,
            })
        });
        DynamicGroup {
            group: group.clone(),
            offsets: [view.offset, offset],
            count: if kind == Globals::View { 1 } else { 2 },
        }
    }
}
