//! Frame uploads shared by every command buffer in a presentation.
use std::collections::HashMap;

use crate::{
    Status,
    gpu::Gpu,
    pipelines::{Layouts, ParamsUniform, ViewUniform},
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
        }
    }

    fn reset(&mut self) -> bool {
        let consolidate = self.block < MAX_SLAB && self.peak > self.block && self.slabs.len() > 1;
        if consolidate {
            self.block = self.peak.next_multiple_of(self.block).min(MAX_SLAB);
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
        let limit = Self::limit(gpu);
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
                self.peak = self.peak.saturating_add(consumed).min(MAX_SLAB);
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

    fn flush(&self, queue: &wgpu::Queue) {
        for slab in &self.slabs {
            if !slab.data.is_empty() {
                queue.write_buffer(&slab.buffer, 0, &slab.data);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Globals {
    View,
    Stops(usize),
    Params(usize),
}

#[derive(Clone, Copy)]
pub enum Extra {
    None,
    Stops(Address),
    Params(Address),
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
    vertices: Pool,
    uniforms: Pool,
    stops: Pool,
    groups: HashMap<(usize, Globals), wgpu::BindGroup>,
    pub buffers_created: u32,
    pub bind_groups_created: u32,
}

impl FrameArena {
    pub fn new() -> Self {
        Self {
            vertices: Pool::new(256 * 1024, wgpu::BufferUsages::VERTEX),
            uniforms: Pool::new(64 * 1024, wgpu::BufferUsages::UNIFORM),
            stops: Pool::new(STOPS_BYTES, wgpu::BufferUsages::UNIFORM),
            groups: HashMap::new(),
            buffers_created: 0,
            bind_groups_created: 0,
        }
    }

    pub fn reset(&mut self) {
        self.vertices.reset();
        if self.uniforms.reset() {
            self.groups.clear();
        }
        self.stops.active = 0;
        for slab in &mut self.stops.slabs {
            slab.data.clear();
        }
        self.buffers_created = 0;
        self.bind_groups_created = 0;
    }

    pub fn trim(&mut self) {
        for pool in [&mut self.vertices, &mut self.uniforms, &mut self.stops] {
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
    }

    pub fn flush(&self, gpu: &Gpu) {
        for pool in [&self.vertices, &self.uniforms, &self.stops] {
            pool.flush(&gpu.queue);
        }
    }

    pub fn bytes_uploaded(&self) -> [u64; 3] {
        [&self.vertices, &self.uniforms, &self.stops]
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

    pub fn same_stops(a: Address, b: Address) -> bool {
        a.slab == b.slab
    }

    pub fn group(
        &mut self,
        gpu: &Gpu,
        layouts: &Layouts,
        view: Address,
        extra: Extra,
    ) -> DynamicGroup {
        let (kind, offset) = match extra {
            Extra::Stops(stops) => (Globals::Stops(stops.slab), 0),
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
            let extra = match kind {
                Globals::View => None,
                Globals::Stops(slab) => Some(binding(
                    1,
                    &self.stops.slabs[slab].buffer,
                    STOPS_BYTES as u64,
                )),
                Globals::Params(slab) => Some(binding(
                    1,
                    &self.uniforms.slabs[slab].buffer,
                    size_of::<ParamsUniform>() as u64,
                )),
            };
            let (layout, entries) = match kind {
                Globals::View => (&layouts.view, [view.clone(), view]),
                Globals::Stops(_) => (
                    &layouts.paint,
                    [view, extra.unwrap_or_else(|| unreachable!("stops binding"))],
                ),
                Globals::Params(_) => (
                    &layouts.effects,
                    [
                        view,
                        extra.unwrap_or_else(|| unreachable!("params binding")),
                    ],
                ),
            };
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("impasto frame globals"),
                layout,
                entries: &entries[..if kind == Globals::View { 1 } else { 2 }],
            })
        });
        DynamicGroup {
            group: group.clone(),
            offsets: [view.offset, offset],
            count: if kind == Globals::View { 1 } else { 2 },
        }
    }
}
