use std::collections::HashSet;

use engine::scene::{BoxShadow, ColorStop, Gradient};

use super::*;
use crate::{compositor::extent, surface::read_rgba};

mod benchmark;

fn capture(
    raster: &mut Rasterizer,
    generation: u64,
    commands: Vec<Command>,
    size: PixelSize,
) -> Vec<u8> {
    raster
        .submit_layers(&tree(generation, commands), 1.0, |_| None)
        .unwrap();
    raster.render_rgba(size).unwrap()
}

fn gradient() -> Gradient {
    Gradient::Linear {
        start: [0.0, 0.0],
        end: [1.0, 0.0],
        stops: vec![
            ColorStop::srgb(0.0, [255, 255, 255, 255]),
            ColorStop::srgb(1.0, [0, 0, 255, 255]),
        ],
    }
}

fn rounded(rect: [f32; 4], radius: f32) -> Command {
    let [x, y, w, h] = rect;
    Command::PushClip {
        x,
        y,
        w,
        h,
        corner_radius: radius,
    }
}

fn shadowed(bounds: [f32; 4]) -> Command {
    let mut command = rect(bounds, [255, 0, 0, 255]);
    if let Command::Rect { box_shadows, .. } = &mut command {
        *box_shadows = Arc::from([BoxShadow {
            offset: [12.0, 0.0],
            blur_sigma: 2.0,
            spread: 5.0,
            color: [0, 255, 0, 255],
        }]);
    }
    command
}

#[test]
fn mixed_rectangles_use_one_draw_and_reuse_warm_resources() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let commands: Vec<_> = (0u16..300)
        .map(|index| {
            let bounds = [
                f32::from(index % 30) * 4.0,
                f32::from(index / 30) * 4.0,
                4.0,
                4.0,
            ];
            if index % 3 == 0 {
                return shadowed(bounds);
            }
            let mut command = rect(bounds, [255, 0, 0, 255]);
            if index % 3 == 1
                && let Command::Rect {
                    gradient: paint, ..
                } = &mut command
            {
                *paint = Some(gradient());
            }
            command
        })
        .collect();
    let size = PixelSize::new(120, 40);
    let scene = tree(1, commands);
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    let pixels = raster.render_rgba(size).unwrap();
    let cold = raster.submit_stats().render;
    assert!(cold.direct_present, "{cold:?}");
    assert_eq!((cold.passes, cold.draws), (1, 1));
    // Each shadow contributes a second instance for its rectangle.
    assert_eq!(cold.vertex_bytes, 400 * 64);
    assert_eq!(
        (
            cold.mesh_upload_bytes,
            cold.atlas_upload_bytes,
            cold.copy_pixels
        ),
        (0, 0, 0)
    );
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    assert_eq!(raster.render_rgba(size).unwrap(), pixels);
    let warm = raster.submit_stats().render;
    assert_eq!((warm.buffers_created, warm.bind_groups_created), (0, 0));
    assert_eq!(
        (warm.passes, warm.draws, warm.vertex_bytes),
        (1, 1, cold.vertex_bytes)
    );
}

fn output(gpu: &crate::Gpu, format: wgpu::TextureFormat, size: PixelSize) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test surface"),
        size: extent([size.width, size.height]),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

#[test]
fn srgb_format_fallback_encodes_gamma() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let size = PixelSize::new(9, 9);
    let color = [80, 135, 205, 255];
    let scene = tree(1, [Command::Fill { color }]);
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    let gpu = raster.device.as_ref().unwrap().gpu.clone();
    let format = wgpu::TextureFormat::Bgra8Unorm;
    let target = output(&gpu, format, size);
    raster
        .encode_and_submit(&target.create_view(&Default::default()), format, size)
        .unwrap();
    assert!(!raster.stats.render.direct_present);
    assert_eq!(
        pixel(&read_rgba(&gpu, &target, size).unwrap(), 9, 4, 4),
        [205, 135, 80, 255]
    );
}

