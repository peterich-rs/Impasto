use std::collections::HashSet;

use engine::scene::{BoxShadow, ColorStop, CornerShape, Gradient, RectPaint};

use super::*;
use crate::{compositor::extent, surface::read_rgba};

mod benchmark;
mod p1;
mod p3;
mod wave;

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
fn mixed_rectangles_keep_shadow_and_fill_order_in_one_pass() {
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
    assert_eq!(cold.passes, 1);
    let table = raster.device.as_ref().unwrap().gpu.caps.paint_storage
        && std::env::var("IMPASTO_STATE_TABLE").as_deref() != Ok("0")
        && std::env::var("IMPASTO_SPATIAL_BATCH").as_deref() != Ok("0");
    if table {
        assert!(cold.draws <= 200, "{cold:?}");
    } else {
        assert_eq!(cold.draws, 200);
    }
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
    // Repeated pictures promote on the second frame. Their third replay
    // preserves the same output and order with warm resources.
    assert_eq!(raster.render_rgba(size).unwrap(), pixels);
    let warm = raster.submit_stats().render;
    assert_eq!((warm.buffers_created, warm.bind_groups_created), (0, 0));
    assert_eq!(warm.passes, 1);
    assert!(warm.draws <= 200);
    assert!(warm.vertex_bytes <= cold.vertex_bytes);
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
fn asymmetric_continuous_corners_agree_in_paint_and_clip() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let bounds = [2.0, 2.0, 24.0, 20.0];
    let corners = CornerShape {
        radii: [12.0, 0.0, 8.0, 3.0],
        curve: 4.0,
    };
    let size = PixelSize::new(28, 24);
    let shape = capture(
        &mut raster,
        1,
        vec![Command::ShapedRect {
            bounds,
            corners,
            paint: RectPaint {
                color: [255; 4],
                stroke_width: 0.0,
                stroke_color: [0; 4],
                gradient: None,
                shadow: None,
                box_shadows: Arc::from([]),
            },
        }],
        size,
    );
    let clipped = capture(
        &mut raster,
        2,
        vec![
            Command::PushCornerClip { bounds, corners },
            rect([0.0, 0.0, 28.0, 24.0], [255; 4]),
            Command::Pop,
        ],
        size,
    );
    assert_eq!(shape, clipped);
    assert_eq!(pixel(&shape, 28, 2, 2)[3], 0);
    assert_eq!(pixel(&shape, 28, 25, 2)[3], 255);
    assert_eq!(pixel(&shape, 28, 25, 21)[3], 0);
    assert_eq!(pixel(&shape, 28, 6, 20)[3], 255);
}

#[test]
fn rounded_shadow_matches_two_dimensional_reference() {
    if std::env::var("IMPASTO_SHADOW_2D").as_deref() == Ok("0") {
        return;
    }
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let bounds = [10.0_f32, 10.0_f32, 12.0_f32, 10.0_f32];
    let corners = CornerShape {
        radii: [4.0; 4],
        curve: 2.0,
    };
    let sigma = 3.0_f32;
    let shadow = BoxShadow {
        offset: [0.0; 2],
        blur_sigma: sigma,
        spread: 0.0,
        color: [255; 4],
    };
    let mut ordinary = rect(bounds, [0; 4]);
    if let Command::Rect {
        corner_radius,
        box_shadows,
        ..
    } = &mut ordinary
    {
        *corner_radius = corners.radii[0];
        *box_shadows = Arc::from([shadow]);
    }
    let commands = [
        ordinary,
        Command::ShapedRect {
            bounds,
            corners,
            paint: RectPaint {
                color: [0; 4],
                stroke_width: 0.0,
                stroke_color: [0; 4],
                gradient: None,
                shadow: None,
                box_shadows: Arc::from([shadow]),
            },
        },
    ];
    let frames = commands
        .into_iter()
        .enumerate()
        .map(|(index, command)| {
            capture(
                &mut raster,
                index as u64 + 1,
                vec![command],
                PixelSize::new(32, 32),
            )
        })
        .collect::<Vec<_>>();
    let steps = 16;
    for [x, y] in [[9, 9], [10, 10], [10, 14], [16, 14]] {
        let mut expected = 0.0;
        for sy in 0..10 * steps {
            for sx in 0..12 * steps {
                let u = (sx as f32 + 0.5) / steps as f32;
                let v = (sy as f32 + 0.5) / steps as f32;
                if corners.contains([12.0, 10.0], [u, v]) {
                    let dx = 9.5 + u - x as f32;
                    let dy = 9.5 + v - y as f32;
                    expected += (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp();
                }
            }
        }
        expected /= std::f32::consts::TAU * sigma * sigma * (steps * steps) as f32;
        for pixels in &frames {
            let actual = f32::from(pixel(pixels, 32, x, y)[3]) / 255.0;
            assert!(
                (actual - expected).abs() < 0.02,
                "{x}, {y}: {actual} vs {expected}"
            );
        }
    }
}

#[test]
fn hollow_shadows_preserve_clipped_and_translucent_edges() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let device = raster.device.as_ref().unwrap();
    if !device.gpu.hollow_shadows {
        return;
    }
    let mut gpu = device.gpu.clone();
    gpu.hollow_shadows = false;
    let mut reference = Rasterizer::create(raster.options, FontLibrary::new()).unwrap();
    reference.device = Some(Rasterizer::new_device(gpu, device.generation, None).unwrap());
    let mut opaque = shadowed([12.0, 12.0, 80.0, 56.0]);
    if let Command::Rect { corner_radius, .. } = &mut opaque {
        *corner_radius = 18.0;
    }
    let mut translucent = opaque.clone();
    if let Command::Rect { color, .. } = &mut translucent {
        color[3] = 128;
    }
    let size = PixelSize::new(128, 96);
    for (generation, commands) in (1..).zip([
        vec![opaque.clone()],
        vec![translucent],
        vec![
            rounded([20.0, 20.0, 70.0, 54.0], 16.0),
            opaque,
            Command::Pop,
        ],
    ]) {
        let expected = capture(&mut reference, generation, commands.clone(), size);
        let actual = capture(&mut raster, generation, commands, size);
        // Straight readback divides by alpha. Compare in the premultiplied
        // space where moving the shadow triangle seam can change rounding.
        let premultiply = |rgba| crate::color::linear_premultiplied(rgba, 255).unwrap_or([0.0; 4]);
        let (offset, delta) = actual
            .as_chunks::<4>()
            .0
            .iter()
            .zip(expected.as_chunks::<4>().0)
            .map(|(&actual, &expected)| {
                premultiply(actual)
                    .into_iter()
                    .zip(premultiply(expected))
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0, f32::max)
            })
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        let x = offset % 128;
        let y = offset / 128;
        assert!(
            delta <= 1.0 / 255.0 + f32::EPSILON,
            "scene {generation}, pixel ({x}, {y}): {:?} vs {:?}",
            pixel(&actual, 128, x, y),
            pixel(&expected, 128, x, y)
        );
    }
}

