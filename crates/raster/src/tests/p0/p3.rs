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
use crate::{DamageRect, FontLibrary, ParleyShaper, PixelSize};

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
fn a_cold_unknown_viewport_uses_the_same_glyph_pixels_without_preparation() {
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
    assert_eq!(parallel.stats.atlas.prerasterized, 0);
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
    assert_eq!(inserted, removed);
    let DamageRect::Rect([_, _, _, bottom]) = inserted else {
        panic!("stable addition must damage its own pixels: {inserted:?}");
    };
    assert!(bottom <= 16, "unrelated backdrop enlarged damage");
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

#[test]
fn zero_ancestor_clip_avoids_glyph_work_and_opening_draws_the_text() {
    let fonts = FontLibrary::new();
    let mut shaper = ParleyShaper::new(fonts.clone());
    assert!(shaper.register("Sans", ROBOTO));
    let Some(mut raster) = headless(fonts) else {
        return;
    };
    let text = bench_text(&mut shaper, Arc::from("Hidden label"), 14.0, 30.0, false);
    let size = PixelSize::new(420, 180);
    raster.set_resource_viewport(Some(size));
    for generation in 1..=4 {
        let open = generation % 2 == 0;
        let mut scene = moved_picture(0.0, generation, Arc::from([text.clone()]));
        scene.layers[1].kind = LayerKind::Clip {
            x: 0.0,
            y: 0.0,
            w: if open { 160.0 } else { 0.0 },
            h: 60.0,
            corner_radius: 0.0,
        };
        raster.submit_layers(&scene, 2.625, |_| None).unwrap();
        let pixels = raster.render_rgba(size).unwrap();
        let stats = raster.submit_stats();
        assert_eq!(
            pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0),
            open
        );
        if !open {
            assert_eq!(stats.atlas.prerasterized, 0);
            assert_eq!(stats.atlas.uploads, 0);
        }
    }
}