#[test]
fn frame_slabs_survive_growing_between_slices() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let mut scene = tree(1, [rect([0.0, 0.0, 12.0, 12.0], [255, 0, 0, 255])]);
    let other = PictureId(2);
    scene.pictures.push(Arc::new(Picture::new(
        other,
        1,
        (0.0, 0.0),
        Arc::from(vec![rect([0.0, 0.0, 12.0, 12.0], [0, 0, 255, 255]); 2300]),
    )));
    scene.layers[0].next = Some(LayerId(1));
    scene.layers.push(Layer {
        kind: LayerKind::Picture { picture: other },
        parent: None,
        first_child: None,
        next: None,
    });
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let size = PixelSize::new(12, 12);
    let device = raster.device.as_mut().unwrap();
    let frame = raster.frame.as_ref().unwrap();
    let gpu = &device.gpu;
    let compositor = &mut device.compositor;
    let targets = [output(gpu, format, size), output(gpu, format, size)];
    compositor.begin_frame();
    gpu.validation(|| {
        let mut commands = Vec::new();
        for (target, id) in targets.iter().zip([LayerId(0), LayerId(1)]) {
            let slice = HashSet::from([id]);
            if !commands.is_empty() {
                compositor.trim();
            }
            commands.push(
                compositor
                    .encode(
                        gpu,
                        frame.scene(&raster.fonts, Some(&slice)),
                        &target.create_view(&Default::default()),
                        format,
                        [12, 12],
                        None,
                    )?
                    .finish(),
            );
        }
        compositor.flush(gpu);
        gpu.queue.submit(commands);
        Ok(())
    })
    .unwrap();
    assert!(compositor.render_stats().buffers_created >= 4);
    assert_eq!(
        pixel(&read_rgba(gpu, &targets[0], size).unwrap(), 12, 6, 6),
        [255, 0, 0, 255]
    );
    assert_eq!(
        pixel(&read_rgba(gpu, &targets[1], size).unwrap(), 12, 6, 6),
        [0, 0, 255, 255]
    );
}

#[test]
fn rounded_clips_stay_in_the_root_pass_until_the_fifth_clip() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    for count in [4, 5] {
        let mut commands = vec![rounded([4.0, 4.0, 24.0, 24.0], 7.0); count];
        commands.extend([
            rect([0.0, 0.0, 40.0, 40.0], [255, 0, 0, 255]),
            Command::PushTransform {
                translate_x: 2.0,
                translate_y: 0.0,
                rotate_rad: 0.0,
                scale_x: 1.0,
                scale_y: 1.0,
            },
            rect([0.0, 0.0, 40.0, 40.0], [0, 0, 255, 255]),
            Command::Pop,
        ]);
        commands.extend(std::iter::repeat_n(Command::Pop, count));
        let pixels = capture(&mut raster, count as u64, commands, PixelSize::new(40, 40));
        let stats = raster.submit_stats().render;
        assert_eq!(pixel(&pixels, 40, 4, 4), [0; 4]);
        assert_eq!(pixel(&pixels, 40, 16, 16), [0, 0, 255, 255]);
        assert_eq!(stats.offscreen_passes, u32::from(count == 5), "{stats:?}");
    }
}

fn path(rect: [f32; 4]) -> CanvasPath {
    let [x, y, w, h] = rect;
    CanvasPath {
        fill: FillRule::EvenOdd,
        verbs: Arc::from([
            PathVerb::Move(x, y),
            PathVerb::Line(x + w, y),
            PathVerb::Line(x + w, y + h),
            PathVerb::Line(x, y + h),
            PathVerb::Close,
        ]),
    }
}

fn fill_path(path: CanvasPath, color: [u8; 4], composite: CanvasComposite) -> CanvasOp {
    CanvasOp::Fill {
        path,
        paint: CanvasPaint {
            brush: CanvasBrush::Color(color),
            alpha: 255,
            composite,
        },
    }
}

fn canvas(ops: Vec<CanvasOp>, width: f32, height: f32) -> Command {
    Command::Canvas {
        x: 0.0,
        y: 0.0,
        width,
        height,
        list: CanvasList {
            body: Arc::new(CanvasBody {
                isolation: CanvasIsolation::Shared,
                bytes: 0,
                images: Box::new([]),
                ops: ops.into_boxed_slice(),
            }),
        },
    }
}

