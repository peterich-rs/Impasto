use std::sync::Arc;

use engine::scene::{
    CanvasComposite, CanvasOp, CanvasPath, Command, ContentMode, FillRule, Layer, LayerId,
    LayerKind, LayerTree, PathVerb, Picture, PictureId,
};

use super::{
    ROBOTO, bench_text, benchmark, canvas, fill_path, headless, output, pixel, read_rgba, rect,
    tree,
};
use crate::compositor::OutputFormat;
use crate::{DamageRect, FontLibrary, ImagePixels, ParleyShaper, PixelSize, Rasterizer};

fn moved_picture(dx: f32, generation: u64, commands: Arc<[Command]>) -> LayerTree {
    let mut scene = tree(generation, commands);
    scene.root = LayerId(1);
    scene.layers[0].parent = Some(scene.root);
    scene.layers.push(Layer {
        kind: LayerKind::Offset { x: dx, y: 0.0 },
        parent: None,
        first_child: Some(LayerId(0)),
        next: None,
    });
    scene
}

fn route(generation: u64, list: &Command) -> LayerTree {
    let mut scene = LayerTree::empty();
    for index in 0..4u32 {
        let id = PictureId(u64::from(index));
        let origin = ((index % 2) as f32 * 176.0, (index / 2) as f32 * 126.0);
        scene.pictures.push(Arc::new(Picture::new(
            id,
            generation,
            origin,
            Arc::from([list.clone()]),
        )));
        scene.layers.push(Layer {
            kind: LayerKind::Picture { picture: id },
            parent: None,
            first_child: None,
            next: (index < 3).then_some(LayerId(index + 1)),
        });
    }
    scene
}

fn canvas_paths(count: u16) -> Command {
    let ops: Vec<CanvasOp> = (0..count)
        .map(|index| {
            let y = 4.0 + f32::from(index % 28) * 3.5;
            let shift = f32::from(index % 13) * 0.3;
            let path = CanvasPath {
                fill: FillRule::NonZero,
                verbs: Arc::from([
                    PathVerb::Move(4.0, y),
                    PathVerb::Cubic(40.0, y - 5.0 - shift, 90.0, y + 8.0, 151.0, y),
                    PathVerb::Line(153.0, y + 2.5),
                    PathVerb::Cubic(110.0, y + 7.0, 50.0, y + 3.0, 4.0, y + 2.0),
                    PathVerb::Close,
                ]),
            };
            fill_path(path, [70, 100, 220, 180], CanvasComposite::SourceOver)
        })
        .collect();
    canvas(ops, 160.0, 120.0)
}

fn blurred_card() -> Arc<[Command]> {
    Arc::from([
        Command::PushBlur {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            sigma: 3.0,
        },
        Command::PushBlur {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            sigma: 2.0,
        },
        rect([34.0, 34.0, 46.0, 32.0], [70, 170, 250, 255]),
        Command::Pop,
        Command::Pop,
        rect([38.0, 38.0, 36.0, 22.0], [30, 70, 135, 255]),
    ])
}

#[test]
fn parallel_prepare_preserves_canvas_output_and_install_order() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let list = canvas_paths(128);
    let size = PixelSize::new(352, 252);
    raster.parallel_prepare = false;
    raster
        .submit_layers(&route(1, &list), 1.0, |_| None)
        .unwrap();
    let serial = raster.render_rgba(size).unwrap();
    raster.parallel_prepare = true;
    let report = raster
        .submit_layers(&route(2, &list), 1.0, |_| None)
        .unwrap();
    assert_eq!(report.stats.picture_rebuilt, 4);
    if crate::prepare::pool().is_some() {
        assert!(report.stats.picture_workers >= 2);
    }
    let parallel = raster.render_rgba(size).unwrap();
    assert_eq!(parallel, serial);
    assert!(parallel.iter().any(|pixel| *pixel != 0));
}

