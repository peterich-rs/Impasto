//! Bounded, nonblocking timestamp readback on the owning raster queue.

use std::sync::{Arc, mpsc};

use engine::scene::ShaderProgramId;

use crate::{RasterBackend, gpu::Gpu};

const SLOT_COUNT: usize = 6;
const MAX_PASSES: u32 = 256;
const QUERY_BYTES: u64 = size_of::<u64>() as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuPassKind {
    Main,
    OffscreenEffect,
    BackdropBlur,
    CanvasMsaa,
    EarlyZ,
    ClipMask,
    ComputeBlur,
    Mipmap,
    Copy,
    ShaderEffect,
    GpuView,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuTimingStatus {
    #[default]
    Disabled,
    Unsupported,
    DroppedSlotFull,
    DroppedPassLimit,
    Pending,
    StaleDevice,
    ReadbackFailed,
    Ready,
}

impl GpuTimingStatus {
    pub fn name(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Unsupported => "unsupported",
            Self::DroppedSlotFull => "droppedSlotFull",
            Self::DroppedPassLimit => "droppedPassLimit",
            Self::Pending => "pending",
            Self::StaleDevice => "staleDevice",
            Self::ReadbackFailed => "readbackFailed",
            Self::Ready => "ready",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GpuSample {
    pub frame_id: u64,
    pub device_generation: u64,
    pub status: GpuTimingStatus,
}

/// Application source of an independently timed pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuPassSource {
    ShaderEffect(ShaderProgramId),
    GpuView(Arc<str>),
}

#[derive(Clone, Debug)]
pub struct GpuPassTiming {
    pub kind: GpuPassKind,
    pub source: Option<GpuPassSource>,
    pub slice_index: Option<usize>,
    pub draws: u32,
    pub start_us: f64,
    pub duration_us: f64,
}

impl GpuPassTiming {
    /// The registered program associated with an application shader pass.
    pub fn shader_id(&self) -> Option<ShaderProgramId> {
        match self.source.as_ref()? {
            GpuPassSource::ShaderEffect(id) => Some(*id),
            _ => None,
        }
    }

    /// The producer name associated with a native GPU view pass.
    pub fn view_id(&self) -> Option<&str> {
        match self.source.as_ref()? {
            GpuPassSource::GpuView(source) => {
                source.strip_prefix(engine::scene::image::GPU_VIEW_TEXTURE_PREFIX)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GpuFrameTiming {
    /// Timestamp clock origin for this sample. Only comparable within a device generation.
    pub clock_start_us: f64,
    pub frame_span_us: f64,
    pub passes_sum_us: f64,
    pub passes: Vec<GpuPassTiming>,
}

#[derive(Debug)]
pub struct GpuTimingUpdate {
    pub sample: GpuSample,
    pub timing: Option<GpuFrameTiming>,
}

struct Buffers {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
}

pub(crate) struct GpuProfiler {
    slots: [Arc<Buffers>; SLOT_COUNT],
    pending: Vec<Readback>,
}

struct Readback {
    capture: Capture,
    state: ReadbackState,
}

enum ReadbackState {
    Rendering(mpsc::Receiver<()>),
    Mapping(mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>),
}

pub(crate) struct Capture {
    buffers: Arc<Buffers>,
    sample: GpuSample,
    passes: Vec<Pass>,
    overflow: bool,
}

struct Pass {
    kind: GpuPassKind,
    source: Option<GpuPassSource>,
    slice_index: Option<usize>,
    draws: u32,
}

impl GpuProfiler {
    pub fn new(gpu: &Gpu) -> Option<Self> {
        gpu.caps.has_timestamp_query.then(|| Self {
            slots: std::array::from_fn(|_| {
                let count = MAX_PASSES * 2;
                let buffer = |label, usage| {
                    gpu.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(label),
                        size: u64::from(count) * QUERY_BYTES,
                        usage,
                        mapped_at_creation: false,
                    })
                };
                Arc::new(Buffers {
                    queries: gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
                        label: Some("frame pass timestamps"),
                        ty: wgpu::QueryType::Timestamp,
                        count,
                    }),
                    resolve: buffer(
                        "timestamp resolve",
                        wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    ),
                    read: buffer(
                        "timestamp readback",
                        wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    ),
                })
            }),
            pending: Vec::with_capacity(SLOT_COUNT),
        })
    }

    pub fn begin(&self, mut sample: GpuSample) -> Option<Capture> {
        let buffers = self
            .slots
            .iter()
            .find(|slot| Arc::strong_count(slot) == 1)?;
        sample.status = GpuTimingStatus::Pending;
        Some(Capture {
            buffers: Arc::clone(buffers),
            sample,
            passes: Vec::new(),
            overflow: false,
        })
    }

    pub fn submitted(&mut self, gpu: &Gpu, capture: Capture) -> GpuSample {
        let status = capture.status();
        let sample = GpuSample {
            status,
            ..capture.sample
        };
        if status == GpuTimingStatus::Pending {
            let state = if gpu.backend == RasterBackend::Metal {
                // On Apple GPUs the resolve can race the fragment-boundary
                // write, even in the same encoder. Keep the slot leased until
                // the render submission completes, then resolve in drain().
                let (ready, rendering) = mpsc::sync_channel(1);
                gpu.queue.on_submitted_work_done(move || {
                    let _ = ready.try_send(());
                });
                ReadbackState::Rendering(rendering)
            } else {
                capture.map()
            };
            self.pending.push(Readback { capture, state });
        }
        sample
    }

    pub fn retire(&mut self) -> Vec<GpuTimingUpdate> {
        self.pending
            .drain(..)
            .map(|pending| GpuTimingUpdate {
                sample: GpuSample {
                    status: GpuTimingStatus::StaleDevice,
                    ..pending.capture.sample
                },
                timing: None,
            })
            .collect()
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn drain(&mut self, gpu: &Gpu) -> Vec<GpuTimingUpdate> {
        let _ = gpu.device.poll(wgpu::PollType::Poll);
        if gpu.is_lost() {
            return self.retire();
        }
        let period = f64::from(gpu.queue.get_timestamp_period()) / 1_000.0;
        let mut updates = Vec::new();
        self.pending.retain_mut(|pending| {
            let capture = &pending.capture;
            let (status, timing) = match &pending.state {
                ReadbackState::Rendering(completed) => match completed.try_recv() {
                    Ok(()) => {
                        let mut encoder =
                            gpu.device
                                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                    label: Some("impasto Metal timestamp resolve"),
                                });
                        capture.resolve(&mut encoder);
                        gpu.submit([encoder.finish()]);
                        pending.state = capture.map();
                        return true;
                    }
                    Err(mpsc::TryRecvError::Empty) => return true,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        (GpuTimingStatus::ReadbackFailed, None)
                    }
                },
                ReadbackState::Mapping(completed) => match completed.try_recv() {
                    Ok(Ok(())) => {
                        let timing = capture.read(period);
                        let status = if timing.is_some() {
                            GpuTimingStatus::Ready
                        } else {
                            GpuTimingStatus::ReadbackFailed
                        };
                        (status, timing)
                    }
                    Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                        (GpuTimingStatus::ReadbackFailed, None)
                    }
                    Err(mpsc::TryRecvError::Empty) => return true,
                },
            };
            updates.push(GpuTimingUpdate {
                sample: GpuSample {
                    status,
                    ..capture.sample
                },
                timing,
            });
            false
        });
        updates
    }
}