#[test]
fn consecutive_meshes_resolve_once_and_reuse_picture_geometry() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let ops = (0u16..500)
        .map(|index| {
            let x = f32::from(index % 25) * 2.0;
            let y = f32::from(index / 25) * 2.0;
            fill_path(
                path([x, y, 2.0, 2.0]),
                [0, 255, 0, 255],
                CanvasComposite::SourceOver,
            )
        })
        .collect();
    let scene = tree(1, [canvas(ops, 56.0, 48.0)]);
    let size = PixelSize::new(64, 64);
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    let pixels = raster.render_rgba(size).unwrap();
    assert_eq!(pixel(&pixels, 64, 44, 34), [0, 255, 0, 255]);
    assert_eq!(pixel(&pixels, 64, 55, 51), [0; 4]);
    assert_eq!(raster.stats.render.offscreen_passes, 1);
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    assert_eq!(raster.render_rgba(size).unwrap(), pixels);
    assert_eq!(raster.stats.picture_reused, 1);
    assert_eq!(raster.stats.render.buffers_created, 0);
    assert_eq!(raster.stats.render.bind_groups_created, 0);
}

#[test]
fn packed_path_clips_intersect_and_use_their_own_ranges() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let triangle = |right| CanvasPath {
        fill: FillRule::NonZero,
        verbs: Arc::from(if right {
            [
                PathVerb::Move(0.0, 0.0),
                PathVerb::Line(32.0, 0.0),
                PathVerb::Line(32.0, 32.0),
                PathVerb::Close,
            ]
        } else {
            [
                PathVerb::Move(0.0, 0.0),
                PathVerb::Line(32.0, 0.0),
                PathVerb::Line(0.0, 32.0),
                PathVerb::Close,
            ]
        }),
    };
    let ops = vec![
        CanvasOp::Clip {
            path: triangle(false),
        },
        CanvasOp::Clip {
            path: triangle(true),
        },
        fill_path(
            path([0.0, 0.0, 32.0, 32.0]),
            [255, 0, 0, 255],
            CanvasComposite::SourceOver,
        ),
    ];
    let other = PictureId(2);
    let mut scene = tree(1, [canvas(ops, 32.0, 32.0)]);
    scene.pictures.push(Arc::new(Picture::new(
        other,
        1,
        (32.0, 0.0),
        Arc::from([canvas(
            vec![
                CanvasOp::Clip {
                    path: triangle(true),
                },
                fill_path(
                    path([0.0, 0.0, 32.0, 32.0]),
                    [0, 0, 255, 255],
                    CanvasComposite::SourceOver,
                ),
            ],
            32.0,
            32.0,
        )]),
    )));
    scene.layers[0].next = Some(LayerId(1));
    scene.layers.push(Layer {
        kind: LayerKind::Picture { picture: other },
        parent: None,
        first_child: None,
        next: None,
    });
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    let pixels = raster.render_rgba(PixelSize::new(64, 32)).unwrap();
    assert_eq!(pixel(&pixels, 64, 15, 5), [255, 0, 0, 255]);
    assert_eq!(pixel(&pixels, 64, 5, 15), [0; 4]);
    assert_eq!(pixel(&pixels, 64, 25, 16), [0; 4]);
    assert_eq!(pixel(&pixels, 64, 57, 16), [0, 0, 255, 255]);
    assert_eq!(pixel(&pixels, 64, 37, 15), [0; 4]);
}

#[test]
fn fixed_blends_and_multiply_keep_linear_premultiplied_color() {
    use CanvasComposite::*;
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let modes = [
        (SourceOver, [188, 0, 187, 255]),
        (DestinationOver, [0, 0, 255, 255]),
        (DestinationOut, [0, 0, 255, 127]),
        (Lighter, [188, 0, 255, 255]),
        (Xor, [0, 0, 255, 127]),
        (Screen, [188, 0, 255, 255]),
        (Multiply, [0, 0, 187, 255]),
    ];
    for (index, (mode, expected)) in modes.into_iter().enumerate() {
        let ops = vec![fill_path(
            path([0.0, 0.0, 20.0, 20.0]),
            [255, 0, 0, 128],
            mode,
        )];
        let pixels = capture(
            &mut raster,
            index as u64,
            vec![
                Command::Fill {
                    color: [0, 0, 255, 255],
                },
                canvas(ops, 32.0, 32.0),
            ],
            PixelSize::new(32, 32),
        );
        let actual = pixel(&pixels, 32, 8, 8);
        assert!(
            actual
                .into_iter()
                .zip(expected)
                .all(|(a, b)| a.abs_diff(b) <= 2),
            "{mode:?}: {actual:?}"
        );
        assert_eq!(pixel(&pixels, 32, 25, 25), [0, 0, 255, 255]);
        assert_eq!(raster.stats.render.direct_present, mode != Multiply);
    }
}

