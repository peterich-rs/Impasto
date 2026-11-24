//! Native producers render into textures allocated on the current raster device.

use std::collections::HashMap;
use std::sync::Arc;

use engine::scene::image::GPU_VIEW_TEXTURE_PREFIX;
use engine::scene::{Command, LayerTree, PictureId};

use crate::compositor::extent;
use crate::gpu::Gpu;
use crate::profiler::{Capture, GpuPassKind, GpuPassSource};
use crate::{GpuImage, PixelSize, Status};

const MAX_PRODUCERS: usize = 32;
const MAX_TEXTURE_BYTES: u64 = 64 * 1024 * 1024;

/// An application-scoped producer name used by an external GPU view.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GpuViewId(Arc<str>);

impl GpuViewId {
    /// Creates a name using 1–64 ASCII letters, digits, dots, underscores, or dashes.
    ///
    /// # Errors
    /// Returns an error when the name is outside this grammar.
    pub fn new(id: &str) -> Result<Self, GpuViewError> {
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(GpuViewError::InvalidId);
        }
        Ok(Self(Arc::from(format!("{GPU_VIEW_TEXTURE_PREFIX}{id}"))))
    }

    /// The image address used by a scene sampling this producer.
    pub fn source(&self) -> &str {
        &self.0
    }
}

/// Working format for one producer's linear-premultiplied output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuViewFormat {
    /// SDR output, with the render target applying the sRGB transfer function.
    #[default]
    Srgb,
    /// Linear floating-point output for extended dynamic range.
    LinearFloat,
}

impl GpuViewFormat {
    fn format(self) -> wgpu::TextureFormat {
        match self {
            Self::Srgb => wgpu::TextureFormat::Rgba8UnormSrgb,
            Self::LinearFloat => wgpu::TextureFormat::Rgba16Float,
        }
    }
}

/// Failure to register a native producer or encode its next image.
#[derive(Debug, thiserror::Error)]
pub enum GpuViewError {
    #[error("invalid GPU view producer name")]
    InvalidId,
    #[error("this GPU view producer is already registered")]
    Duplicate,
    #[error("GPU view producer capacity reached")]
    Capacity,
    #[error("{0}")]
    Producer(String),
}

/// Device and frame information borrowed for a native rendering callback.
pub struct GpuContext<'a> {
    gpu: &'a Gpu,
    generation: u64,
    format: wgpu::TextureFormat,
}

impl GpuContext<'_> {
    /// The producer creates pipelines and buffers on the device that samples its texture.
    pub fn device(&self) -> &wgpu::Device {
        &self.gpu.device
    }

    /// The queue accepts resource uploads before the producer returns its command buffer.
    ///
    /// Return draw commands to Impasto so submission stays ordered with native
    /// presentation and external image imports.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.gpu.queue
    }

    /// The generation to which all current device resources belong.
    pub fn device_generation(&self) -> u64 {
        self.generation
    }

    /// The required color attachment format for the provided view.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }
}

/// A native producer owned by the Raster queue. The output is sampled in the same frame.
pub trait GpuViewProducer: Send + 'static {
    /// Selects the working format. The default is an sRGB render target.
    fn format(&self) -> GpuViewFormat {
        GpuViewFormat::Srgb
    }

    /// Encodes the next frame, clearing or fully overwriting the supplied view.
    ///
    /// Return linear-premultiplied colors and use the predicted timestamp in milliseconds.
    ///
    /// # Errors
    /// A producer error omits the view for this frame. The rest of the scene still renders.
    fn prepare_or_render(
        &mut self,
        ctx: &GpuContext<'_>,
        target: &wgpu::TextureView,
        size: PixelSize,
        time_ms: f64,
    ) -> Result<wgpu::CommandBuffer, GpuViewError>;

    /// Releases resources from the lost device before the next rendering callback.
    fn on_device_lost(&mut self) {}
}

struct Producer {
    backend: Box<dyn GpuViewProducer>,
    target: Option<GpuImage>,
    last_error: Option<String>,
}

#[derive(Default)]
pub(crate) struct GpuViews {
    producers: HashMap<Arc<str>, Producer>,
    timing_target: Option<wgpu::TextureView>,
}