#[test]
fn swash_workers_preserve_glyph_pixels_and_atlas_install_order() {
    let fonts = FontLibrary::new();
    let (Some(mut serial), Some(mut parallel)) = (headless(fonts.clone()), headless(fonts.clone()))
    else {
        return;
    };
    let mut shaper = ParleyShaper::new(fonts);
    assert!(shaper.register("Sans", ROBOTO));
    let text = bench_text(&mut shaper, Arc::from("Impasto 120 Hz"), 17.0, 150.0, false);
    let scene = route(1, &text);
    let size = PixelSize::new(352, 252);
    serial.parallel_prepare = false;
    serial.submit_layers(&scene, 1.0, |_| None).unwrap();
    parallel.submit_layers(&scene, 1.0, |_| None).unwrap();
    let expected = serial.render_rgba(size).unwrap();
    let actual = parallel.render_rgba(size).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(parallel.stats.atlas.uploads, serial.stats.atlas.uploads);
    if crate::prepare::pool().is_some() {
        assert!(parallel.stats.atlas.prerasterized >= 4);
    }
}

#[test]
fn worker_known_image_reuses_geometry_but_new_pixels_mark_damage() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let factory = raster.texture_factory().unwrap();
    let command = Command::Image {
        x: 4.0,
        y: 4.0,
        w: 24.0,
        h: 24.0,
        source: Arc::from("swatch"),
        mode: ContentMode::Fill,
        required: true,
    };
    let scene = route(1, &command);
    let size = PixelSize::new(352, 252);
    let first = factory.upload_rgba(1, 1, &[100, 200, 220, 255]).unwrap();
    raster.install_uploaded("swatch", &first);
    let initial = raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    assert_eq!(initial.stats.picture_rebuilt, 4);
    assert_eq!(
        pixel(&raster.render_rgba(size).unwrap(), 352, 10, 10),
        [100, 200, 220, 255]
    );

    let next = factory.upload_rgba(1, 1, &[220, 50, 40, 255]).unwrap();
    raster.install_uploaded("swatch", &next);
    let updated = raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    assert_eq!(updated.stats.picture_reused, 4);
    assert!(matches!(updated.stats.damage, DamageRect::Rect(_)));
    assert_eq!(
        pixel(&raster.render_rgba(size).unwrap(), 352, 10, 10),
        [220, 50, 40, 255]
    );
}

#[test]
fn expensive_picture_reuses_a_bounded_texture_after_three_frames() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let commands = blurred_card();
    let size = PixelSize::new(180, 110);
    let render = |raster: &mut Rasterizer, dx, generation| {
        raster
            .submit_layers(
                &moved_picture(dx, generation, Arc::clone(&commands)),
                1.0,
                |_| None,
            )
            .unwrap();
        raster.render_rgba(size).unwrap()
    };
    render(&mut raster, 0.0, 1);
    render(&mut raster, 1.0, 1);
    render(&mut raster, 2.0, 1);
    let cached = render(&mut raster, 3.0, 1);
    let warm = raster.stats.render;
    assert_eq!(warm.subtree_cache.hits, 1);
    assert!((1..=24 * 1024 * 1024).contains(&warm.subtree_cache.bytes));
    assert!(warm.direct_present);
    render(&mut raster, 3.0, 2);
    assert_eq!(raster.stats.render.subtree_cache.bytes, 0);
    raster
        .device
        .as_mut()
        .unwrap()
        .compositor
        .disable_subtree_cache();
    let uncached = render(&mut raster, 3.0, 2);
    assert!(uncached.iter().zip(cached).all(|(a, b)| a.abs_diff(b) <= 2));
    assert!(warm.passes < raster.stats.render.passes);
}