#[test]
fn opaque_root_can_use_lcd_glyphs() {
    let fonts = FontLibrary::new();
    let mut shaper = ParleyShaper::new(fonts.clone());
    assert!(shaper.register("Sans", ROBOTO));
    let key = metrics_key(
        Arc::from("Impasto"),
        Arc::new(TextStyle::default()),
        &[],
        14.0,
        192.0,
    );
    let Some(mut raster) = headless(fonts) else {
        return;
    };
    let lcd = raster.device.as_ref().unwrap().gpu.caps.lcd;
    let size = PixelSize::new(192, 32);
    let pixels = capture(
        &mut raster,
        1,
        vec![
            rect([0.0, 0.0, 192.0, 32.0], [255; 4]),
            Command::Text {
                x: 4.0,
                y: 2.0,
                shaped: shaper.metrics(&key).shaped.clone(),
                color: [0, 0, 0, 255],
                span_paints: Arc::from([]),
            },
        ],
        size,
    );
    let (pixels, _) = pixels.as_chunks::<4>();
    assert!(pixels.iter().all(|p| p[3] == 255));
    assert!(pixels.iter().any(|p| p[0] < 200));
    assert_eq!(pixels.iter().any(|p| p[0] != p[1] || p[1] != p[2]), lcd);
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
    compositor.disable_residency();
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
                        frame.scene(&raster.fonts, &device.textures, Some(&slice)),
                        &target.create_view(&Default::default()),
                        format,
                        [12, 12],
                        None,
                    )?
                    .finish(),
            );
        }
        let (_, prelude) = compositor.flush(gpu)?;
        gpu.submit(prelude.into_iter().chain(commands));
        compositor.submitted(gpu);
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
    assert_eq!(
        raster.stats.render.offscreen_passes,
        u32::from(std::env::var("IMPASTO_CANVAS_DIRECT").as_deref() == Ok("0")),
    );
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
    let bytes_per_pixel = if std::env::var("IMPASTO_DITHER").as_deref() == Ok("1") {
        8
    } else {
        4
    };
    assert!(raster.stats.render.pooled_texture_bytes < 2 * 256 * 192 * bytes_per_pixel);
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
        stats.atlas.uploads >= 50 && stats.atlas.upload_calls <= 8,
        "{stats:?}"
    );
    assert_eq!(stats.render.atlas_upload_bytes, stats.atlas.upload_bytes);
    assert!((1..2048 * 64).contains(&stats.render.atlas_upload_bytes));
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
    bench_card_scroll(1_000, 10.0);
}

#[test]
#[ignore = "GPU microbenchmark"]
fn bench_scroll_10000_cards() {
    bench_card_scroll(10_000, 100.0);
}

