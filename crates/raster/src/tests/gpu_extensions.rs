use super::{headless, pixel, rect, tree};
use crate::{
    FontLibrary, GpuContext, GpuPassKind, GpuViewError, GpuViewId, GpuViewProducer, PixelSize,
    RasterOptions, Rasterizer,
};
use engine::motion::{Easing, MotionCurve};
use engine::scene::{
    BACKDROP_TEXTURE, Command, CompositorProp, CompositorTrack, CompositorTrackId, ContentMode,
    LayerId, ShaderProgram, ShaderProgramId,
};
use std::sync::{Arc, Mutex};

fn program(id: u64, source: &str, animated: bool) -> ShaderProgram {
    ShaderProgram {
        id: ShaderProgramId(id),
        source: Arc::from(source),
        animated,
    }
}

fn effect(id: u64, bounds: [f32; 4], uniforms: &[u8], textures: &[&str]) -> Command {
    Command::ShaderEffect {
        bounds,
        shader: ShaderProgramId(id),
        uniforms: Arc::from(uniforms),
        textures: textures.iter().map(|source| Arc::from(*source)).collect(),
    }
}

#[test]
fn shader_effect_composites_inputs_and_degrades_on_error() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let mut scene = tree(
        1,
        vec![
            rect([0.0, 0.0, 32.0, 8.0], [255, 0, 0, 255]),
            Command::PushClip {
                x: 1.0,
                y: 0.0,
                w: 8.0,
                h: 8.0,
                corner_radius: 4.0,
            },
            effect(1, [1.0, 0.0, 8.0, 8.0], &0.5_f32.to_le_bytes(), &["blue"]),
            Command::Pop,
            effect(2, [11.0, 0.0, 8.0, 8.0], &[], &[BACKDROP_TEXTURE]),
            effect(3, [22.0, 0.0, 8.0, 8.0], &[], &[]),
        ],
    );
    scene.shader_programs = Arc::from([
        program(
            1,
            "
            fn effect_main(p: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
                let sample = effect_sample(0, p / size);
                return vec4(sample.rgb, effect_uniform(0));
            }",
            false,
        ),
        program(
            2,
            "
            fn effect_main(p: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
                let sample = effect_sample(0, p / size);
                return sample.bgra;
            }",
            false,
        ),
        program(
            3,
            "
            @group(3) @binding(0) var<storage, read_write> writes: array<u32>;
            fn effect_main(p: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
                return vec4<f32>(0.0, 1.0, 0.0, 1.0);
            }",
            false,
        ),
    ]);
    raster
        .submit_layers(&scene, 1.0, |source| {
            (source == "blue").then_some(crate::ImagePixels {
                width: 1,
                height: 1,
                rgba: &[0, 0, 255, 255],
            })
        })
        .unwrap();
    let output = raster.render_rgba(PixelSize::new(32, 8)).unwrap();
    assert_eq!(pixel(&output, 32, 1, 0), [255, 0, 0, 255], "rounded clip");
    let mix = pixel(&output, 32, 5, 4);
    assert!(
        (186..=190).contains(&mix[0]) && (186..=190).contains(&mix[2]),
        "linear blend {mix:?}"
    );
    assert_eq!(
        pixel(&output, 32, 14, 4),
        [0, 0, 255, 255],
        "backdrop sampled beneath effect"
    );
    assert_eq!(
        pixel(&output, 32, 25, 4),
        [255, 0, 0, 255],
        "invalid effect leaves previous color"
    );
    let failures = raster.take_effect_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].shader, ShaderProgramId(3));
    raster.render_rgba(PixelSize::new(32, 8)).unwrap();
    assert!(
        raster.take_effect_failures().is_empty(),
        "the diagnostic is emitted once on this device"
    );
}

#[test]
fn animated_shader_samples_the_raster_clock_without_new_pictures() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let mut scene = tree(1, [effect(4, [0.0, 0.0, 4.0, 4.0], &[], &[])]);
    scene.shader_programs = Arc::from([program(
        4,
        "
        fn effect_main(p: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
            return vec4(effect_time(), 0.0, 0.0, 1.0);
        }",
        true,
    )]);
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    raster.set_render_time_ms(18_000.0);
    let first = raster.render_rgba(PixelSize::new(4, 4)).unwrap();
    raster.set_render_time_ms(18_500.0);
    let second = raster.render_rgba(PixelSize::new(4, 4)).unwrap();
    assert_eq!(pixel(&first, 4, 2, 2), [0, 0, 0, 255]);
    let later = pixel(&second, 4, 2, 2);
    assert!(
        (186..=190).contains(&later[0]),
        "half a second of raster clock {later:?}"
    );
    assert_eq!(raster.stats.render.bind_groups_created, 0);
}