#[test]
fn fractional_scroll_reuses_blur_and_replays_sharp_content() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let commands = blurred_card();
    let size = PixelSize::new(180, 110);
    let render = |raster: &mut Rasterizer, dx| {
        raster
            .submit_layers(&moved_picture(dx, 1, Arc::clone(&commands)), 1.0, |_| None)
            .unwrap();
        raster.render_rgba(size).unwrap()
    };
    for dx in [-0.15, 0.35, 0.8] {
        render(&mut raster, dx);
    }
    for dx in [1.15, 1.7, 2.25, 2.6, 3.1] {
        let cached = render(&mut raster, dx);
        let warm = raster.stats.render;
        assert_eq!(warm.subtree_cache.hits, 1, "scroll offset {dx}");
        assert_eq!(warm.subtree_cache.evictions, 0, "scroll offset {dx}");
        assert_eq!(pixel(&cached, 180, 60, 45), [30, 70, 135, 255]);
    }
    let cached = render(&mut raster, 3.1);
    let warm_passes = raster.stats.render.passes;
    raster
        .device
        .as_mut()
        .unwrap()
        .compositor
        .disable_subtree_cache();
    let uncached = render(&mut raster, 3.1);
    let max_alpha_delta = cached
        .as_chunks::<4>()
        .0
        .iter()
        .zip(uncached.as_chunks::<4>().0)
        .map(|(a, b)| a[3].abs_diff(b[3]))
        .max()
        .unwrap();
    assert!(max_alpha_delta <= 8, "alpha error {max_alpha_delta}");
    assert!(warm_passes < raster.stats.render.passes);
}

#[test]
fn retained_offscreen_redraws_only_dirty_pixels_and_keeps_direct_present() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let device = raster.device.as_mut().unwrap();
    device.compositor.set_retain_offscreen(true);
    let gpu = device.gpu.clone();
    let size = PixelSize::new(100, 96);
    let format = wgpu::TextureFormat::Bgra8Unorm;
    let target = output(&gpu, format, size);
    let view = target.create_view(&Default::default());
    let commands: Arc<[Command]> = Arc::from([rect([8.0, 12.0, 10.0, 10.0], [210, 50, 25, 255])]);
    let before = moved_picture(0.0, 1, Arc::clone(&commands));
    let after = moved_picture(15.0, 1, commands);
    let submitted = raster.submit_layers(&before, 1.0, |_| None).unwrap();
    assert_eq!(submitted.stats.damage, DamageRect::Full);
    raster.encode_and_submit(&view, format, size).unwrap();
    let moved = raster.submit_layers(&after, 1.0, |_| None).unwrap();
    assert!(moved.stats.damage.fraction(size) < 0.15);
    assert_ne!(moved.stats.damage, DamageRect::Empty);
    raster.encode_and_submit(&view, format, size).unwrap();
    assert!(raster.stats.render.partial_redraw);
    let partial = read_rgba(&gpu, &target, size).unwrap();
    assert_eq!(pixel(&partial, 100, 12, 15), [0; 4]);
    assert_eq!(pixel(&partial, 100, 25, 15), [25, 50, 210, 255]);
    let unchanged = raster.submit_layers(&after, 1.0, |_| None).unwrap();
    assert_eq!(unchanged.stats.damage, DamageRect::Empty);

    raster
        .device
        .as_mut()
        .unwrap()
        .compositor
        .set_retain_offscreen(false);
    raster.encode_and_submit(&view, format, size).unwrap();
    assert_eq!(read_rgba(&gpu, &target, size).unwrap(), partial);
    raster
        .device
        .as_mut()
        .unwrap()
        .compositor
        .set_retain_offscreen(true);
    raster.render_rgba(size).unwrap();
    assert!(raster.stats.render.direct_present);
    assert!(!raster.stats.render.partial_redraw);
}