fn bench_card_scroll(count: u16, scroll_per_frame: f32) {
    let fonts = FontLibrary::new();
    let mut shaper = ParleyShaper::new(fonts.clone());
    assert!(shaper.register("Sans", ROBOTO));
    let text = bench_text(&mut shaper, Arc::from("Card"), 14.0, 140.0, false);
    let mut commands = Vec::new();
    for index in 0..count {
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
        &format!("scroll_{count}_cards"),
        120,
        PixelSize::new(640, 720),
        |frame| {
            let mut scene = tree(1, Arc::clone(&commands));
            scene.root = LayerId(1);
            scene.layers[0].parent = Some(scene.root);
            scene.layers.push(Layer {
                kind: LayerKind::Offset {
                    x: 0.0,
                    y: -(frame as f32) * scroll_per_frame,
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
#[ignore = "3x GPU glyph atlas workload"]
fn bench_cjk_atlas_scroll_3x() {
    let fonts = FontLibrary::new();
    let mut shaper = ParleyShaper::new(fonts.clone());
    assert!(shaper.register("Sans", ROBOTO));
    let text: String = (0x4e00..0x5600).filter_map(char::from_u32).collect();
    let commands = Arc::from([bench_text(&mut shaper, text.into(), 14.0, 300.0, false)]);
    let mut raster = headless(fonts).unwrap();
    let gpu = raster.device.as_ref().unwrap().gpu.clone();
    let pixels = PixelSize::new(900, 1800);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let output = output(&gpu, format, pixels).create_view(&Default::default());
    let mut uploads = 0;
    let mut evictions = 0;
    let mut reclaim = 0;
    let mut live_entries = 0;
    for step in 0..24u16 {
        let mut scene = tree(1, Arc::clone(&commands));
        scene.root = LayerId(1);
        scene.layers[0].parent = Some(scene.root);
        scene.layers.push(Layer {
            kind: LayerKind::Offset {
                x: 0.0,
                y: -f32::from(step) * 24.5,
            },
            parent: None,
            first_child: Some(LayerId(0)),
            next: None,
        });
        raster.submit_layers(&scene, 3.0, |_| None).unwrap();
        raster.encode_and_submit(&output, format, pixels).unwrap();
        let atlas = raster.submit_stats().atlas;
        uploads += atlas.uploads;
        evictions += atlas.evictions;
        reclaim += atlas.region_reclaims;
        live_entries = atlas.entries;
    }
    println!(
        "CJK 3x atlas uploads={uploads} entries={live_entries} evictions={evictions} region_reclaims={reclaim}"
    );
    assert_ne!(uploads, 0);
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
    raster
        .device
        .as_mut()
        .unwrap()
        .compositor
        .disable_glyph_outlines();
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
    for _ in 0..3 {
        raster.render_rgba(size).unwrap();
    }
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

#[test]
fn opaque_depth_segments_preserve_painter_order() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let mut commands = vec![rect([2.0, 2.0, 11.0, 13.0], [255, 0, 0, 255]); 24];
    commands.push(rect([5.0, 5.0, 9.0, 9.0], [0, 0, 255, 255]));
    let pixels = capture(&mut raster, 1, commands, PixelSize::new(19, 23));
    assert_eq!(pixel(&pixels, 19, 2, 2), [255, 0, 0, 255]);
    assert_eq!(pixel(&pixels, 19, 6, 6), [0, 0, 255, 255]);
    assert_eq!(pixel(&pixels, 19, 4, 1), [0; 4]);
    let stats = raster.submit_stats().render;
    assert!(stats.direct_present);
    assert_eq!(stats.passes, 1);
    assert_eq!(
        stats.early_z_instances,
        if std::env::var("IMPASTO_EARLY_Z").as_deref() == Ok("1")
            || (std::env::var_os("IMPASTO_EARLY_Z").is_none()
                && std::env::var("IMPASTO_RENDER_GRAPH").as_deref() != Ok("0")
                && std::env::var("IMPASTO_STATE_TABLE").as_deref() != Ok("0")
                && raster.device.as_ref().unwrap().gpu.caps.paint_storage
                && raster
                    .device
                    .as_ref()
                    .unwrap()
                    .gpu
                    .adapter
                    .get_info()
                    .transient_saves_memory
                    == Some(true))
        {
            25
        } else {
            0
        },
    );
}

#[test]
fn rounded_depth_interiors_leave_antialiased_edges_in_painter_order() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let mut back = rect([3.0, 3.0, 23.0, 23.0], [255, 0, 0, 255]);
    let mut front = rect([6.25, 6.5, 23.0, 23.0], [0, 0, 255, 255]);
    for command in [&mut back, &mut front] {
        if let Command::Rect { corner_radius, .. } = command {
            *corner_radius = 6.0;
        }
    }
    let mut commands = vec![back; 64];
    commands.push(front);
    let size = PixelSize::new(32, 32);
    // The rectangular scissor excludes the depth path without clipping the cards.
    let mut reference = vec![rounded([0.0, 0.0, 31.0, 31.0], 0.0)];
    reference.extend_from_slice(&commands);
    reference.push(Command::Pop);
    let expected = capture(&mut raster, 1, reference, size);
    let actual = capture(&mut raster, 2, commands, size);
    assert!(expected.iter().zip(actual).all(|(a, b)| a.abs_diff(b) <= 2));
    if std::env::var("IMPASTO_EARLY_Z").as_deref() == Ok("1") {
        assert_eq!(raster.submit_stats().render.early_z_instances, 65);
    }
}

#[test]
fn color_matrix_filters_composited_children_and_keeps_holes_clear() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let matrix = Arc::new([
        [0.0, 0.0, 1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.5, 0.0],
    ]);
    let children = [
        rect([0.0, 0.0, 12.0, 12.0], [255, 0, 0, 255]),
        rect([8.0, 8.0, 12.0, 12.0], [0, 0, 255, 255]),
    ];
    let size = PixelSize::new(24, 24);
    let mut commands = vec![Command::PushColorFilter {
        matrix: Arc::clone(&matrix),
    }];
    commands.extend_from_slice(&children);
    commands.push(Command::Pop);
    let pixels = capture(&mut raster, 1, commands, size);
    assert!(pixel(&pixels, 24, 4, 4)[2] > 180);
    assert_eq!(pixel(&pixels, 24, 9, 9)[3], 128);
    assert!(pixel(&pixels, 24, 9, 9)[0] > 180);
    assert_eq!(pixel(&pixels, 24, 2, 18), [0; 4]);

    let mut scene = tree(2, children);
    scene.root = LayerId(1);
    scene.layers[0].parent = Some(scene.root);
    scene.layers.push(Layer {
        kind: LayerKind::ColorFilter { matrix },
        parent: None,
        first_child: Some(LayerId(0)),
        next: None,
    });
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    assert_eq!(raster.render_rgba(size).unwrap(), pixels);
    assert_eq!(raster.render_rgba(size).unwrap(), pixels);
    let warm = raster.submit_stats().render;
    assert_eq!((warm.buffers_created, warm.bind_groups_created), (0, 0));
}

#[test]
fn backdrop_material_tapers_blur_and_clips_its_corners() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let commands = vec![
        rect([0.0, 0.0, 24.0, 48.0], [255, 0, 0, 255]),
        rect([24.0, 0.0, 24.0, 48.0], [0, 0, 255, 255]),
        Command::PushBackdropMaterial {
            bounds: [4.0, 4.0, 40.0, 40.0],
            corners: Some(CornerShape {
                radii: [8.0; 4],
                curve: 2.0,
            }),
            material: Arc::new(engine::scene::BackdropMaterial {
                sigma: 5.0,
                saturation: 1.0,
                brightness: 1.0,
                tint: [0.0; 4],
                taper: 1.0,
            }),
        },
        Command::Pop,
    ];
    let size = PixelSize::new(48, 48);
    let scene = tree(1, commands);
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    let pixels = raster.render_rgba(size).unwrap();
    assert_eq!(pixel(&pixels, 48, 4, 4), [255, 0, 0, 255]);
    let upper = pixel(&pixels, 48, 23, 12);
    let lower = pixel(&pixels, 48, 23, 39);
    assert!(upper[2] > lower[2] + 50, "{upper:?} {lower:?}");
    assert!(lower[0] > upper[0]);
    assert_eq!(pixel(&pixels, 48, 23, 45), [255, 0, 0, 255]);
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    assert_eq!(raster.render_rgba(size).unwrap(), pixels);
    assert_eq!(raster.render_rgba(size).unwrap(), pixels);
    let warm = raster.submit_stats().render;
    assert_eq!(warm.backdrop_cache_hits, 1);
    assert_eq!((warm.buffers_created, warm.bind_groups_created), (0, 0));
}

#[test]
fn stencil_intersects_pixel_aligned_canvas_clips_with_even_odd_holes() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let mut ring = path([2.0, 2.0, 26.0, 26.0]).verbs.to_vec();
    ring.extend_from_slice(&path([8.0, 8.0, 8.0, 8.0]).verbs);
    let pixels = capture(
        &mut raster,
        1,
        vec![canvas(
            vec![
                CanvasOp::Clip {
                    path: CanvasPath {
                        fill: FillRule::EvenOdd,
                        verbs: ring.into(),
                    },
                },
                CanvasOp::Save,
                CanvasOp::Clip {
                    path: path([0.0, 0.0, 20.0, 32.0]),
                },
                fill_path(
                    path([0.0, 0.0, 32.0, 32.0]),
                    [255, 0, 0, 255],
                    CanvasComposite::SourceOver,
                ),
                CanvasOp::Restore,
                fill_path(
                    path([20.0, 0.0, 12.0, 32.0]),
                    [0, 0, 255, 255],
                    CanvasComposite::SourceOver,
                ),
            ],
            32.0,
            32.0,
        )],
        PixelSize::new(32, 32),
    );
    assert_eq!(pixel(&pixels, 32, 2, 2), [255, 0, 0, 255]);
    assert_eq!(pixel(&pixels, 32, 10, 10), [0; 4]);
    assert_eq!(pixel(&pixels, 32, 21, 10), [0, 0, 255, 255]);
    assert_eq!(pixel(&pixels, 32, 29, 10), [0; 4]);
    if std::env::var("IMPASTO_CANVAS_STENCIL").as_deref() == Ok("1") {
        let gpu = &raster.device.as_ref().unwrap().gpu;
        let format =
            crate::pipelines::CANVAS_STENCIL.guaranteed_format_features(gpu.device.features());
        if !gpu.caps.msaa4_srgb
            || format
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4)
        {
            assert_eq!(raster.submit_stats().render.stencil_draws, 3);
        }
    }
}

#[test]
fn winding_stencil_matches_tessellated_holes_and_resets_between_fills() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let ring = |fill| {
        let mut verbs = Vec::new();
        for ([x, y, w, h], reverse) in [
            ([4.0, 4.0, 24.0, 24.0], false),
            ([11.0, 11.0, 10.0, 10.0], fill == FillRule::NonZero),
        ] {
            let mut outline = [
                [x, y],
                [x + w * 0.5, y],
                [x + w, y],
                [x + w, y + h * 0.5],
                [x + w, y + h],
                [x + w * 0.5, y + h],
                [x, y + h],
                [x, y + h * 0.5],
            ];
            if reverse {
                outline.reverse();
            }
            verbs.push(PathVerb::Move(outline[0][0], outline[0][1]));
            verbs.extend(outline[1..].iter().map(|&[x, y]| PathVerb::Line(x, y)));
            verbs.push(PathVerb::Close);
        }
        CanvasPath {
            verbs: verbs.into(),
            fill,
        }
    };
    let commands = |force_lyon, fill| {
        let mut ops = Vec::new();
        if force_lyon {
            ops.push(CanvasOp::Clip {
                path: path([0.0, 0.0, 32.0, 32.0]),
            });
        }
        ops.push(fill_path(
            ring(fill),
            [225, 30, 56, 255],
            CanvasComposite::SourceOver,
        ));
        ops.push(fill_path(
            path([13.0, 13.0, 3.0, 3.0]),
            [50, 110, 200, 255],
            CanvasComposite::SourceOver,
        ));
        vec![canvas(ops, 32.0, 32.0)]
    };
    let size = PixelSize::new(32, 32);
    for (i, rule) in (1..).zip([FillRule::NonZero, FillRule::EvenOdd]) {
        let expected = capture(&mut raster, i * 2, commands(true, rule), size);
        let actual = capture(&mut raster, i * 2 + 1, commands(false, rule), size);
        assert_eq!(expected, actual);
        assert_eq!(raster.submit_stats().render.stencil_draws, 1);
        assert_eq!(pixel(&actual, 32, 13, 13), [50, 110, 200, 255]);
        assert_eq!(pixel(&actual, 32, 16, 16), [0; 4]);
    }
}