#[test]
fn runtime_effect_adopts_an_image_that_arrives_after_scene_submission() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let mut scene = tree(1, [effect(4, [0.0, 0.0, 4.0, 4.0], &[], &["late"])]);
    scene.shader_programs = Arc::from([program(
        4,
        "
        fn effect_main(p: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
            return effect_sample(0, p / size);
        }",
        false,
    )]);
    let pixels = PixelSize::new(4, 4);
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    assert_eq!(pixel(&raster.render_rgba(pixels).unwrap(), 4, 2, 2), [0; 4]);
    let image = raster
        .texture_factory()
        .unwrap()
        .upload_rgba(1, 1, &[0, 255, 0, 255])
        .unwrap();
    raster.install_uploaded("late", &image);
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    assert_eq!(
        pixel(&raster.render_rgba(pixels).unwrap(), 4, 2, 2),
        [0, 255, 0, 255]
    );
}

#[test]
fn scalar_track_overrides_uniform_on_its_layer() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let mut scene = tree(1, [effect(7, [0.0, 0.0, 4.0, 4.0], &[], &[])]);
    scene.shader_programs = Arc::from([program(
        7,
        "
        fn effect_main(p: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
            return vec4(0.0, effect_uniform(3), 0.0, 1.0);
        }",
        false,
    )]);
    scene.tracks = Arc::from([CompositorTrack::new(
        CompositorTrackId(1),
        LayerId(0),
        CompositorProp::ShaderUniform(3),
        0.0,
        MotionCurve::Tween {
            from: 0.0,
            to: 1.0,
            duration_ms: 64.0,
            easing: Easing::Linear,
        },
        None,
        None,
    )]);
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    assert_eq!(
        raster
            .sample_compositor_tracks(scene.epoch, 16.0)
            .unwrap()
            .samples[0]
            .value,
        0.25
    );
    let output = raster.render_rgba(PixelSize::new(4, 4)).unwrap();
    assert!((135..=139).contains(&pixel(&output, 4, 2, 2)[1]));
}

#[derive(Default)]
struct Observation {
    frames: Vec<(u64, PixelSize)>,
    losses: usize,
}

struct Producer(Arc<Mutex<Observation>>);

impl GpuViewProducer for Producer {
    fn prepare_or_render(
        &mut self,
        ctx: &GpuContext<'_>,
        target: &wgpu::TextureView,
        size: PixelSize,
        time_ms: f64,
    ) -> Result<wgpu::CommandBuffer, GpuViewError> {
        self.0
            .lock()
            .unwrap()
            .frames
            .push((ctx.device_generation(), size));
        let mut commands = ctx.device().create_command_encoder(&Default::default());
        let color = if time_ms < 100.0 {
            wgpu::Color {
                r: 0.0,
                g: 1.0,
                b: 0.0,
                a: 1.0,
            }
        } else {
            wgpu::Color {
                r: 0.0,
                g: 0.0,
                b: 1.0,
                a: 1.0,
            }
        };
        drop(commands.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        }));
        Ok(commands.finish())
    }

    fn on_device_lost(&mut self) {
        self.0.lock().unwrap().losses += 1;
    }
}

#[test]
fn gpu_view_composes_without_upload_and_recreates_after_device_loss() {
    let Some(device) = headless(FontLibrary::new()) else {
        return;
    };
    let mut raster = Rasterizer::headless(
        RasterOptions {
            validation: crate::ValidationMode::AsyncOnly,
            ..device.options
        },
        FontLibrary::new(),
    )
    .unwrap();
    let obs = Arc::new(Mutex::new(Observation::default()));
    let id = GpuViewId::new("test").unwrap();
    let source = Arc::from(id.source());
    raster
        .register_gpu_view(id, Producer(Arc::clone(&obs)))
        .unwrap();
    let scene = |generation, width| {
        tree(
            generation,
            vec![
                Command::PushClip {
                    x: 0.0,
                    y: 0.0,
                    w: width,
                    h: 12.0,
                    corner_radius: 5.0,
                },
                Command::PushOpacity {
                    x: 0.0,
                    y: 0.0,
                    w: width,
                    h: 12.0,
                    alpha: 128,
                },
                Command::Image {
                    x: 0.0,
                    y: 0.0,
                    w: width,
                    h: 12.0,
                    source: Arc::clone(&source),
                    mode: ContentMode::Fill,
                    required: false,
                },
                Command::Pop,
                Command::Pop,
            ],
        )
    };
    raster
        .submit_layers(&scene(1, 12.0), 1.0, |_| {
            panic!("GPU views never use the CPU image resolver")
        })
        .unwrap();
    assert!(raster.has_registered_gpu_views_in_scene());
    let output = raster.render_rgba(PixelSize::new(24, 24)).unwrap();
    assert_eq!(
        pixel(&output, 24, 0, 0),
        [0; 4],
        "native content obeys rounded clipping"
    );
    assert_eq!(pixel(&output, 24, 6, 6), [0, 255, 0, 128]);
    assert_eq!(raster.submit_stats().image_upload.source_bytes, 0);
    let old_generation = obs.lock().unwrap().frames[0].0;

    raster.set_render_time_ms(100.0);
    let output = raster.render_rgba(PixelSize::new(24, 24)).unwrap();
    assert_eq!(pixel(&output, 24, 6, 6), [0, 0, 255, 128]);
    assert_eq!(obs.lock().unwrap().frames[1].1, PixelSize::new(12, 12));

    raster
        .submit_layers(&scene(2, 16.0), 1.0, |_| None)
        .unwrap();
    let output = raster.render_rgba(PixelSize::new(24, 24)).unwrap();
    assert_eq!(pixel(&output, 24, 13, 6), [0, 0, 255, 128]);
    assert_eq!(obs.lock().unwrap().frames[2].1, PixelSize::new(16, 12));

    raster.discard_device();
    assert_eq!(obs.lock().unwrap().losses, 1);
    let mut fresh = Rasterizer::headless(RasterOptions::default(), FontLibrary::new()).unwrap();
    raster.device = fresh.device.take();
    raster
        .submit_layers(&scene(3, 16.0), 1.0, |_| None)
        .unwrap();
    let output = raster.render_rgba(PixelSize::new(24, 24)).unwrap();
    assert_eq!(pixel(&output, 24, 13, 6), [0, 0, 255, 128]);
    assert_ne!(obs.lock().unwrap().frames[3].0, old_generation);
}