#[test]
fn changing_canvas_meshes_stream_and_partially_replay_through_a_path_clip() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let device = raster.device.as_mut().unwrap();
    device.compositor.set_retain_offscreen(true);
    let gpu = device.gpu.clone();
    let size = PixelSize::new(160, 128);
    let format = wgpu::TextureFormat::Bgra8Unorm;
    let target = output(&gpu, format, size);
    let view = target.create_view(&Default::default());
    let make_commands = |color| {
        let triangle = CanvasPath {
            fill: FillRule::NonZero,
            verbs: Arc::from([
                PathVerb::Move(1.0, 1.0),
                PathVerb::Line(16.0, 1.0),
                PathVerb::Line(12.0, 17.0),
                PathVerb::Close,
            ]),
        };
        Arc::from([canvas(
            vec![
                CanvasOp::Clip {
                    path: triangle.clone(),
                },
                fill_path(triangle, color, CanvasComposite::SourceOver),
            ],
            20.0,
            20.0,
        )])
    };
    let before = moved_picture(0.0, 1, make_commands([210, 50, 25, 255]));
    let after = moved_picture(8.0, 2, make_commands([25, 50, 210, 255]));
    raster.submit_layers(&before, 1.0, |_| None).unwrap();
    raster.encode_and_submit(&view, format, size).unwrap();

    let submit = raster.submit_layers(&after, 1.0, |_| None).unwrap();
    assert_eq!(submit.stats.render.mesh_upload_bytes, 0);
    raster.encode_and_submit(&view, format, size).unwrap();
    assert!(raster.stats.render.partial_redraw);
    assert!(raster.stats.render.mesh_upload_bytes > 0);
    let partial = read_rgba(&gpu, &target, size).unwrap();
    assert_eq!(pixel(&partial, 160, 4, 3), [0; 4]);
    assert_eq!(pixel(&partial, 160, 19, 3), [210, 50, 25, 255]);

    raster
        .device
        .as_mut()
        .unwrap()
        .compositor
        .set_retain_offscreen(false);
    raster.encode_and_submit(&view, format, size).unwrap();
    assert_eq!(read_rgba(&gpu, &target, size).unwrap(), partial);
    raster.encode_and_submit(&view, format, size).unwrap();
    assert_eq!(raster.stats.render.mesh_upload_bytes, 0);
}

#[test]
#[ignore = "GPU microbenchmark"]
fn bench_p3_cold_canvas_route() {
    let mut raster = benchmark::headless(FontLibrary::new());
    let list = canvas_paths(224);
    benchmark::run(
        &mut raster,
        "p3_cold_canvas_route",
        60,
        PixelSize::new(352, 252),
        |frame| route(u64::from(frame), &list),
    );
}

#[test]
#[ignore = "GPU microbenchmark"]
fn bench_p3_cached_blur_scroll() {
    let mut raster = benchmark::headless(FontLibrary::new());
    let commands = blurred_card();
    benchmark::run(
        &mut raster,
        "p3_cached_blur_scroll",
        80,
        PixelSize::new(180, 110),
        |frame| moved_picture((frame % 50) as f32, 1, Arc::clone(&commands)),
    );
}

#[test]
#[ignore = "GPU microbenchmark"]
fn bench_p3_retained_offscreen_cursor() {
    let mut raster = benchmark::headless(FontLibrary::new());
    let cursor = Arc::from([rect([20.0, 20.0, 3.0, 25.0], [255, 255, 255, 255])]);
    let mut grid = vec![Command::Fill {
        color: [18, 24, 38, 255],
    }];
    for row in 0..20u16 {
        for column in 0..15u16 {
            grid.push(Command::PushTransform {
                translate_x: 20.0 + f32::from(column) * 42.0,
                translate_y: 20.0 + f32::from(row) * 35.0,
                rotate_rad: 0.0,
                scale_x: 1.0,
                scale_y: 1.0,
            });
            grid.push(rect([0.0, 0.0, 39.0, 30.0], [55, 75, 100, 255]));
            grid.push(Command::Pop);
        }
    }
    let background = Arc::new(Picture::new(PictureId(2), 1, (0.0, 0.0), grid.into()));
    benchmark::run_in_format(
        &mut raster,
        "p3_retained_offscreen_cursor",
        80,
        PixelSize::new(660, 735),
        |frame| {
            let mut scene = moved_picture((frame % 55) as f32, 1, Arc::clone(&cursor));
            scene.pictures.push(Arc::clone(&background));
            scene.layers.push(Layer {
                kind: LayerKind::Picture {
                    picture: PictureId(2),
                },
                parent: None,
                first_child: None,
                next: Some(scene.root),
            });
            scene.root = LayerId(2);
            scene
        },
        ImagePixels {
            width: 1,
            height: 1,
            rgba: &[0; 4],
        },
        wgpu::TextureFormat::Bgra8Unorm,
    );
}