pub(crate) type Requests = Vec<(Arc<str>, PixelSize)>;

impl GpuViews {
    pub fn register(
        &mut self,
        id: GpuViewId,
        producer: impl GpuViewProducer,
    ) -> Result<(), GpuViewError> {
        if self.producers.contains_key(&id.0) {
            return Err(GpuViewError::Duplicate);
        }
        if self.producers.len() >= MAX_PRODUCERS {
            return Err(GpuViewError::Capacity);
        }
        self.producers.insert(
            id.0,
            Producer {
                backend: Box::new(producer),
                target: None,
                last_error: None,
            },
        );
        Ok(())
    }

    pub fn device_lost(&mut self) {
        self.timing_target = None;
        for entry in self.producers.values_mut() {
            entry.target = None;
            entry.last_error = None;
            entry.backend.on_device_lost();
        }
    }

    pub fn requests(
        &self,
        tree: &LayerTree,
        scales: &HashMap<PictureId, f32>,
        point_scale: f32,
    ) -> Requests {
        if self.producers.is_empty() {
            return Vec::new();
        }
        let mut requests = Vec::<(Arc<str>, PixelSize)>::new();
        for picture in &tree.pictures {
            if !picture
                .images()
                .iter()
                .any(|(source, _)| self.producers.contains_key(source))
            {
                continue;
            }
            let scale = scales.get(&picture.id).copied().unwrap_or(point_scale);
            for command in picture.commands().iter() {
                let mut request = |source: &Arc<str>, width: f32, height: f32| {
                    if !self.producers.contains_key(source) || !(width > 0.0 && height > 0.0) {
                        return;
                    }
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "round up to whole pixels before clamping to the device's texture limit"
                    )]
                    let pixels = |dimension: f32| (dimension * scale).ceil().max(1.0) as u32;
                    let size = PixelSize::new(pixels(width), pixels(height));
                    if let Some((_, previous)) = requests.iter_mut().find(|(id, _)| id == source) {
                        previous.width = previous.width.max(size.width);
                        previous.height = previous.height.max(size.height);
                    } else {
                        requests.push((Arc::clone(source), size));
                    }
                };
                match command {
                    Command::Image { source, w, h, .. } => request(source, *w, *h),
                    Command::ShaderEffect {
                        bounds, textures, ..
                    } => {
                        for source in textures.iter() {
                            request(source, bounds[2], bounds[3]);
                        }
                    }
                    _ => {}
                }
            }
        }
        requests
    }

    pub fn render(
        &mut self,
        gpu: &Gpu,
        generation: u64,
        requests: &Requests,
        time_ms: f64,
        mut capture: Option<&mut Capture>,
        frame_commands: &mut Vec<wgpu::CommandBuffer>,
    ) -> HashMap<Arc<str>, GpuImage> {
        for (source, entry) in &mut self.producers {
            if entry.target.is_some() && !requests.iter().any(|(used, _)| used == source) {
                entry.target = None;
            }
        }
        let mut remaining = MAX_TEXTURE_BYTES;
        let mut views = HashMap::with_capacity(requests.len());
        for (source, requested) in requests {
            let Some(entry) = self.producers.get_mut(source) else {
                continue;
            };
            let mut format = entry.backend.format().format();
            if format == wgpu::TextureFormat::Rgba16Float
                && !gpu
                    .adapter
                    .get_texture_format_features(format)
                    .flags
                    .contains(wgpu::TextureFormatFeatureFlags::FILTERABLE)
            {
                format = GpuViewFormat::Srgb.format();
            }
            let max = gpu.max_texture_size();
            let size = PixelSize::new(requested.width.min(max), requested.height.min(max));
            let channels = if format == wgpu::TextureFormat::Rgba16Float {
                8
            } else {
                4
            };
            let bytes = u64::from(size.width) * u64::from(size.height) * channels;
            if bytes > remaining {
                entry.target = None;
                entry.report(source, "GPU view texture budget exceeded".to_owned());
                continue;
            }
            remaining -= bytes;
            let context = GpuContext {
                gpu,
                generation,
                format,
            };
            let _span = tracing::debug_span!(target: "impasto::effect", "gpu_view_render",
                view_id = source.as_ref(), width = size.width, height = size.height)
            .entered();
            let cold = entry.target.as_ref().is_none_or(|target| {
                target.width != size.width
                    || target.height != size.height
                    || target.device != generation
                    || target.view().texture().format() != context.format
            });
            let mut producer_error = None;
            let render = || {
                if cold {
                    entry.target = Some(make_target(&context, size));
                }
                let target = entry.target.as_ref().ok_or(Status::NotReady)?;
                let commands = entry
                    .backend
                    .prepare_or_render(&context, target.view(), size, time_ms)
                    .map_err(|error| {
                        producer_error = Some(error.to_string());
                        Status::PresentFailed
                    })?;
                if cold {
                    // A submit-time error on first use must leave the compositor's
                    // command buffers available to render the rest of the scene.
                    gpu.submit(timed_commands(
                        gpu,
                        &mut self.timing_target,
                        capture.as_deref_mut(),
                        source,
                        commands,
                    ));
                    Ok(None)
                } else {
                    Ok(Some(commands))
                }
            };
            let rendered = if cold {
                gpu.validation(render)
            } else {
                gpu.hot_validation(render)
            };
            match rendered {
                Ok(commands) => {
                    if let Some(commands) = commands {
                        frame_commands.extend(timed_commands(
                            gpu,
                            &mut self.timing_target,
                            capture.as_deref_mut(),
                            source,
                            commands,
                        ));
                    }
                    entry.last_error = None;
                    if let Some(target) = &entry.target {
                        views.insert(Arc::clone(source), target.clone());
                    }
                }
                Err(error) => {
                    entry.target = None;
                    entry.report(source, producer_error.unwrap_or_else(|| error.to_string()));
                }
            }
        }
        views
    }
}