impl Capture {
    pub fn pass(
        &mut self,
        kind: GpuPassKind,
        slice: Option<usize>,
        draws: u32,
    ) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.tagged_pass(kind, slice, draws, None)
    }

    pub fn tagged_pass(
        &mut self,
        kind: GpuPassKind,
        slice_index: Option<usize>,
        draws: u32,
        source: Option<GpuPassSource>,
    ) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let index = u32::try_from(self.passes.len()).ok()?;
        if index >= MAX_PASSES {
            self.overflow = true;
            return None;
        }
        self.passes.push(Pass {
            kind,
            source,
            slice_index,
            draws,
        });
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.buffers.queries,
            beginning_of_pass_write_index: Some(index * 2),
            end_of_pass_write_index: Some(index * 2 + 1),
        })
    }

    fn status(&self) -> GpuTimingStatus {
        if self.overflow {
            GpuTimingStatus::DroppedPassLimit
        } else if self.passes.is_empty() {
            GpuTimingStatus::ReadbackFailed
        } else {
            GpuTimingStatus::Pending
        }
    }

    fn bytes(&self) -> u64 {
        self.passes.len() as u64 * 2 * QUERY_BYTES
    }

    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.status() != GpuTimingStatus::Pending {
            return;
        }
        let Ok(count) = u32::try_from(self.passes.len()) else {
            return;
        };

        let buffers = &self.buffers;
        encoder.resolve_query_set(&buffers.queries, 0..count * 2, &buffers.resolve, 0);
        encoder.copy_buffer_to_buffer(&buffers.resolve, 0, &buffers.read, 0, self.bytes());
    }

    fn map(&self) -> ReadbackState {
        let (sender, completed) = mpsc::sync_channel(1);
        self.buffers
            .read
            .slice(..self.bytes())
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.try_send(result);
            });
        ReadbackState::Mapping(completed)
    }

    fn read(&self, period: f64) -> Option<GpuFrameTiming> {
        let slice = self.buffers.read.slice(..self.bytes());
        let timing = slice
            .get_mapped_range()
            .ok()
            .and_then(|data| decode(&data, &self.passes, period));
        self.buffers.read.unmap();
        timing
    }
}

fn decode(data: &[u8], metadata: &[Pass], period: f64) -> Option<GpuFrameTiming> {
    let stamps = bytemuck::try_cast_slice::<_, u64>(data).ok()?;
    // Drivers may overlap or reorder passes. Use the earliest and latest
    // samples for the envelope, and reject intervals that wrap.
    let (pairs, remainder) = stamps.as_chunks::<2>();
    if !remainder.is_empty() || pairs.len() != metadata.len() || period <= 0.0 {
        return None;
    }
    let first = *stamps.iter().min()?;
    let last = *stamps.iter().max()?;
    if last == 0 {
        return None;
    }
    let mut passes = Vec::with_capacity(metadata.len());
    for (pass, pair) in metadata.iter().zip(pairs) {
        let duration = pair[1].checked_sub(pair[0])?;
        passes.push(GpuPassTiming {
            kind: pass.kind,
            source: pass.source.clone(),
            slice_index: pass.slice_index,
            draws: pass.draws,
            start_us: (pair[0] - first) as f64 * period,
            duration_us: duration as f64 * period,
        });
    }
    Some(GpuFrameTiming {
        clock_start_us: first as f64 * period,
        frame_span_us: (last - first) as f64 * period,
        passes_sum_us: passes.iter().map(|pass| pass.duration_us).sum(),
        passes,
    })
}