#[test]
fn independent_effects_keep_one_main_pass_and_preserve_paint_order() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let mut commands = vec![rect([0.0, 0.0, 140.0, 96.0], [0, 32, 89, 255])];
    for idx in 0..5u8 {
        let x = 8.0 + f32::from(idx) * 22.0;
        commands.extend([
            Command::PushOpacity {
                x,
                y: 4.0,
                w: 20.0,
                h: 64.0,
                alpha: 180,
            },
            rect([x, 4.0, 20.0, 64.0], [120, 240, 12, 255]),
            rect([x, 20.0, 20.0, 48.0], [200, 20, 30, 255]),
            Command::Pop,
            rect([x + 8.0, 10.0, 4.0, 58.0], [255, 255, 255, 255]),
        ]);
    }
    raster
        .submit_layers(&tree(1, commands), 1.0, |_| None)
        .unwrap();
    let size = PixelSize::new(140, 96);
    let pixels = raster.render_rgba(size).unwrap();
    let render = raster.submit_stats().render;
    assert_eq!(render.passes - render.offscreen_passes, 1);
    assert_eq!(render.offscreen_passes, 5);
    for idx in 0..5usize {
        let x = 8 + idx * 22;
        let shadow = pixel(&pixels, 140, x + 4, 24);
        assert!(shadow[0] > shadow[1]);
        assert_eq!(pixel(&pixels, 140, x + 9, 24), [255; 4]);
    }
}

#[test]
fn late_surface_retry_preserves_pooled_effects_and_draw_order() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let size = PixelSize::new(96, 80);
    let gpu = raster.device.as_ref().unwrap().gpu.clone();
    for (format, backdrop) in [
        (wgpu::TextureFormat::Rgba8UnormSrgb, false),
        (wgpu::TextureFormat::Rgba8UnormSrgb, true),
        (wgpu::TextureFormat::Rgba8Unorm, false),
    ] {
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
            view_formats: Vec::new(),
            color_space: wgpu::SurfaceColorSpace::Srgb,
        };
        let format = OutputFormat::surface(&config);
        let texture = output(&gpu, config.format, size);
        let view = texture.create_view(&Default::default());
        let mut commands = vec![rect([0.0, 0.0, 96.0, 80.0], [0, 32, 89, 255])];
        commands.extend((0..64u8).map(|i| rect([4.0, 4.0, 88.0, 72.0], [i, 150, 40, 255])));
        for idx in 0..60u8 {
            let x = f32::from(idx % 10) * 9.0;
            let y = f32::from(idx / 10) * 12.0;
            commands.extend([
                Command::PushOpacity {
                    x,
                    y,
                    w: 8.0,
                    h: 9.0,
                    alpha: 180,
                },
                rect([x, y, 8.0, 9.0], [idx * 4, 240, 12, 255]),
                rect([x + 1.0, y + 2.0, 4.0, 5.0], [200, 20, 30, 255]),
                Command::Pop,
            ]);
        }
        if backdrop {
            commands.extend([
                Command::PushBackdropBlur {
                    x: 2.0,
                    y: 3.0,
                    w: 32.0,
                    h: 34.0,
                    sigma: 2.0,
                },
                Command::Pop,
            ]);
        }
        raster
            .submit_layers(&tree(if backdrop { 2 } else { 1 }, commands), 1.0, |_| None)
            .unwrap();
        raster.encode_and_submit(&view, format, size).unwrap();
        let reference = read_rgba(&gpu, &texture, size).unwrap();
        let counts = raster.stats.render;
        assert_eq!(counts.direct_present, config.format.is_srgb() && !backdrop);
        let missed = raster.prepare_surface_frame(format, size).unwrap();
        drop(missed);
        let prepared = raster.prepare_surface_frame(format, size).unwrap();
        raster.submit_surface_frame(&view, prepared).unwrap();
        assert_eq!(read_rgba(&gpu, &texture, size).unwrap(), reference);
        assert_eq!(raster.stats.render.direct_present, counts.direct_present);
        assert_eq!(raster.stats.render.passes, counts.passes);
    }
}