#[test]
fn scrolling_picture_reuses_rect_image_and_glyph_geometry() {
    let fonts = FontLibrary::new();
    let mut shaper = ParleyShaper::new(fonts.clone());
    assert!(shaper.register("Sans", ROBOTO));
    let Some(mut raster) = headless(fonts) else {
        return;
    };
    let gpu = &raster.device.as_ref().unwrap().gpu;
    let resident = gpu.caps.paint_storage
        && !gpu.caps.lcd
        && std::env::var("IMPASTO_RESIDENT_PICTURES").as_deref() != Ok("0")
        && std::env::var("IMPASTO_SPECIALIZE_PAINT").as_deref() != Ok("0")
        && std::env::var("IMPASTO_EARLY_Z").as_deref() != Ok("1")
        && !gpu.gradient_lut;
    let mut scene = tree(
        1,
        [
            rect([0.0, 0.0, 64.0, 32.0], [0, 0, 255, 255]),
            Command::Image {
                x: 50.0,
                y: 2.0,
                w: 8.0,
                h: 8.0,
                source: Arc::from("icon"),
                mode: ContentMode::Fill,
                required: true,
            },
            bench_text(&mut shaper, Arc::from("Scroll"), 14.0, 48.0, false),
        ],
    );
    scene.root = LayerId(1);
    scene.layers[0].parent = Some(scene.root);
    scene.layers.push(Layer {
        kind: LayerKind::Offset { x: 0.0, y: 0.0 },
        parent: None,
        first_child: Some(LayerId(0)),
        next: None,
    });
    let size = PixelSize::new(72, 42);
    let resolve = |_: &str| {
        Some(ImagePixels {
            width: 1,
            height: 1,
            rgba: &[255, 0, 0, 255],
        })
    };
    let mut scene = Arc::new(scene);
    raster
        .submit_shared_layers(Arc::clone(&scene), 1.0, resolve)
        .unwrap();
    let original = raster.render_rgba(size).unwrap();
    assert!(raster.submit_stats().render.vertex_bytes > 0);
    Arc::make_mut(&mut scene).layers[1].kind = LayerKind::Offset { x: 4.0, y: 4.0 };
    raster.submit_shared_layers(scene, 1.0, resolve).unwrap();
    let translated = raster.render_rgba(size).unwrap();
    assert_eq!(pixel(&original, 72, 52, 5), [255, 0, 0, 255]);
    assert_eq!(pixel(&translated, 72, 56, 9), [255, 0, 0, 255]);
    let stats = raster.submit_stats();
    assert_eq!(stats.picture_reused, 1);
    assert_eq!(stats.atlas.upload_bytes, 0);
    assert!(
        stats.render.vertex_bytes > 0,
        "the second frame promotes the picture"
    );
    assert_eq!(raster.render_rgba(size).unwrap(), translated);
    let stats = raster.submit_stats();
    if resident {
        assert_eq!(stats.render.vertex_bytes, 0, "{stats:?}");
        assert_eq!(stats.render.bind_groups_created, 0, "{stats:?}");
    }
}