impl Producer {
    fn report(&mut self, source: &str, error: String) {
        if self.last_error.as_ref() != Some(&error) {
            tracing::warn!(target: "impasto::effect", view_id = source, %error, "GPU view frame omitted");
            self.last_error = Some(error);
        }
    }
}

fn make_target(context: &GpuContext<'_>, size: PixelSize) -> GpuImage {
    let texture = context.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("impasto native GPU view"),
        size: extent([size.width, size.height]),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: context.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    GpuImage::from_rendered(texture, context.generation)
}

fn timed_commands(
    gpu: &Gpu,
    timing_target: &mut Option<wgpu::TextureView>,
    capture: Option<&mut Capture>,
    source: &Arc<str>,
    commands: wgpu::CommandBuffer,
) -> impl Iterator<Item = wgpu::CommandBuffer> {
    let Some(stamps) = capture.and_then(|capture| {
        capture.tagged_pass(
            GpuPassKind::GpuView,
            None,
            0,
            Some(GpuPassSource::GpuView(Arc::clone(source))),
        )
    }) else {
        return [None, Some(commands), None].into_iter().flatten();
    };
    // Bracket producers with a disposable one-pixel attachment so timestamp
    // passes never load or store tiles of the producer's image.
    let target = timing_target.get_or_insert_with(|| {
        gpu.device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("impasto GPU view timing attachment"),
                size: extent([1, 1]),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    });
    [
        Some(marker(gpu, target, &stamps, true)),
        Some(commands),
        Some(marker(gpu, target, &stamps, false)),
    ]
    .into_iter()
    .flatten()
}

fn marker(
    gpu: &Gpu,
    texture: &wgpu::TextureView,
    stamps: &wgpu::RenderPassTimestampWrites<'_>,
    start: bool,
) -> wgpu::CommandBuffer {
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("impasto GPU view timing"),
        });
    let timestamp_writes = wgpu::RenderPassTimestampWrites {
        query_set: stamps.query_set,
        beginning_of_pass_write_index: if start {
            stamps.beginning_of_pass_write_index
        } else {
            None
        },
        end_of_pass_write_index: if start {
            None
        } else {
            stamps.end_of_pass_write_index
        },
    };
    drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("impasto GPU view timing"),
        timestamp_writes: Some(timestamp_writes),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: texture,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Discard,
            },
        })],
        ..Default::default()
    }));
    encoder.finish()
}