#[test]
fn stable_picture_addition_does_not_expand_damage_to_a_distant_backdrop() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let mut before = tree(1, [rect([0.0, 0.0, 160.0, 160.0], [16, 32, 120, 255])]);
    before.layers[0].next = Some(LayerId(1));
    before.layers.push(Layer {
        kind: LayerKind::BackdropBlur {
            x: 25.0,
            y: 120.0,
            w: 110.0,
            h: 26.0,
            sigma: 3.0,
        },
        parent: None,
        first_child: None,
        next: None,
    });
    raster.submit_layers(&before, 1.0, |_| None).unwrap();
    let size = PixelSize::new(160, 160);
    let original_pixels = raster.render_rgba(size).unwrap();
    assert_eq!(raster.submit_stats().render.backdrop_cache_hits, 0);
    let mut after = before.clone();
    let new = PictureId(7);
    after.pictures.push(Arc::new(Picture::new(
        new,
        1,
        (0.0, 0.0),
        Arc::from([rect([10.0, 5.0, 6.0, 8.0], [220, 110, 30, 255])]),
    )));
    after.layers[0].next = Some(LayerId(2));
    after.layers.push(Layer {
        kind: LayerKind::Picture { picture: new },
        parent: None,
        first_child: None,
        next: Some(LayerId(1)),
    });
    let inserted = raster
        .submit_layers(&after, 1.0, |_| None)
        .unwrap()
        .stats
        .damage;
    let with_new_picture = raster.render_rgba(size).unwrap();
    assert_ne!(with_new_picture, original_pixels);
    let removed = raster
        .submit_layers(&before, 1.0, |_| None)
        .unwrap()
        .stats
        .damage;
    let restored = raster.render_rgba(size).unwrap();
    assert_eq!(restored, original_pixels);
    assert_eq!(raster.submit_stats().render.backdrop_cache_hits, 1);
    assert_eq!(inserted, removed);
    let DamageRect::Rect([_, _, _, bottom]) = inserted else {
        panic!("stable addition must damage its own pixels: {inserted:?}");
    };
    assert!(bottom <= 16, "unrelated backdrop enlarged damage");
}

#[test]
fn active_opacity_reuses_a_snapshot_and_keeps_new_alpha() {
    use engine::{
        motion::MotionCurve,
        scene::{CompositorProp, CompositorTrack, CompositorTrackId},
    };
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let mut scene = tree(
        1,
        [
            rect([8.0, 8.0, 88.0, 64.0], [200, 12, 40, 255]),
            rect([16.0, 24.0, 60.0, 40.0], [20, 120, 230, 255]),
        ],
    );
    scene.root = LayerId(1);
    scene.layers[0].parent = Some(scene.root);
    scene.layers.push(Layer {
        kind: LayerKind::Opacity {
            alpha: 230,
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 90.0,
        },
        parent: None,
        first_child: Some(LayerId(0)),
        next: None,
    });
    scene.tracks = Arc::from([CompositorTrack::new(
        CompositorTrackId(1),
        scene.root,
        CompositorProp::Opacity,
        0.0,
        MotionCurve::Tween {
            from: 230.0,
            to: 10.0,
            duration_ms: 100.0,
            easing: engine::motion::Easing::Linear,
        },
        Some([0.0, 255.0]),
        None,
    )]);
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    let size = PixelSize::new(100, 90);
    let first = raster.render_rgba(size).unwrap();
    assert_eq!(raster.submit_stats().render.transition_snapshot_bakes, 1);
    raster.sample_compositor_tracks(scene.epoch, 50.0).unwrap();
    let second = raster.render_rgba(size).unwrap();
    let render = raster.submit_stats().render;
    assert_eq!(render.transition_snapshot_hits, 1);
    assert_eq!(render.offscreen_passes, 0);
    assert_eq!(pixel(&first, 100, 22, 42)[3], 230);
    assert_eq!(pixel(&second, 100, 22, 42)[3], 120);
}