#[test]
fn backdrop_blur_samples_native_gpu_view_pixels() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let id = GpuViewId::new("blur-input").unwrap();
    let source = Arc::from(id.source());
    raster
        .register_gpu_view(id, Producer(Arc::new(Mutex::new(Observation::default()))))
        .unwrap();
    let scene = tree(
        1,
        [
            rect([0.0, 0.0, 16.0, 16.0], [255, 0, 0, 255]),
            Command::Image {
                x: 0.0,
                y: 0.0,
                w: 8.0,
                h: 16.0,
                source,
                mode: ContentMode::Fill,
                required: false,
            },
            Command::PushBackdropBlur {
                x: 4.0,
                y: 0.0,
                w: 8.0,
                h: 16.0,
                sigma: 2.0,
            },
            Command::Pop,
        ],
    );
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    let rgba = raster.render_rgba(PixelSize::new(16, 16)).unwrap();
    let edge = pixel(&rgba, 16, 8, 8);
    assert!(
        edge[0] > 20 && edge[1] > 20,
        "blur crossed the native texture edge {edge:?}"
    );
    assert_eq!(pixel(&rgba, 16, 2, 8), [0, 255, 0, 255]);
}

#[test]
fn gpu_timing_attributes_shader_and_native_producer_work() {
    let Some(device) = headless(FontLibrary::new()) else {
        return;
    };
    let mut raster = Rasterizer::headless(
        RasterOptions {
            enable_gpu_timing: true,
            ..device.options
        },
        FontLibrary::new(),
    )
    .unwrap();
    let id = GpuViewId::new("timing").unwrap();
    let source = Arc::from(id.source());
    raster
        .register_gpu_view(id, Producer(Arc::new(Mutex::new(Observation::default()))))
        .unwrap();
    let mut scene = tree(
        1,
        [
            Command::Image {
                x: 0.0,
                y: 0.0,
                w: 12.0,
                h: 12.0,
                source,
                mode: ContentMode::Fill,
                required: false,
            },
            effect(2, [0.0, 0.0, 6.0, 6.0], &[], &[]),
        ],
    );
    scene.shader_programs = Arc::from([program(
        2,
        "
        fn effect_main(p: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
            return vec4(0.0, 0.2, 0.0, 1.0);
        }",
        false,
    )]);
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    raster.render_rgba(PixelSize::new(16, 16)).unwrap();
    raster.begin_gpu_frame(Some(9));
    raster.render_rgba(PixelSize::new(16, 16)).unwrap();
    if raster.gpu_sample().status == crate::GpuTimingStatus::Unsupported {
        return;
    }
    let mut samples = Vec::new();
    for _ in 0..4 {
        raster
            .device
            .as_ref()
            .unwrap()
            .gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        samples.extend(raster.drain_gpu_timings());
        if !raster.gpu_timings_pending() {
            break;
        }
    }
    let timing = samples
        .into_iter()
        .find_map(|sample| sample.timing)
        .expect("GPU timestamp sample");
    for kind in [GpuPassKind::GpuView, GpuPassKind::ShaderEffect] {
        assert!(
            timing
                .passes
                .iter()
                .any(|pass| pass.kind == kind && pass.duration_us > 0.0),
            "{kind:?}: {:?}",
            timing.passes
        );
    }
    assert!(
        timing
            .passes
            .iter()
            .any(|pass| pass.shader_id() == Some(ShaderProgramId(2)))
    );
    assert!(
        timing
            .passes
            .iter()
            .any(|pass| pass.view_id() == Some("timing"))
    );
}