#[test]
fn image_array_preserves_source_order_and_filtering() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let images = [
        ("red", &[255, 0, 0, 255]),
        ("green", &[0, 255, 0, 255]),
        ("blue", &[0, 0, 255, 255]),
    ];
    let scene = tree(
        1,
        images
            .iter()
            .zip([0.0, 4.0, 8.0])
            .map(|((source, _), x)| Command::Image {
                x,
                y: 0.0,
                w: 8.0,
                h: 8.0,
                source: Arc::from(*source),
                mode: ContentMode::Fill,
                required: true,
            })
            .collect::<Vec<_>>(),
    );
    raster
        .submit_layers(&scene, 1.0, |source| {
            let (_, rgba) = images.iter().find(|(name, _)| *name == source)?;
            Some(ImagePixels {
                width: 1,
                height: 1,
                rgba: rgba.as_slice(),
            })
        })
        .unwrap();
    let pixels = raster.render_rgba(PixelSize::new(16, 8)).unwrap();
    assert_eq!(pixel(&pixels, 16, 3, 3), *images[0].1);
    assert_eq!(pixel(&pixels, 16, 7, 3), *images[1].1);
    assert_eq!(pixel(&pixels, 16, 11, 3), *images[2].1);
    let gpu = &raster.device.as_ref().unwrap().gpu;
    if gpu.image_array()
        || gpu.max_texture_size() >= 1024
            && gpu.caps.max_array_layers >= 2
            && std::env::var("IMPASTO_SMALL_IMAGES").as_deref() != Ok("0")
    {
        assert_eq!(raster.submit_stats().render.draws, 1);
    }
}

