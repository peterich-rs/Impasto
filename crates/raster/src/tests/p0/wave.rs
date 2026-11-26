//! Synthetic workloads for render scheduling, text, image uploads and Canvas.

use std::sync::Arc;

use engine::scene::{
    BackdropMaterial, CanvasComposite, CanvasPath, Command, ContentMode, CornerShape, FillRule,
    Layer, LayerId, LayerKind, LayerTree, PathVerb, Picture, PictureId,
};

use super::{
    ROBOTO, bench_text, benchmark, canvas, fill_path, gradient, path, rect, rounded, shadowed, tree,
};
use crate::{FontLibrary, ImagePixels, ParleyShaper, PixelSize};

#[test]
#[ignore = "GPU microbenchmark"]
fn bench_hetero_cards_50() {
    let fonts = FontLibrary::new();
    let mut shaper = ParleyShaper::new(fonts.clone());
    assert!(shaper.register("Sans", ROBOTO));
    let headline = bench_text(&mut shaper, Arc::from("Title"), 15.0, 90.0, false);
    let mut detail = bench_text(&mut shaper, Arc::from("Status"), 12.0, 90.0, false);
    if let Command::Text { y, .. } = &mut detail {
        *y = 26.0;
    }
    let mut background = shadowed([0.0, 0.0, 124.0, 72.0]);
    if let Command::Rect { corner_radius, .. } = &mut background {
        *corner_radius = 9.0;
    }
    let mut accent = rect([0.0, 48.0, 124.0, 24.0], [255; 4]);
    if let Command::Rect {
        gradient: paint, ..
    } = &mut accent
    {
        *paint = Some(gradient());
    }
    let mut scene = LayerTree::empty();
    scene.layers.push(Layer {
        kind: LayerKind::Offset { x: 0.0, y: 0.0 },
        parent: None,
        first_child: Some(LayerId(1)),
        next: None,
    });
    for index in 0..50u32 {
        let offset = LayerId(1 + index * 2);
        let content = LayerId(offset.0 + 1);
        let picture = PictureId(u64::from(index) + 1);
        scene.layers.extend([
            Layer {
                kind: LayerKind::Offset {
                    x: 8.0 + (index % 10) as f32 * 142.0,
                    y: 8.0 + (index / 10) as f32 * 90.0,
                },
                parent: Some(scene.root),
                first_child: Some(content),
                next: (index < 49).then_some(LayerId(offset.0 + 2)),
            },
            Layer {
                kind: LayerKind::Picture { picture },
                parent: Some(offset),
                first_child: None,
                next: None,
            },
        ]);
        scene.pictures.push(Arc::new(Picture::new(
            picture,
            1,
            (0.0, 0.0),
            Arc::from([
                background.clone(),
                rounded([0.0, 0.0, 124.0, 72.0], 9.0),
                accent.clone(),
                Command::Image {
                    x: 94.0,
                    y: 12.0,
                    w: 24.0,
                    h: 24.0,
                    source: Arc::from(format!("avatar-{}", index % 32)),
                    mode: ContentMode::Fill,
                    required: true,
                },
                headline.clone(),
                detail.clone(),
                Command::Pop,
            ]),
        )));
    }
    let mut host = benchmark::headless(fonts);
    benchmark::run(
        &mut host,
        "hetero_cards_50",
        120,
        PixelSize::new(1432, 464),
        |frame| {
            let mut scene = scene.clone();
            scene.layers[0].kind = LayerKind::Offset {
                x: 0.0,
                y: (frame % 4) as f32 * 0.25,
            };
            scene
        },
    );
}

#[test]
#[ignore = "GPU microbenchmark"]
fn bench_opaque_modal_stack() {
    let mut commands = Vec::new();
    for level in 0..4u8 {
        let inset = f32::from(level) * 24.0;
        let bounds = [inset, inset, 800.0 - 2.0 * inset, 560.0 - 2.0 * inset];
        let mut panel = shadowed(bounds);
        if let Command::Rect {
            corner_radius,
            color,
            ..
        } = &mut panel
        {
            *corner_radius = 16.0;
            *color = [50 + level * 40, 90, 120, 255];
        }
        commands.extend([panel, rounded(bounds, 16.0)]);
        for item in 0..200u16 {
            let x = inset + 12.0 + f32::from(item % 20) * 30.0;
            let y = inset + 12.0 + f32::from(item / 20) * 38.0;
            commands.extend([
                rect([x, y, 24.0, 24.0], [230, 220, 200, 255]),
                Command::Image {
                    x: x + 2.0,
                    y: y + 2.0,
                    w: 20.0,
                    h: 20.0,
                    source: Arc::from("thumbnail"),
                    mode: ContentMode::Fill,
                    required: true,
                },
            ]);
        }
        commands.push(Command::Pop);
    }
    let scene = tree(1, commands);
    let mut host = benchmark::headless(FontLibrary::new());
    benchmark::run(
        &mut host,
        "opaque_modal_stack",
        60,
        PixelSize::new(800, 560),
        |_| scene.clone(),
    );
}

