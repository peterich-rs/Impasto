//! Optional device-local ramp atlas for long, smoothly interpolated gradients.

use std::collections::HashMap;

use crate::gpu::Gpu;

const WIDTH: u32 = 256;
const SIZE: usize = WIDTH as usize;
const LAYERS: u32 = 64;
const MAX_ERROR: f32 = 0.002;
type Ramp = [[u16; 4]; SIZE];

pub(crate) struct GradientLut {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    enabled: bool,
    layers: u32,
    ramps: HashMap<Vec<[u32; 4]>, u32>,
    pub upload_bytes: u64,
    pub hits: u32,
}

impl GradientLut {
    pub fn new(gpu: &Gpu) -> Self {
        let enabled = std::env::var("IMPASTO_GRADIENT_LUT").as_deref() == Ok("1");
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
            enabled,
            layers,
            ramps: HashMap::new(),
            upload_bytes: 0,
            hits: 0,
        }
    }

    pub fn reset(&mut self) {
        self.upload_bytes = 0;
        self.hits = 0;
        if self.ramps.len() as u64 == u64::from(self.layers) {
            // All prior slices have been submitted or discarded. Queue writes
            // in the next frame can now replace these layers safely.
            self.ramps.clear();
        }
    }

    pub fn trim(&mut self) {
        self.ramps.clear();
    }

    pub fn layer(&mut self, gpu: &Gpu, pairs: &[[f32; 4]]) -> u32 {
        if !self.enabled || pairs.len() < 16 {
            return 0;
        }
        let key = pairs
            .iter()
            .map(|pair| pair.map(f32::to_bits))
            .collect::<Vec<_>>();
        if let Some(&id) = self.ramps.get(&key) {
            self.hits += 1;
            return id;
        }
        let layer = u32::try_from(self.ramps.len()).unwrap_or(self.layers);
        if layer == self.layers {
            return 0;
        }
        let Some(ramp) = bake(pairs) else { return 0 };
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
            bytemuck::cast_slice(&ramp),
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
        self.ramps.insert(key, id);
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
        stops[3][0] = 0.0;
        assert!(bake(&stops).is_none());
    }
}