#[test]
fn opacity_and_blur_keep_shadows_outside_the_layout_box() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let bounds = [30.0, 20.0, 12.0, 12.0];
    let [x, y, w, h] = bounds;
    for (generation, effect) in [
        Command::PushOpacity {
            x,
            y,
            w,
            h,
            alpha: 160,
        },
        Command::PushBlur {
            x,
            y,
            w,
            h,
            sigma: 2.0,
        },
    ]
    .into_iter()
    .enumerate()
    {
        let pixels = capture(
            &mut raster,
            generation as u64,
            vec![effect, shadowed(bounds), Command::Pop],
            PixelSize::new(128, 96),
        );
        let shadow = pixel(&pixels, 128, 51, 25);
        assert!(shadow[3] > 72 && shadow[1] > 220, "{shadow:?}");
        assert_eq!(pixel(&pixels, 128, 90, 25), [0; 4]);
        assert!(raster.stats.render.pooled_texture_bytes < 3 * 128 * 96 * 4);
    }
}

#[test]
fn local_backdrop_reuses_buckets_without_sampling_old_pixels() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let mut fresh = headless(FontLibrary::new()).unwrap();
    let size = PixelSize::new(256, 192);
    let scene = |x, w, h| {
        vec![
            Command::Fill {
                color: [0, 0, 0, 255],
            },
            rect([0.0, 0.0, 23.0, 192.0], [255; 4]),
            Command::PushBackdropBlur {
                x,
                y: 50.0,
                w,
                h,
                sigma: 3.0,
            },
            Command::Pop,
        ]
    };
    capture(&mut raster, 1, scene(12.0, 39.0, 41.0), size);
    let pixels = capture(&mut raster, 2, scene(0.0, 28.0, 26.0), size);
    assert_eq!(pixels, capture(&mut fresh, 2, scene(0.0, 28.0, 26.0), size));
    assert!(raster.stats.render.pooled_texture_bytes < 2 * 256 * 192 * 4);
    assert_eq!(pixel(&pixels, 256, 0, 60), [255; 4]);
    assert_eq!(pixel(&pixels, 256, 30, 60), [0, 0, 0, 255]);
}

#[test]
fn dense_mask_glyphs_share_a_draw_and_coalesce_uploads() {
    let fonts = FontLibrary::new();
    let mut shaper = ParleyShaper::new(fonts.clone());
    assert!(shaper.register("Sans", ROBOTO));
    let font = Arc::new(TextStyle::default());
    let key = metrics_key(
        Arc::from("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"),
        font,
        &[],
        18.0,
        640.0,
    );
    let text = Command::Text {
        x: 0.0,
        y: 0.0,
        shaped: shaper.metrics(&key).shaped.clone(),
        color: [0, 0, 255, 255],
        span_paints: Arc::from([]),
    };
    let Some(mut raster) = headless(fonts) else {
        return;
    };
    capture(&mut raster, 1, vec![text], PixelSize::new(640, 30));
    let stats = raster.submit_stats();
    assert_eq!(stats.render.draws, 1);
    assert!(
        stats.atlas.uploads >= 50 && stats.atlas.upload_calls <= 2,
        "{stats:?}"
    );
    assert_eq!(stats.render.atlas_upload_bytes, stats.atlas.upload_bytes);
    assert!(stats.render.atlas_upload_bytes > 0);
}

fn bench_text(
    shaper: &mut ParleyShaper,
    text: Arc<str>,
    size: f32,
    width: f32,
    decorated: bool,
) -> Command {
    let spans = if decorated {
        vec![engine::scene::TextSpan {
            utf8: text.to_string(),
            color: [255, 255, 255, 255],
            gradient: Some(gradient()),
            weight: 400,
            underline: true,
            line_through: false,
            family: None,
        }]
    } else {
        Vec::new()
    };
    let key = metrics_key(text, Arc::new(TextStyle::default()), &spans, size, width);
    Command::Text {
        x: 4.0,
        y: 4.0,
        shaped: shaper.metrics(&key).shaped.clone(),
        color: [255; 4],
        span_paints: spans.iter().map(engine::scene::TextSpan::paint).collect(),
    }
}