#[test]
fn large_glyph_outlines_are_reused_across_scale_changes() {
    let fonts = FontLibrary::new();
    let mut shaper = ParleyShaper::new(fonts.clone());
    assert!(shaper.register("Sans", ROBOTO));
    let Some(mut raster) = headless(fonts) else {
        return;
    };
    let text = bench_text(&mut shaper, Arc::from("I"), 108.0, 150.0, false);
    let size = PixelSize::new(196, 196);
    let first = capture(&mut raster, 1, vec![text.clone()], size);
    let cold = raster.submit_stats();
    assert!(first.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 200));
    if std::env::var("IMPASTO_TEXT_MSDF").as_deref() == Ok("0") {
        assert!(cold.render.mesh_upload_bytes > 0);
    } else {
        assert!(cold.atlas.uploads > 0);
        assert_eq!(cold.render.offscreen_passes, 0);
    }
    let enlarged = capture(
        &mut raster,
        2,
        vec![
            Command::PushTransform {
                translate_x: 0.0,
                translate_y: 0.0,
                rotate_rad: 0.0,
                scale_x: 1.25,
                scale_y: 1.25,
            },
            text,
            Command::Pop,
        ],
        size,
    );
    assert_ne!(first, enlarged);
    let warm = raster.submit_stats();
    assert_eq!(warm.render.mesh_upload_bytes, 0);
    assert_eq!(warm.atlas.uploads, 0);
}

#[test]
fn display_p3_output_maps_linear_srgb_and_preserves_p3_gradient_stops() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let red = [255, 0, 0, 255];
    let mut p3 = rect([4.0, 0.0, 4.0, 4.0], red);
    if let Command::Rect { gradient, .. } = &mut p3 {
        *gradient = Some(Gradient::Linear {
            start: [0.0, 0.0],
            end: [1.0, 0.0],
            stops: [0.0, 1.0]
                .map(|offset| ColorStop {
                    offset,
                    color: red,
                    space: engine::scene::ColorSpace::DisplayP3,
                })
                .to_vec(),
        });
    }
    raster
        .submit_layers(&tree(1, [rect([0.0, 0.0, 4.0, 4.0], red), p3]), 1.0, |_| {
            None
        })
        .unwrap();
    let size = PixelSize::new(8, 4);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let gpu = raster.device.as_ref().unwrap().gpu.clone();
    let target = output(&gpu, format, size);
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: size.width,
        height: size.height,
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 2,
        alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
        view_formats: vec![],
        color_space: wgpu::SurfaceColorSpace::DisplayP3,
    };
    raster
        .encode_and_submit(
            &target.create_view(&Default::default()),
            crate::compositor::OutputFormat::surface(&config),
            size,
        )
        .unwrap();
    let bytes = read_rgba(&gpu, &target, size).unwrap();
    let rgb = pixel(&bytes, 8, 2, 2);
    assert!(
        rgb[0].abs_diff(234) <= 2 && rgb[1].abs_diff(51) <= 2 && rgb[2].abs_diff(35) <= 2,
        "{rgb:?}"
    );
    let wide = pixel(&bytes, 8, 6, 2);
    assert!(wide[0] >= 254 && wide[1] <= 4 && wide[2] <= 4, "{wide:?}");
}