#[test]
#[ignore = "GPU microbenchmark"]
fn bench_text_scale_and_cjk() {
    let fonts = FontLibrary::new();
    let mut shaper = ParleyShaper::new(fonts.clone());
    assert!(shaper.register("Sans", ROBOTO));
    let body: String = (0x4e00..0x4e00 + 2000)
        .filter_map(char::from_u32)
        .enumerate()
        .flat_map(|(index, ch)| [ch, if index % 19 == 0 { '🦊' } else { 'a' }])
        .take(2000)
        .collect();
    let body = bench_text(&mut shaper, body.into(), 15.0, 920.0, false);
    let title = bench_text(
        &mut shaper,
        Arc::from("多尺度文字 Dynamic Text"),
        48.0,
        980.0,
        false,
    );
    let mut scene = tree(1, [body]);
    let title_id = PictureId(2);
    scene.pictures.push(Arc::new(Picture::new(
        title_id,
        1,
        (0.0, 0.0),
        Arc::from([title]),
    )));
    scene.root = LayerId(1);
    scene.layers[0].parent = Some(scene.root);
    scene.layers[0].next = Some(LayerId(2));
    scene.layers.extend([
        Layer {
            kind: LayerKind::Offset { x: 0.0, y: 0.0 },
            parent: None,
            first_child: Some(LayerId(0)),
            next: None,
        },
        Layer {
            kind: LayerKind::Offset { x: 0.0, y: 0.0 },
            parent: Some(scene.root),
            first_child: Some(LayerId(3)),
            next: None,
        },
        Layer {
            kind: LayerKind::Picture { picture: title_id },
            parent: Some(LayerId(2)),
            first_child: None,
            next: None,
        },
    ]);
    let mut host = benchmark::headless(fonts);
    benchmark::run(
        &mut host,
        "text_scale_and_cjk",
        120,
        PixelSize::new(1600, 1200),
        |frame| {
            let mut scene = scene.clone();
            let scale = 1.0 + 1.5 * frame as f32 / 119.0;
            scene.layers[2].kind = LayerKind::Transform {
                translate_x: 0.0,
                translate_y: 870.0,
                rotate_rad: 0.0,
                scale_x: scale,
                scale_y: scale,
            };
            scene
        },
    );
}

#[test]
#[ignore = "GPU microbenchmark"]
fn bench_blur_and_mip_matrix() {
    const SIDE: u32 = 1024;
    let pixels = vec![[162, 78, 30, 180]; (SIDE * SIDE) as usize];
    let matrix = Arc::new([
        [1.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.9, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.8, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0],
    ]);
    let material = Arc::new(BackdropMaterial {
        sigma: 6.0,
        saturation: 0.85,
        brightness: 1.0,
        tint: [0.05, 0.2, 0.3, 0.12],
        taper: 0.25,
    });
    let mut host = benchmark::headless(FontLibrary::new());
    benchmark::run_with_images(
        &mut host,
        "blur_and_mip_matrix",
        16,
        PixelSize::new(1024, 1024),
        |frame| {
            let frame = frame.min(3);
            let mut commands = Vec::new();
            for (row, side) in [28.0, 56.0, 112.0, 224.0].into_iter().enumerate() {
                for (col, sigma) in [2.5, 4.0, 8.0, 16.0].into_iter().enumerate() {
                    let x = 12.0 + col as f32 * 252.0;
                    let y = 12.0 + row as f32 * 180.0;
                    commands.extend([
                        Command::PushBlur {
                            x,
                            y,
                            w: side,
                            h: side,
                            sigma,
                        },
                        rect([x, y, side, side], [60, 170, 220, 255]),
                        Command::Pop,
                        Command::Image {
                            x,
                            y,
                            w: 48.0,
                            h: 36.0,
                            source: Arc::from(format!("decoded-{frame}-{row}-{col}")),
                            mode: ContentMode::Fill,
                            required: true,
                        },
                    ]);
                }
            }
            commands.extend([
                Command::PushColorFilter {
                    matrix: Arc::clone(&matrix),
                },
                rect([8.0, 832.0, 256.0, 160.0], [60, 170, 220, 255]),
                Command::Pop,
                Command::PushBackdropMaterial {
                    bounds: [124.0, 856.0, 160.0, 100.0],
                    corners: Some(CornerShape {
                        radii: [12.0; 4],
                        curve: 2.0,
                    }),
                    material: Arc::clone(&material),
                },
                Command::Pop,
            ]);
            tree(u64::from(frame), commands)
        },
        ImagePixels {
            width: SIDE,
            height: SIDE,
            rgba: pixels.as_flattened(),
        },
    );
}

#[test]
#[ignore = "GPU microbenchmark"]
fn bench_canvas_mixed_paths() {
    let mut host = benchmark::headless(FontLibrary::new());
    benchmark::run(
        &mut host,
        "canvas_mixed_paths",
        60,
        PixelSize::new(800, 600),
        |frame| {
            let mut ops = Vec::with_capacity(400);
            for index in 0..200u16 {
                let x = f32::from(index % 20) * 40.0;
                let y = f32::from(index / 20) * 58.0;
                ops.push(fill_path(
                    path([x, y, 14.0, 12.0]),
                    [130, 80, 210, 255],
                    CanvasComposite::SourceOver,
                ));
                let phase = (frame % 12) as f32;
                let curve = CanvasPath {
                    fill: FillRule::EvenOdd,
                    verbs: Arc::from([
                        PathVerb::Move(x + 2.0, y + 16.0),
                        PathVerb::Cubic(x + 30.0, y + 50.0, x + phase, y + 7.0, x + 32.0, y + 44.0),
                        PathVerb::Line(x + 1.0, y + 44.0),
                        PathVerb::Cubic(x + 24.0, y + 26.0, x + 12.0, y + 40.0, x + 32.0, y + 16.0),
                        PathVerb::Close,
                    ]),
                };
                ops.push(fill_path(
                    curve,
                    [25, 170, 100, 192],
                    CanvasComposite::SourceOver,
                ));
            }
            tree(u64::from(frame), [canvas(ops, 800.0, 600.0)])
        },
    );
}