#[test]
#[ignore = "GPU microbenchmark"]
fn bench_scroll_1000_cards() {
    let fonts = FontLibrary::new();
    let mut shaper = ParleyShaper::new(fonts.clone());
    assert!(shaper.register("Sans", ROBOTO));
    let text = bench_text(&mut shaper, Arc::from("Card"), 14.0, 140.0, false);
    let mut commands = Vec::new();
    for index in 0..1000u16 {
        commands.push(Command::PushTransform {
            translate_x: f32::from(index % 4) * 160.0,
            translate_y: f32::from(index / 4) * 60.0,
            rotate_rad: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
        });
        let mut card = shadowed([0.0, 0.0, 140.0, 54.0]);
        if let Command::Rect {
            gradient: brush,
            corner_radius,
            stroke_width,
            stroke_color,
            ..
        } = &mut card
        {
            *brush = Some(gradient());
            *corner_radius = 7.0;
            *stroke_width = 1.0;
            *stroke_color = [190, 200, 220, 255];
        }
        commands.extend([
            card,
            rounded([0.0, 0.0, 140.0, 54.0], 7.0),
            Command::Image {
                x: 100.0,
                y: 8.0,
                w: 28.0,
                h: 28.0,
                source: Arc::from("avatar"),
                mode: ContentMode::Fill,
                required: true,
            },
            text.clone(),
            Command::Pop,
            Command::Pop,
        ]);
    }
    let commands: Arc<[Command]> = commands.into();
    let mut raster = benchmark::headless(fonts);
    benchmark::run(
        &mut raster,
        "scroll_1000_cards",
        120,
        PixelSize::new(640, 720),
        |frame| {
            let mut scene = tree(1, Arc::clone(&commands));
            scene.root = LayerId(1);
            scene.layers[0].parent = Some(scene.root);
            scene.layers.push(Layer {
                kind: LayerKind::Offset {
                    x: 0.0,
                    y: -(frame as f32) * 10.0,
                },
                parent: None,
                first_child: Some(LayerId(0)),
                next: None,
            });
            scene
        },
    );
}

#[test]
#[ignore = "GPU microbenchmark"]
fn bench_dense_text_wall() {
    let fonts = FontLibrary::new();
    let mut shaper = ParleyShaper::new(fonts.clone());
    assert!(shaper.register("Sans", ROBOTO));
    let mut text: String = (0x4e00..0x4e00 + 500).filter_map(char::from_u32).collect();
    text.push_str(" abcdefghijklmnopqrstuvwxyz 🌈🧑🏽‍💻❤️🎨");
    let text: Arc<str> = text.into();
    let mut raster = benchmark::headless(fonts);
    benchmark::run(
        &mut raster,
        "dense_text_wall",
        30,
        PixelSize::new(800, 680),
        |frame| {
            let command = bench_text(
                &mut shaper,
                Arc::clone(&text),
                20.0 + (frame % 8) as f32,
                784.0,
                true,
            );
            tree(u64::from(frame), [command])
        },
    );
}

#[test]
#[ignore = "GPU microbenchmark"]
fn bench_backdrop_blur_overlays() {
    let mut raster = benchmark::headless(FontLibrary::new());
    benchmark::run(
        &mut raster,
        "backdrop_blur_overlays",
        60,
        PixelSize::new(800, 600),
        |frame| {
            let mut commands = vec![Command::Fill {
                color: [44, 60, 90, 255],
            }];
            for index in 0..20u16 {
                commands.push(rect(
                    [f32::from(index) * 45.0 - frame as f32, 0.0, 20.0, 600.0],
                    [180, 100, 50, 255],
                ));
            }
            for index in 0..3u16 {
                let [x, y, w, h] = [
                    80.0 + f32::from(index) * 160.0,
                    80.0 + f32::from(index) * 130.0,
                    240.0,
                    120.0,
                ];
                commands.extend([
                    rounded([x, y, w, h], 16.0),
                    Command::PushBackdropBlur {
                        x,
                        y,
                        w,
                        h,
                        sigma: 9.0,
                    },
                    rect([x, y, w, h], [255, 255, 255, 50]),
                    Command::Pop,
                    Command::Pop,
                ]);
            }
            tree(u64::from(frame), commands)
        },
    );
}

