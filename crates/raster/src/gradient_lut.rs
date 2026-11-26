//! Optional device-local ramp atlas for long, smoothly interpolated gradients.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::gpu::Gpu;

const WIDTH: u32 = 256;
const SIZE: usize = WIDTH as usize;
const LAYERS: u32 = 64;
const MAX_ERROR: f32 = 0.002;
type Ramp = [[u16; 4]; SIZE];
type RampCache = HashMap<Box<[[u32; 4]]>, Weak<PreparedRamp>>;
const PREPARED_RAMP_LIMIT: usize = 1024;

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("IMPASTO_GRADIENT_LUT").as_deref() == Ok("1"))
}

#[derive(Debug)]
pub(crate) struct PreparedRamp {
    id: u64,
    pixels: Ramp,
}

impl PreparedRamp {
    pub fn new(pairs: &[[f32; 4]]) -> Option<Arc<Self>> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        static CACHE: OnceLock<Mutex<RampCache>> = OnceLock::new();
        if !enabled() || pairs.len() < 16 {
            return None;
        }
        let cache = CACHE.get_or_init(Mutex::default);
        let key: &[[u32; 4]] = bytemuck::cast_slice(pairs);
        if let Some(ramp) = cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(key)?.upgrade())
        {
            return Some(ramp);
        }
        let ramp = Arc::new(Self {
            pixels: bake(pairs)?,
            id: NEXT.fetch_add(1, Ordering::Relaxed),
        });
        if let Ok(mut cache) = cache.lock() {
            if let Some(existing) = cache.get(key).and_then(Weak::upgrade) {
                return Some(existing);
            }
            if cache.len() >= PREPARED_RAMP_LIMIT {
                cache.retain(|_, ramp| ramp.strong_count() != 0);
            }
            if cache.len() < PREPARED_RAMP_LIMIT {
                cache.insert(key.into(), Arc::downgrade(&ramp));
            }
        }
        Some(ramp)
    }
}

impl PartialEq for PreparedRamp {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id || self.pixels == other.pixels
    }
}

pub(crate) struct GradientLut {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    layers: u32,
    used_layers: u32,
    overflowed: bool,
    ramps: HashMap<u64, u32>,
    pub upload_bytes: u64,
    pub hits: u32,
}

impl GradientLut {
    pub fn new(gpu: &Gpu) -> Self {
        let enabled = gpu.gradient_lut;
        let layers = if enabled {
            LAYERS.min(gpu.caps.max_array_layers)
        } else {
            1
        };
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("impasto gradient ramps"),
            size: wgpu::Extent3d {
                width: if enabled { WIDTH } else { 1 },
                height: 1,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        Self {
            texture,
            view,
            layers,
            used_layers: 0,
            overflowed: false,
            ramps: HashMap::new(),
            upload_bytes: 0,
            hits: 0,
        }
    }

    pub fn reset(&mut self) {
        self.upload_bytes = 0;
        self.hits = 0;
        if self.overflowed {
            // All prior slices have been submitted or discarded. Queue writes
            // in the next frame can now replace these layers safely.
            self.trim();
        }
    }

    pub fn trim(&mut self) {
        self.ramps.clear();
        self.used_layers = 0;
        self.overflowed = false;
    }

    pub fn layer(&mut self, gpu: &Gpu, ramp: Option<&PreparedRamp>) -> u32 {
        let Some(ramp) = ramp else {
            return 0;
        };
        if let Some(&layer) = self.ramps.get(&ramp.id) {
            self.hits += 1;
            return layer;
        }
        let layer = self.used_layers;
        if layer == self.layers {
            self.overflowed = true;
            return 0;
        }
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&ramp.pixels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let id = layer + 1;
        self.ramps.insert(ramp.id, id);
        self.used_layers += 1;
        self.upload_bytes += size_of::<Ramp>() as u64;
        id
    }
}

fn bake(pairs: &[[f32; 4]]) -> Option<Ramp> {
    let (stops, remainder) = pairs.as_chunks::<2>();
    if !remainder.is_empty()
        || stops
            .windows(2)
            .any(|pair| pair[1][1][0] - pair[0][1][0] <= 2.0 / (SIZE - 1) as f32)
    {
        return None;
    }
    let ideal = |t: f32| {
        let i = stops
            .partition_point(|stop| stop[1][0] < t)
            .saturating_sub(1);
        let [a, start] = stops[i];
        let [b, end] = stops[(i + 1).min(stops.len() - 1)];
        let f = ((t - start[0]) / (end[0] - start[0]).max(1e-6)).clamp(0.0, 1.0);
        std::array::from_fn::<_, 4, _>(|i| a[i] * (1.0 - f) + b[i] * f)
    };
    let ramp = std::array::from_fn(|i| {
        ideal(i as f32 / (SIZE - 1) as f32).map(|v| half::f16::from_f32(v).to_bits())
    });
    // The difference of the two piecewise-linear ramps is linear between
    // any two knots. Test both sets of knots to bound the maximum error.
    let within = |t: f32| {
        let position = t.clamp(0.0, 1.0) * (SIZE - 1) as f32;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "bounded ramp coordinate"
        )]
        let index = position as usize;
        let f = position.fract();
        let [a, b] = [ramp[index], ramp[(index + 1).min(SIZE - 1)]]
            .map(|entry| entry.map(|value| half::f16::from_bits(value).to_f32()));
        ideal(t)
            .into_iter()
            .enumerate()
            .all(|(i, target)| (a[i] * (1.0 - f) + b[i] * f - target).abs() <= MAX_ERROR)
    };
    (stops.iter().all(|stop| within(stop[1][0]))
        && (0..SIZE).all(|i| within(i as f32 / (SIZE - 1) as f32)))
    .then_some(ramp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramps_accept_smooth_stops_and_keep_discontinuities_exact() {
        let mut stops = [[0.0; 4]; 16];
        for (i, pair) in stops.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let value = i as f32 / 7.0;
            *pair = [[value; 4], [value, 0.0, 0.0, 0.0]];
        }
        assert!(bake(&stops).is_some());
        if enabled() {
            let first = PreparedRamp::new(&stops).unwrap();
            let second = PreparedRamp::new(&stops).unwrap();
            assert!(Arc::ptr_eq(&first, &second));
        }
        stops[3][0] = 0.0;
        assert!(bake(&stops).is_none());
    }
}
