//! Reconstructs compact image placeholders into textures owned by the image cache.

use base64::Engine as _;
use engine::scene::image::{BLURHASH_PREFIX, THUMBHASH_PREFIX};

use crate::{GpuImage, compositor::extent, gpu::Gpu, pipelines};

const ALPHABET: &[u8] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz#$%*+,-.:;=?@[]^_{|}~";
const MAX_FACTORS: usize = 9 * 9;
const MAX_SIDE: u32 = 32;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

struct ImageFactors {
    size: [u32; 2],
    // First vector is the coefficient grid dimensions and the encoded-color flag.
    data: [[f32; 4]; 1 + MAX_FACTORS],
}

fn base83(digits: &[u8]) -> Option<u32> {
    digits.iter().try_fold(0, |value, digit| {
        let index = ALPHABET.iter().position(|symbol| symbol == digit)?;
        Some(value * 83 + u32::try_from(index).ok()?)
    })
}

fn blurhash(hash: &str) -> Option<ImageFactors> {
    // https://github.com/woltapp/blurhash/blob/master/Algorithm.md
    let hash = hash.as_bytes();
    let size = base83(hash.get(..1)?)?;
    let width = usize::try_from(1 + size % 9).ok()?;
    let height = usize::try_from(1 + size / 9).ok()?;
    let count = width * height;
    if count > MAX_FACTORS || hash.len() != 4 + 2 * count {
        return None;
    }
    let peak = (base83(&hash[1..2])? + 1) as f32 / 166.0;
    let dc = base83(&hash[2..6])?;
    if dc > 0x00ff_ffff {
        return None;
    }
    let [r, g, b] =
        [16, 8, 0].map(|shift| crate::color::srgb_to_linear(dc.to_le_bytes()[shift / 8]));
    let mut factors = [[0.0; 4]; 1 + MAX_FACTORS];
    factors[0] = [width as f32, height as f32, 0.0, 0.0];
    factors[1] = [r, g, b, 1.0];
    let (pairs, _) = hash[6..].as_chunks::<2>();
    for (slot, pair) in factors[2..].iter_mut().zip(pairs) {
        let ac = base83(pair)?;
        if ac >= 19 * 19 * 19 {
            return None;
        }
        let [r, g, b] = [(ac / (19 * 19)), (ac / 19) % 19, ac % 19].map(|channel| {
            let normalized = (channel as f32 - 9.0) / 9.0;
            normalized * normalized.abs() * peak
        });
        *slot = [r, g, b, 0.0];
    }
    Some(ImageFactors {
        size: [MAX_SIDE; 2],
        data: factors,
    })
}

fn thumbhash(hash: &str) -> Option<ImageFactors> {
    // https://evanw.github.io/thumbhash/
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};

    if hash.len() > 40 {
        return None;
    }
    let bytes = [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD]
        .into_iter()
        .find_map(|encoding| encoding.decode(hash).ok())?;
    let [low, middle, high, scale_low, scale_high, rest @ ..] = bytes.as_slice() else {
        return None;
    };
    let color = u32::from_le_bytes([*low, *middle, *high, 0]);
    let chroma = u16::from_le_bytes([*scale_low, *scale_high]);
    let has_alpha = color & (1 << 23) != 0;
    let landscape = chroma & (1 << 15) != 0;
    let long = if has_alpha { 5 } else { 7 };
    let short = chroma & 7;
    if short == 0 || short > long {
        return None;
    }
    let (alpha, ac) = if has_alpha {
        let (&alpha, ac) = rest.split_first()?;
        (alpha, ac)
    } else {
        (15, rest)
    };
    let (width, height) = if landscape {
        (long, short)
    } else {
        (short, long)
    };
    let long = u32::from(long);
    let [width_px, height_px] =
        [width, height].map(|side| (MAX_SIDE * u32::from(side) + long / 2) / long);
    let (lx, ly) = (usize::from(width.max(3)), usize::from(height.max(3)));
    let alpha_grid = if has_alpha { 5 } else { 0 };
    let grid = (lx.max(alpha_grid), ly.max(alpha_grid));
    let mut factors = [[0.0; 4]; 1 + MAX_FACTORS];
    factors[0] = [grid.0 as f32, grid.1 as f32, 1.0, 0.0];
    factors[1] = [
        (color & 63) as f32 / 63.0,
        ((color >> 6) & 63) as f32 / 31.5 - 1.0,
        ((color >> 12) & 63) as f32 / 31.5 - 1.0,
        f32::from(alpha & 15) / 15.0,
    ];
    let channels = [
        (lx, ly, ((color >> 18) & 31) as f32 / 31.0),
        (3, 3, f32::from((chroma >> 3) & 63) * 1.25 / 63.0),
        (3, 3, f32::from((chroma >> 9) & 63) * 1.25 / 63.0),
        (alpha_grid, alpha_grid, f32::from(alpha >> 4) / 15.0),
    ];
    let mut nibbles = ac.iter().flat_map(|byte| [byte & 15, byte >> 4]);
    let mut read = 0usize;
    for (channel, (nx, ny, scale)) in channels.into_iter().enumerate() {
        for y in 0..ny {
            for x in usize::from(y == 0)..nx {
                if x * ny >= nx * (ny - y) {
                    break;
                }
                let value = (f32::from(nibbles.next()?) / 7.5 - 1.0) * 2.0 * scale;
                factors[1 + y * grid.0 + x][channel] = value;
                read += 1;
            }
        }
    }
    if read.div_ceil(2) != ac.len() {
        return None;
    }
    Some(ImageFactors {
        size: [width_px, height_px],
        data: factors,
    })
}

pub(crate) fn prepare(
    gpu: &Gpu,
    device: u64,
    source: &str,
) -> Option<(GpuImage, wgpu::CommandBuffer)> {
    let ImageFactors { size, data } = if let Some(hash) = source.strip_prefix(BLURHASH_PREFIX) {
        blurhash(hash)?
    } else {
        thumbhash(source.strip_prefix(THUMBHASH_PREFIX)?)?
    };
    let pipeline = gpu
        .inline_image_pipeline
        .get_or_init(|| gpu.validation(|| Ok(create_pipeline(gpu))).ok())
        .as_ref()?;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("impasto inline image texture"),
        size: extent(size),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut image = GpuImage::from_rendered(texture, device);
    image.upload.render_passes = 1;
    image.upload.resident_bytes = u64::from(size[0] * size[1] * 4);
    let buffer = pipelines::buffer_from(
        gpu,
        "impasto inline image factors",
        bytemuck::cast_slice(&data),
        wgpu::BufferUsages::UNIFORM,
    )
    .ok()?;
    let group = pipelines::bind_group(
        gpu,
        &pipeline.get_bind_group_layout(0),
        &[buffer.as_entire_binding()],
    );
    let mut command = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("impasto inline image reconstruction"),
        });
    {
        let mut pass = command.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("impasto inline image"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: image.view(),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
    Some((image, command.finish()))
}

fn create_pipeline(gpu: &Gpu) -> wgpu::RenderPipeline {
    let shader = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("impasto inline image shader"),
            source: wgpu::ShaderSource::Wgsl(pipelines::INLINE_IMAGE.into()),
        });
    gpu.device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("impasto inline image"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(FORMAT.into())],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: gpu.pipeline_cache(),
        })
}