#[test]
#[ignore = "GPU microbenchmark"]
fn bench_canvas_500_paths() {
    let mut raster = benchmark::headless(FontLibrary::new());
    benchmark::run(
        &mut raster,
        "canvas_500_paths",
        60,
        PixelSize::new(720, 480),
        |frame| {
            let mut ops = vec![CanvasOp::Clip {
                path: path([20.0, 20.0, 650.0, 400.0]),
            }];
            ops.extend((0..500u16).map(|index| {
                let y = f32::from(index % 125) * 3.0 + 20.0;
                let shift = (frame / 10) as f32;
                let curve = CanvasPath {
                    fill: FillRule::NonZero,
                    verbs: Arc::from([
                        PathVerb::Move(20.0, y),
                        PathVerb::Cubic(160.0, y - shift, 440.0, y + 8.0, 660.0, y),
                        PathVerb::Line(660.0, y + 2.0),
                        PathVerb::Line(20.0, y + 2.0),
                        PathVerb::Close,
                    ]),
                };
                let mode = match index % 101 {
                    99 => CanvasComposite::Screen,
                    100 => CanvasComposite::Lighter,
                    _ => CanvasComposite::SourceOver,
                };
                fill_path(curve, [70, 100, 220, 128], mode)
            }));
            ops.push(CanvasOp::Clear {
                x: 5.0,
                y: 5.0,
                w: 30.0,
                h: 30.0,
                ctm: crate::compiler::IDENTITY,
            });
            tree(u64::from(frame / 10), [canvas(ops, 720.0, 480.0)])
        },
    );
}

#[test]
fn anisotropic_group_bounds_preserve_blur_and_antialiasing() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let size = PixelSize::new(128, 96);
    let scenes = [
        (
            [1.0, 0.1],
            vec![
                Command::PushBlur {
                    x: 38.0,
                    y: 300.0,
                    w: 20.0,
                    h: 140.0,
                    sigma: 10.0,
                },
                rect([38.0, 300.0, 20.0, 140.0], [255; 4]),
                Command::Pop,
            ],
            [48, 24],
        ),
        (
            [10.0, 0.1],
            vec![
                rect([5.0, 300.0, 3.0, 140.0], [255; 4]),
                rect([9.0, 300.0, 2.0, 140.0], [255; 4]),
            ],
            [35, 37],
        ),
    ];
    for ([sx, sy], body, [x, y]) in scenes {
        let mut plain = tree(1, body.clone());
        plain.root = LayerId(1);
        plain.layers[0].parent = Some(plain.root);
        plain.layers.push(Layer {
            kind: LayerKind::Transform {
                translate_x: 0.0,
                translate_y: 0.0,
                rotate_rad: 0.0,
                scale_x: sx,
                scale_y: sy,
            },
            parent: None,
            first_child: Some(LayerId(0)),
            next: None,
        });
        let mut grouped = plain.clone();
        let mut commands = vec![Command::PushOpacity {
            x: 0.0,
            y: 0.0,
            w: 2.0,
            h: 2.0,
            alpha: 255,
        }];
        commands.extend(body);
        commands.push(Command::Pop);
        grouped.pictures[0] = Arc::new(Picture::new(PictureId(1), 2, (0.0, 0.0), commands.into()));
        raster.invalidate_pictures();
        raster.submit_layers(&plain, 1.0, |_| None).unwrap();
        let reference = raster.render_rgba(size).unwrap();
        raster.submit_layers(&grouped, 1.0, |_| None).unwrap();
        let actual = raster.render_rgba(size).unwrap();
        assert!(pixel(&reference, 128, x, y)[3] > 0);
        assert!(
            actual
                .iter()
                .zip(&reference)
                .all(|(a, b)| a.abs_diff(*b) <= 1),
            "offscreen replay cropped visible pixels under {sx} × {sy}"
        );
    }
}

#[test]
fn resident_mask_pages_color_gradients_and_decorations_share_a_draw() {
    let fonts = FontLibrary::new();
    let mut shaper = ParleyShaper::new(fonts.clone());
    assert!(shaper.register("Sans", ROBOTO));
    let command = bench_text(&mut shaper, Arc::from("AB😀"), 930.0, 2800.0, true);
    let Some(mut raster) = headless(fonts) else {
        return;
    };
    let size = PixelSize::new(2800, 1200);
    let pixels = capture(&mut raster, 1, vec![command], size);
    let stats = raster.submit_stats();
    assert_eq!(stats.render.draws, 1, "{stats:?}");
    assert!(stats.atlas.bytes >= 2 * 1024 * 1024, "{stats:?}");
    if cfg!(target_os = "macos") {
        assert!(
            stats.atlas.bytes >= 6 * 1024 * 1024,
            "expected the system color emoji atlas: {stats:?}"
        );
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[0] > 230 && p[1] > 150 && p[2] < 120 && p[3] == 255),
            "expected the system emoji's yellow pixels"
        );
    }
}