#[test]
fn marked_snapshot_keeps_content_that_moves_on_screen() {
    use engine::{
        motion::{Easing, MotionCurve},
        scene::{CompositorProp, CompositorTrack, CompositorTrackId, CompositorTransform},
    };
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let mut scene = moved_picture(0.0, 1, Arc::from([rect([-12.0, 8.0, 8.0, 8.0], [255; 4])]));
    scene.layers[1].kind = LayerKind::Transform {
        translate_x: 0.0,
        translate_y: 0.0,
        rotate_rad: 0.0,
        scale_x: 1.0,
        scale_y: 1.0,
    };
    let mut track = CompositorTrack::new(
        CompositorTrackId(1),
        scene.root,
        CompositorProp::TranslateX,
        0.0,
        MotionCurve::Tween {
            from: 0.0,
            to: 40.0,
            duration_ms: 100.0,
            easing: Easing::Linear,
        },
        None,
        Some(CompositorTransform {
            translate: [0.0, 0.0],
            rotate_rad: 0.0,
            scale: [1.0, 1.0],
            pivot: None,
        }),
    );
    track.freeze_subtree = true;
    scene.tracks = Arc::from([track]);
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    let size = PixelSize::new(32, 32);
    let first = raster.render_rgba(size).unwrap();
    assert_eq!(pixel(&first, 32, 10, 10)[3], 0);
    assert_eq!(raster.submit_stats().render.transition_snapshot_bakes, 1);
    raster.sample_compositor_tracks(scene.epoch, 50.0).unwrap();
    let second = raster.render_rgba(size).unwrap();
    assert_eq!(pixel(&second, 32, 10, 10), [255; 4]);
    assert_eq!(raster.submit_stats().render.transition_snapshot_hits, 1);
}

#[test]
fn small_image_atlas_samples_mips_without_sampling_adjacent_images() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let image = |x, source: &'static str| Command::Image {
        x,
        y: 0.0,
        w: 32.0,
        h: 32.0,
        source: Arc::from(source),
        mode: ContentMode::Fill,
        required: true,
    };
    let texture = |rgb: [u8; 3]| {
        (0..128 * 128)
            .flat_map(|index| {
                let [r, g, b] = rgb;
                if (index / 128 + index % 128) % 2 == 0 {
                    [r, g, b, 255]
                } else {
                    [0, 0, 0, 255]
                }
            })
            .collect::<Vec<_>>()
    };
    let red = texture([255, 0, 0]);
    let blue = texture([0, 0, 255]);
    let resolve = |source: &str| {
        Some(ImagePixels {
            width: 128,
            height: 128,
            rgba: if source == "red" { &red } else { &blue },
        })
    };
    let size = PixelSize::new(64, 32);
    let left = image(0.0, "red");
    let right = image(32.0, "blue");
    // In the reference, the clip separates the images into two image runs.
    let standalone = tree(
        1,
        [
            left.clone(),
            super::rounded([32.0, 0.0, 32.0, 32.0], 0.0),
            right.clone(),
            Command::Pop,
        ],
    );
    raster.submit_layers(&standalone, 1.0, resolve).unwrap();
    let expected = raster.render_rgba(size).unwrap();
    raster
        .submit_layers(&tree(2, [left, right]), 1.0, resolve)
        .unwrap();
    let actual = raster.render_rgba(size).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(pixel(&actual, 64, 31, 14)[2], 0);
    assert_eq!(pixel(&actual, 64, 32, 14)[0], 0);
    if std::env::var("IMPASTO_IMAGE_ARRAY").as_deref() == Ok("0")
        && std::env::var("IMPASTO_IMAGE_UPLOAD").is_err()
        && std::env::var("IMPASTO_SMALL_IMAGES").is_err()
    {
        assert_eq!(raster.submit_stats().render.draws, 1);
    }
}