#[test]
fn declarative_scroll_bindings_render_identical_pixels_on_ui_and_raster() {
    use engine::motion::{AnimProp, Easing, Tween};
    use engine::scene::{
        Extrapolate, PiecewiseColorMap, PiecewiseMap, ScrollCurve, ScrollSource, ScrollTargetProp,
    };
    use engine::text::MonoShaper;
    use engine::tree::Tree;

    let Some(mut compositor) = headless(FontLibrary::new()) else {
        return;
    };
    let Some(mut ui) = headless(FontLibrary::new()) else {
        return;
    };
    let size = PixelSize::new(80, 80);
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 80.0, 80.0).unwrap();
    let root = tree.stack().unwrap();
    tree.set_size(root, 80.0, 80.0).unwrap();
    let scroll = tree.scroll().unwrap();
    tree.set_max_size(scroll, 80.0, 80.0).unwrap();
    tree.add_child(root, scroll).unwrap();
    let list = tree.view().unwrap();
    tree.add_child(scroll, list).unwrap();
    for index in 0..10 {
        let color = if index % 2 == 0 {
            [255, 168, 94, 255]
        } else {
            [18, 77, 112, 255]
        };
        let row = tree.decorated(color, 0.0).unwrap();
        tree.set_size(row, 80.0, 40.0).unwrap();
        tree.add_child(list, row).unwrap();
    }
    let header = tree.sized(44.0, 30.0).unwrap();
    tree.add_child(root, header).unwrap();
    for (prop, output) in [
        (ScrollTargetProp::TranslateX, [0.0, 24.0]),
        (ScrollTargetProp::ScaleX, [1.0, 0.7]),
        (ScrollTargetProp::Rotate, [0.0, 0.11]),
        (ScrollTargetProp::Opacity, [1.0, 0.6]),
        (ScrollTargetProp::BlurSigma, [0.0, 1.0]),
        (ScrollTargetProp::BackdropBlurSigma, [0.0, 3.0]),
    ] {
        tree.add_scroll_binding(
            scroll,
            ScrollSource::Y,
            header,
            prop,
            ScrollCurve::Scalar(
                PiecewiseMap::new(Box::new([5.0, 175.0]), Box::new(output), Extrapolate::Clamp)
                    .unwrap(),
            ),
        )
        .unwrap();
    }
    tree.add_scroll_binding(
        scroll,
        ScrollSource::Y,
        header,
        ScrollTargetProp::BackgroundColor,
        ScrollCurve::Color(
            PiecewiseColorMap::new(
                Box::new([5.0, 175.0]),
                Box::new([[0.8, 0.24, 0.1, 0.8], [0.12, 0.35, 0.7, 0.7]]),
                Extrapolate::Clamp,
            )
            .unwrap(),
        ),
    )
    .unwrap();
    tree.layout().unwrap();
    tree.tick_motions(100.0);
    tree.set_scroll_offset(scroll, 0.0, 5.0).unwrap();
    tree.animate(
        scroll,
        AnimProp::ScrollY,
        Box::new(Tween {
            from: 5.0,
            to: 205.0,
            duration_ms: 400.0,
            easing: Easing::Linear,
        }),
    );
    tree.record_layers().unwrap();
    let installed = tree.layer_tree();
    assert_eq!(installed.tracks.len(), 1);
    assert_eq!(installed.tracks[0].scroll_bindings.len(), 7);
    compositor.submit_layers(&installed, 1.0, |_| None).unwrap();

    let mut previous = vec![];
    for target in [100.0, 135.0, 200.0, 370.0, 499.0] {
        compositor
            .sample_compositor_tracks(installed.epoch, target)
            .unwrap();
        let actual = compositor.render_rgba(size).unwrap();
        assert_ne!(previous, actual, "test scene must change at {target} ms");
        tree.tick_motions(target);
        tree.record_layers().unwrap();
        ui.submit_layers(&tree.layer_tree(), 1.0, |_| None).unwrap();
        assert!(
            actual == ui.render_rgba(size).unwrap(),
            "pixel mismatch at {target} ms"
        );
        previous = actual;
    }
}

#[test]
fn spatial_batches_preserve_overlapping_draws_across_transforms_and_clips() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let size = PixelSize::new(256, 64);
    let mut draws = Vec::new();
    for x in [10.0, 65.0, 120.0, 175.0] {
        draws.push(Command::PushTransform {
            translate_x: x,
            translate_y: 12.0,
            rotate_rad: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
        });
        draws.push(shadowed([0.0, 0.0, 22.0, 24.0]));
        draws.push(rounded([0.0, 0.0, 22.0, 24.0], 5.0));
        draws.push(rect([0.0, 0.0, 22.0, 24.0], [255, 44, 32, 168]));
        draws.push(Command::Pop);
        draws.push(Command::Pop);
    }
    let compositor = &mut raster.device.as_mut().unwrap().compositor;
    compositor.disable_residency();
    let spatial = compositor.set_spatial_batch(false);
    let expected = capture(&mut raster, 1, draws.clone(), size);
    let ordered = raster.submit_stats().render;
    raster
        .device
        .as_mut()
        .unwrap()
        .compositor
        .set_spatial_batch(spatial);
    let actual = capture(&mut raster, 2, draws, size);
    let batched = raster.submit_stats().render;
    assert_eq!(expected, actual);
    assert_eq!(batched.passes, 1);
    if spatial
        && raster.device.as_ref().unwrap().gpu.caps.paint_storage
        && std::env::var("IMPASTO_STATE_TABLE").as_deref() != Ok("0")
    {
        assert!(batched.draws < ordered.draws, "{batched:?}");
    }
}

#[test]
fn independent_offscreen_producers_keep_one_main_pass() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let size = PixelSize::new(384, 384);
    let mut draws = Vec::new();
    for index in 0u16..64 {
        let x = f32::from(index % 8) * 48.0 + 8.0;
        let y = f32::from(index / 8) * 48.0 + 8.0;
        draws.extend([
            Command::PushBlur {
                x,
                y,
                w: 24.0,
                h: 24.0,
                sigma: 1.0,
            },
            rect([x + 4.0, y + 4.0, 16.0, 16.0], [255, 50, 30, 255]),
            Command::Pop,
        ]);
    }
    let graph = raster
        .device
        .as_mut()
        .unwrap()
        .compositor
        .set_render_graph(false);
    let expected = capture(&mut raster, 1, draws.clone(), size);
    let unplanned = raster.submit_stats().render;
    raster
        .device
        .as_mut()
        .unwrap()
        .compositor
        .set_render_graph(graph);
    let actual = capture(&mut raster, 2, draws, size);
    let planned = raster.submit_stats().render;
    assert_eq!(expected, actual);
    if graph {
        assert_eq!(planned.passes - planned.offscreen_passes, 1);
        assert!(unplanned.passes > planned.passes);
    }
}