#[test]
fn rotated_rounded_mask_is_shared_by_separate_scopes() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let pixels = capture(
        &mut raster,
        1,
        vec![
            Command::PushTransform {
                translate_x: 20.0,
                translate_y: 20.0,
                rotate_rad: std::f32::consts::FRAC_PI_4,
                scale_x: 1.0,
                scale_y: 1.0,
            },
            rounded([-10.0, -10.0, 20.0, 20.0], 5.0),
            rect([-20.0, -20.0, 40.0, 40.0], [255, 0, 0, 255]),
            Command::PushTransform {
                translate_x: 2.0,
                translate_y: 0.0,
                rotate_rad: 0.0,
                scale_x: 1.0,
                scale_y: 1.0,
            },
            rect([-20.0, -20.0, 40.0, 40.0], [0, 0, 255, 255]),
            Command::Pop,
            Command::Pop,
            Command::Pop,
        ],
        PixelSize::new(40, 40),
    );
    assert_eq!(pixel(&pixels, 40, 20, 20), [0, 0, 255, 255]);
    assert_eq!(pixel(&pixels, 40, 10, 10), [0; 4]);
    assert_eq!(raster.stats.render.offscreen_passes, 1);
}

#[test]
fn timestamp_readback_keeps_frame_and_device_identity() {
    let mut raster = match Rasterizer::headless(
        RasterOptions {
            enable_gpu_timing: true,
            validation: crate::ValidationMode::AsyncOnly,
            ..RasterOptions::default()
        },
        FontLibrary::new(),
    ) {
        Ok(raster) => raster,
        Err(Status::NoDevice(_)) if std::env::var_os("IMPASTO_TEST_GPU").is_none() => return,
        Err(error) => panic!("headless renderer: {error}"),
    };
    let size = PixelSize::new(32, 32);
    let scene = tree(1, [rect([0.0, 0.0, 32.0, 32.0], [255; 4])]);
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    raster.render_rgba(size).unwrap();
    let reference = raster.submit_stats().render;
    let frame_id = 42;
    raster.begin_gpu_frame(Some(frame_id));
    raster.render_rgba(size).unwrap();
    let sample = raster.gpu_sample();
    let counters = raster.submit_stats().render;
    assert_eq!(sample.frame_id, frame_id);
    assert_eq!((counters.passes, counters.draws), (1, 1));
    assert_eq!(counters.vertex_bytes, reference.vertex_bytes);
    assert_eq!(counters.buffers_created, 0);
    if raster.device.as_ref().unwrap().gpu.caps.has_timestamp_query {
        assert_eq!(sample.status, crate::GpuTimingStatus::Pending);
        let mut updates = raster.drain_gpu_timings();
        if raster.gpu_timings_pending() {
            benchmark::wait_for_gpu(&raster.device.as_ref().unwrap().gpu);
            updates.extend(raster.drain_gpu_timings());
        }
        let [update] = updates.try_into().unwrap();
        let update: crate::GpuTimingUpdate = update;
        assert_eq!(update.sample.frame_id, frame_id);
        assert_eq!(update.sample.device_generation, sample.device_generation);
        assert_eq!(update.sample.status, crate::GpuTimingStatus::Ready);
        let timing = update.timing.unwrap();
        assert_eq!(timing.passes.len(), 1);
        assert_eq!(timing.passes[0].kind, crate::GpuPassKind::Main);
        assert_eq!(timing.passes[0].draws, 1);
        assert_eq!(timing.frame_span_us, timing.passes_sum_us);
        assert!(timing.frame_span_us > 0.0);
    } else {
        assert_eq!(sample.status, crate::GpuTimingStatus::Unsupported);
        assert!(raster.drain_gpu_timings().is_empty());
    }
}

#[test]
fn timestamp_capture_reports_unsupported_without_an_enabled_feature() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let size = PixelSize::new(16, 16);
    raster
        .submit_layers(
            &tree(1, [rect([0.0, 0.0, 16.0, 16.0], [255; 4])]),
            1.0,
            |_| None,
        )
        .unwrap();
    raster.begin_gpu_frame(Some(8));
    raster.render_rgba(size).unwrap();
    assert_eq!(
        raster.gpu_sample().status,
        crate::GpuTimingStatus::Unsupported
    );
    assert!(!raster.gpu_timings_pending());
}