#[test]
fn inline_blurhash_reuses_its_gpu_decoded_texture() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let scene = tree(
        1,
        [Command::Image {
            x: 0.0,
            y: 0.0,
            w: 32.0,
            h: 32.0,
            source: Arc::from("blurhash:LEHV6nWB2yk8pyo0adR*.7kCMdnj"),
            mode: ContentMode::Fill,
            required: true,
        }],
    );
    assert!(scene.image_requirements().is_empty());
    let resolve = |_: &str| panic!("an inline BlurHash must not request a decoded asset");
    let first = raster.submit_layers(&scene, 1.0, resolve).unwrap();
    assert_eq!(first.stats.image_upload.render_passes, 1);
    let size = PixelSize::new(32, 32);
    let pixels = raster.render_rgba(size).unwrap();
    assert_eq!(pixel(&pixels, 32, 6, 6)[3], 255);
    assert_ne!(pixel(&pixels, 32, 6, 6), pixel(&pixels, 32, 25, 25));
    let reused = raster.submit_layers(&scene, 1.0, resolve).unwrap();
    assert_eq!(reused.stats.image_upload.render_passes, 0);
    assert_eq!(raster.render_rgba(size).unwrap(), pixels);
}

#[test]
fn inline_thumbhash_reconstructs_alpha_and_aspect_ratio() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    // Reference ThumbHash decoder gives 32 × 13 and RGBA [125, 121, 87, 120]
    // at (10, 8). Exercise aspect fitting inside a wider picture.
    let scene = tree(
        1,
        [Command::Image {
            x: 0.0,
            y: 0.0,
            w: 32.0,
            h: 25.0,
            source: Arc::from("thumbhash:XWmGEooooIeSiIigkgjad3B3hoiHeHg="),
            mode: ContentMode::Fit,
            required: true,
        }],
    );
    assert!(scene.image_requirements().is_empty());
    let submitted = raster
        .submit_layers(&scene, 1.0, |_| panic!("ThumbHash is raster-owned"))
        .unwrap();
    assert_eq!(submitted.stats.image_upload.render_passes, 1);
    assert_eq!(submitted.stats.image_upload.resident_bytes, 32 * 13 * 4);
    let pixels = raster.render_rgba(PixelSize::new(32, 25)).unwrap();
    assert_eq!(pixel(&pixels, 32, 10, 5)[3], 0);
    let sampled = pixel(&pixels, 32, 10, 14);
    assert!(
        sampled
            .iter()
            .zip([125, 121, 87, 120])
            .all(|(actual, expected)| actual.abs_diff(expected) <= 3),
        "unexpected pixel {sampled:?}"
    );
}

#[test]
fn opaque_translated_sibling_culls_only_the_subtree_inside_its_cover() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let mut scene = tree(1, [rect([9.0, 9.0, 4.0, 4.0], [255, 0, 0, 255])]);
    scene.layers[0].next = Some(LayerId(1));
    for (id, commands) in [
        (
            PictureId(2),
            [rect([1.0, 1.0, 17.0, 3.0], [20, 220, 20, 255])],
        ),
        (
            PictureId(3),
            [rect([0.0, 0.0, 19.0, 19.0], [20, 42, 220, 255])],
        ),
    ] {
        scene.pictures.push(Arc::new(Picture::new(
            id,
            1,
            (0.0, 0.0),
            Arc::from(commands),
        )));
    }
    scene.layers.extend([
        Layer {
            kind: LayerKind::Picture {
                picture: PictureId(2),
            },
            parent: None,
            first_child: None,
            next: Some(LayerId(2)),
        },
        Layer {
            kind: LayerKind::Offset { x: 6.0, y: 6.0 },
            parent: None,
            first_child: Some(LayerId(3)),
            next: None,
        },
        Layer {
            kind: LayerKind::Picture {
                picture: PictureId(3),
            },
            parent: Some(LayerId(2)),
            first_child: None,
            next: None,
        },
    ]);
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    let pixels = raster.render_rgba(PixelSize::new(32, 32)).unwrap();
    assert_eq!(pixel(&pixels, 32, 10, 10), [20, 42, 220, 255]);
    assert_eq!(pixel(&pixels, 32, 3, 2), [20, 220, 20, 255]);
    assert_eq!(raster.submit_stats().render.occluded_siblings, 1);
}