#[test]
fn disjoint_stencil_fills_share_writes_and_covers() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let ops = (0u16..5)
        .map(|index| {
            let x = f32::from(index) * 16.0;
            let mut verbs = Vec::new();
            for (left, right) in [(x + 1.0, x + 14.0), (x + 4.0, x + 11.0)] {
                let center = (left + right) * 0.5;
                verbs.extend([
                    PathVerb::Move(left, 1.0),
                    PathVerb::Line(center, 1.0),
                    PathVerb::Quad(right, 1.0, right, 4.0),
                    PathVerb::Line(right, 8.0),
                    PathVerb::Line(right, 13.0),
                    PathVerb::Line(center, 13.0),
                    PathVerb::Line(left, 13.0),
                    PathVerb::Line(left, 8.0),
                    PathVerb::Close,
                ]);
            }
            fill_path(
                CanvasPath {
                    fill: FillRule::EvenOdd,
                    verbs: verbs.into(),
                },
                [20, 210, 20, 255],
                CanvasComposite::SourceOver,
            )
        })
        .collect();
    let pixels = capture(
        &mut raster,
        1,
        vec![canvas(ops, 80.0, 16.0)],
        PixelSize::new(80, 16),
    );
    assert_eq!(pixel(&pixels, 80, 2, 8), [20, 210, 20, 255]);
    assert_eq!(pixel(&pixels, 80, 7, 8), [0; 4]);
    let stats = raster.submit_stats().render;
    if std::env::var("IMPASTO_CANVAS_STENCIL").as_deref() != Ok("0") {
        assert_eq!((stats.stencil_draws, stats.stencil_batches), (5, 1));
    }
}

#[test]
fn global_depth_keeps_translucent_edges_in_painter_order() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let size = PixelSize::new(80, 80);
    let mut back = rect([2.0, 2.0, 76.0, 76.0], [14, 100, 80, 255]);
    let mut front = rect([7.25, 11.0, 66.0, 62.0], [20, 40, 180, 255]);
    for command in [&mut back, &mut front] {
        if let Command::Rect { corner_radius, .. } = command {
            *corner_radius = 8.0;
        }
    }
    let commands = vec![
        back,
        Command::PushColorFilter {
            matrix: Arc::new([
                [1.0, 0.0, 0.0, 0.0, 0.0],
                [0.0, 0.9, 0.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0, 0.0],
            ]),
        },
        rect([3.0, 20.0, 76.0, 8.0], [255, 190, 0, 150]),
        Command::Pop,
        front,
        canvas(
            vec![fill_path(
                path([10.0, 11.0, 10.0, 6.0]),
                [150, 0, 0, 120],
                CanvasComposite::SourceOver,
            )],
            80.0,
            80.0,
        ),
        rect([10.0, 10.0, 10.0, 12.0], [220, 10, 0, 120]),
    ];
    let compositor = &mut raster.device.as_mut().unwrap().compositor;
    if !compositor.force_early_depth() {
        return;
    }
    compositor.set_render_graph(false);
    let expected = capture(&mut raster, 1, commands.clone(), size);
    assert_eq!(raster.submit_stats().render.early_z_instances, 0);
    raster
        .device
        .as_mut()
        .unwrap()
        .compositor
        .set_render_graph(true);
    let actual = capture(&mut raster, 2, commands, size);
    assert!(expected.iter().zip(actual).all(|(a, b)| a.abs_diff(b) <= 2));
    let stats = raster.submit_stats().render;
    assert_eq!(stats.passes - stats.offscreen_passes, 1);
    assert_eq!(stats.early_z_instances, 2);
}

#[test]
fn doubled_convex_outline_respects_winding_and_alpha() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let color = [218, 102, 15, 128];
    let size = PixelSize::new(24, 24);
    let outline = [
        PathVerb::Line(18.0, 4.0),
        PathVerb::Line(18.0, 18.0),
        PathVerb::Line(4.0, 18.0),
        PathVerb::Line(4.0, 4.0),
    ];
    for (id, fill) in [(1, FillRule::EvenOdd), (2, FillRule::NonZero)] {
        let path = CanvasPath {
            fill,
            verbs: std::iter::once(PathVerb::Move(4.0, 4.0))
                .chain(outline)
                .chain(outline)
                .chain(std::iter::once(PathVerb::Close))
                .collect(),
        };
        let pixels = capture(
            &mut raster,
            id,
            vec![canvas(
                vec![fill_path(path, color, CanvasComposite::SourceOver)],
                24.0,
                24.0,
            )],
            size,
        );
        let alpha = pixel(&pixels, 24, 8, 8)[3];
        assert_eq!(
            alpha,
            if fill == FillRule::EvenOdd {
                0
            } else {
                color[3]
            }
        );
    }
}
