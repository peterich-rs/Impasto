use std::sync::Arc;

use engine::scene::{
    CanvasBody, CanvasBrush, CanvasComposite, CanvasIsolation, CanvasList, CanvasOp, CanvasPaint,
    CanvasPath, Command, ContentMode, FillRule, Layer, LayerId, LayerKind, LayerTree, PathVerb,
    Picture, PictureId, TextStyle,
};
use engine::text::{Shaper, metrics_key};

use super::{FontLibrary, ImagePixels, ParleyShaper, PixelSize, RasterOptions, Rasterizer, Status};

pub(crate) const ROBOTO: &[u8] = include_bytes!("../../embedder/android/assets/fonts/Sans.ttf");

pub(crate) fn headless(fonts: FontLibrary) -> Option<Rasterizer> {
    match Rasterizer::headless(RasterOptions::default(), fonts) {
        Ok(raster) => Some(raster),
        Err(Status::NoDevice(backend)) if std::env::var_os("IMPASTO_TEST_GPU").is_none() => {
            eprintln!("headless {backend:?} unavailable");
            None
        }
        Err(error) => panic!("headless renderer: {error}"),
    }
}

fn tree(generation: u64, commands: impl Into<Arc<[Command]>>) -> LayerTree {
    let picture = PictureId(1);
    LayerTree {
        layers: vec![Layer {
            kind: LayerKind::Picture { picture },
            parent: None,
            first_child: None,
            next: None,
        }],
        pictures: vec![Arc::new(Picture::new(
            picture,
            generation,
            (0.0, 0.0),
            commands.into(),
        ))],
        root: LayerId(0),
        epoch: generation,
        ..LayerTree::empty()
    }
}

fn rect(rect: [f32; 4], color: [u8; 4]) -> Command {
    let [x, y, w, h] = rect;
    Command::Rect {
        x,
        y,
        w,
        h,
        color,
        corner_radius: 0.0,
        stroke_width: 0.0,
        stroke_color: [0; 4],
        gradient: None,
        shadow: None,
        box_shadows: Arc::from([]),
    }
}

fn pixel(rgba: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    let offset = (y * width + x) * 4;
    rgba[offset..offset + 4]
        .try_into()
        .expect("pixel is in bounds")
}

#[test]
fn rgba_readback_is_tight_and_submit_is_atomic() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let scene = tree(1, [rect([0.0, 0.0, 3.0, 1.0], [255, 0, 0, 128])]);
    raster
        .submit_layers(&scene, 1.0, |_| None)
        .expect("valid scene");
    let size = PixelSize::new(3, 2);
    let rgba = raster.render_rgba(size).expect("odd-width readback");
    assert_eq!(
        rgba,
        [[255, 0, 0, 128]; 3]
            .concat()
            .into_iter()
            .chain([0; 12])
            .collect::<Vec<_>>()
    );

    let reused = raster
        .submit_layers(&scene, 1.0, |_| None)
        .expect("reused scene");
    assert_eq!(reused.stats.picture_reused, 1);
    let bad = tree(2, [Command::Pop]);
    assert_eq!(
        raster.submit_layers(&bad, 1.0, |_| None).unwrap_err(),
        Status::BadScene
    );
    assert_eq!(raster.render_rgba(size).expect("last good frame"), rgba);

    let mut cycle = scene.clone();
    cycle.layers[0].next = Some(cycle.root);
    assert_eq!(
        raster.submit_layers(&cycle, 1.0, |_| None).unwrap_err(),
        Status::BadScene
    );
}

#[test]
fn canvas_clear_and_group_opacity_respect_the_parent_scene() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let canvas = CanvasList {
        body: Arc::new(CanvasBody {
            isolation: CanvasIsolation::Isolated,
            bytes: 0,
            images: Box::new([]),
            ops: Box::new([
                CanvasOp::Fill {
                    path: CanvasPath {
                        fill: FillRule::EvenOdd,
                        verbs: Arc::from([
                            PathVerb::Move(0.0, 0.0),
                            PathVerb::Line(12.0, 0.0),
                            PathVerb::Line(0.0, 12.0),
                            PathVerb::Close,
                        ]),
                    },
                    paint: CanvasPaint {
                        brush: CanvasBrush::Color([0, 255, 0, 255]),
                        alpha: 255,
                        composite: CanvasComposite::SourceOver,
                    },
                },
                CanvasOp::Clear {
                    x: 2.0,
                    y: 2.0,
                    w: 3.0,
                    h: 3.0,
                    ctm: super::compiler::IDENTITY,
                },
            ]),
        }),
    };
    let scene = tree(
        1,
        [
            Command::Fill {
                color: [0, 0, 255, 255],
            },
            Command::PushClip {
                x: 0.0,
                y: 0.0,
                w: 12.0,
                h: 12.0,
                corner_radius: 3.0,
            },
            Command::PushOpacity {
                x: 0.0,
                y: 0.0,
                w: 12.0,
                h: 12.0,
                alpha: 128,
            },
            Command::Canvas {
                x: 0.0,
                y: 0.0,
                width: 12.0,
                height: 12.0,
                list: canvas,
            },
            Command::Pop,
            Command::Pop,
        ],
    );
    raster
        .submit_layers(&scene, 1.0, |_| None)
        .expect("canvas scene");
    let rgba = raster
        .render_rgba(PixelSize::new(13, 12))
        .expect("canvas readback");
    assert_eq!(
        pixel(&rgba, 13, 3, 3),
        [0, 0, 255, 255],
        "hole reveals the UI behind the canvas"
    );
    assert_eq!(
        pixel(&rgba, 13, 11, 11),
        [0, 0, 255, 255],
        "outside the triangle"
    );
    let blended = pixel(&rgba, 13, 6, 2);
    assert_eq!(blended[0], 0);
    assert!(
        (185..=190).contains(&blended[1]) && (185..=190).contains(&blended[2]),
        "{blended:?}"
    );
}

#[test]
fn nine_slice_retains_corners_and_rejects_overlapping_source_edges() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let make_scene = |generation, right, width| {
        tree(
            generation,
            [Command::Image {
                x: 0.0,
                y: 0.0,
                w: width,
                h: 3.0,
                source: Arc::from("slices"),
                mode: ContentMode::NineSlice {
                    left: 1.0,
                    top: 0.0,
                    right,
                    bottom: 0.0,
                },
                required: true,
            }],
        )
    };
    let red = [255, 0, 0, 255];
    let green = [0, 255, 0, 255];
    let blue = [0, 0, 255, 255];
    let pixels = [red, green, blue].concat();
    let resolve = |_: &str| {
        Some(ImagePixels {
            width: 3,
            height: 1,
            rgba: &pixels,
        })
    };
    raster
        .submit_layers(&make_scene(1, 1.0, 7.0), 1.0, resolve)
        .expect("valid slices");
    let size = PixelSize::new(7, 3);
    let rgba = raster.render_rgba(size).expect("nine-slice capture");
    assert_eq!(pixel(&rgba, 7, 0, 1), red);
    assert_eq!(pixel(&rgba, 7, 3, 1), green);
    assert_eq!(pixel(&rgba, 7, 6, 1), blue);
    assert_eq!(
        raster
            .submit_layers(&make_scene(2, 3.0, 7.0), 1.0, resolve)
            .unwrap_err(),
        Status::BadScene
    );
    assert_eq!(raster.render_rgba(size).expect("preserved frame"), rgba);

    raster
        .submit_layers(&make_scene(3, 1.0, 1.0), 1.0, resolve)
        .expect("small destination falls back to Fill");
    let rgba = raster.render_rgba(size).expect("Fill capture");
    assert_eq!(pixel(&rgba, 7, 0, 1), green);
}

#[test]
fn backdrop_blur_reads_prior_draws_and_leaves_outside_pixels_intact() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let black = [0, 0, 0, 255];
    let white = [255, 255, 255, 255];
    let scene = tree(
        1,
        [
            Command::Fill { color: black },
            rect([0.0, 0.0, 8.0, 12.0], white),
            Command::PushBackdropBlur {
                x: 4.0,
                y: 3.0,
                w: 8.0,
                h: 6.0,
                sigma: 2.0,
            },
            Command::Pop,
        ],
    );
    raster
        .submit_layers(&scene, 1.0, |_| None)
        .expect("backdrop scene");
    let rgba = raster
        .render_rgba(PixelSize::new(16, 12))
        .expect("backdrop readback");
    assert_eq!(pixel(&rgba, 16, 7, 1), white);
    assert_eq!(pixel(&rgba, 16, 8, 1), black);
    for x in [7, 8] {
        let [r, g, b, a] = pixel(&rgba, 16, x, 6);
        assert!(r > 0 && r < 255, "blur at {x} should mix both sides");
        assert_eq!([g, b, a], [r, r, 255]);
    }
}

#[test]
fn frames_reuse_glyphs_and_reject_uploads_from_another_device() {
    let fonts = FontLibrary::new();
    let mut shaper = ParleyShaper::new(fonts.clone());
    assert!(shaper.register("Sans", ROBOTO));
    let key = metrics_key(
        Arc::from("Ag"),
        Arc::new(TextStyle::default()),
        &[],
        22.0,
        60.0,
    );
    let shaped = shaper.metrics(&key).shaped.clone().expect("frozen glyphs");
    let Some(mut raster) = headless(fonts.clone()) else {
        return;
    };
    let size = PixelSize::new(61, 30);
    for (generation, color) in [(1, [255, 0, 0, 255]), (2, [0, 255, 0, 255])] {
        let scene = tree(
            generation,
            [Command::Text {
                x: 1.0,
                y: 1.0,
                shaped: Some(Arc::clone(&shaped)),
                color,
                span_paints: Arc::from([]),
            }],
        );
        raster
            .submit_layers(&scene, 1.0, |_| None)
            .expect("text scene");
        let rgba = raster.render_rgba(size).expect("text readback");
        let channel = usize::from(generation == 2);
        assert!(
            rgba.as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[channel] > 240 && pixel[3] > 128)
        );
        let atlas = raster.submit_stats().atlas;
        if generation == 1 {
            assert!(atlas.uploads >= 2);
        } else {
            assert!(atlas.hits >= 2);
            assert_eq!(atlas.uploads, 0);
        }
    }

    let stale = raster
        .texture_factory()
        .expect("first device")
        .upload_rgba(1, 1, &[255, 0, 0, 255])
        .expect("upload");
    raster.device.as_ref().expect("device").gpu.device.destroy();
    assert_eq!(raster.render_rgba(size).unwrap_err(), Status::DeviceLost);
    assert!(raster.texture_factory().is_none());
    let Some(mut replacement) = headless(fonts) else {
        panic!("replacement device")
    };
    let image = "pixel";
    replacement.install_uploaded(image, &stale);
    let scene = tree(
        3,
        [Command::Image {
            x: 0.0,
            y: 0.0,
            w: 3.0,
            h: 3.0,
            source: Arc::from(image),
            mode: ContentMode::Fill,
            required: true,
        }],
    );
    replacement
        .submit_layers(&scene, 1.0, |_| {
            Some(ImagePixels {
                width: 1,
                height: 1,
                rgba: &[0, 255, 0, 255],
            })
        })
        .expect("resubmitted image");
    let rgba = replacement
        .render_rgba(PixelSize::new(3, 3))
        .expect("recovered readback");
    assert_eq!(pixel(&rgba, 3, 1, 1), [0, 255, 0, 255]);
}

mod p0;

#[test]
fn odd_image_mips_average_linear_premultiplied_texels() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    const SIDE: u32 = 129;
    let width = SIDE as usize;
    let mut image = vec![[0, 0, 255, 0]; width * width];
    for (index, pixel) in image.iter_mut().enumerate() {
        if index % width % 3 == 1 && index / width % 3 == 1 {
            *pixel = [255, 0, 0, 255];
        }
    }
    let levels = width.ilog2() + 1;
    let scene = tree(
        1,
        [Command::Image {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            source: Arc::from("checker"),
            mode: ContentMode::Fill,
            required: true,
        }],
    );
    raster
        .submit_layers(&scene, 1.0, |_| {
            Some(ImagePixels {
                width: SIDE,
                height: SIDE,
                rgba: image.as_flattened(),
            })
        })
        .unwrap();
    let stats = raster.submit_stats().image_upload;
    assert_eq!(stats.render_passes, levels, "{stats:?}");
    let pixel = raster.render_rgba(PixelSize::new(1, 1)).unwrap();
    assert!(
        pixel[0] >= 250 && pixel[1] == 0 && pixel[2] == 0 && pixel[3].abs_diff(28) <= 1,
        "{pixel:?}"
    );
}

#[test]
fn three_texel_mip_keeps_the_two_edge_texels() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let mut pixels = [[0; 4]; 192];
    pixels[..64].fill([255, 0, 0, 255]);
    let scene = tree(
        1,
        [Command::Image {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            source: Arc::from("left-third"),
            mode: ContentMode::Fill,
            required: true,
        }],
    );
    raster
        .submit_layers(&scene, 1.0, |_| {
            Some(ImagePixels {
                width: 192,
                height: 1,
                rgba: pixels.as_flattened(),
            })
        })
        .unwrap();
    assert_eq!(
        raster.render_rgba(PixelSize::new(1, 1)).unwrap(),
        [255, 0, 0, 85]
    );
}

#[test]
fn compute_blur_writes_bounded_output_and_counts_both_axes() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let size = PixelSize::new(112, 104);
    let scene = tree(
        1,
        [
            Command::PushBlur {
                x: 16.0,
                y: 16.0,
                w: 79.0,
                h: 71.0,
                sigma: 3.0,
            },
            rect([16.0, 16.0, 79.0, 71.0], [5, 5, 5, 255]),
            Command::Pop,
        ],
    );
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    let image = raster.render_rgba(size).unwrap();
    let stats = raster.submit_stats().render;
    let gpu = &raster.device.as_ref().unwrap().gpu;
    if gpu.caps.has_compute
        && gpu
            .caps
            .downlevel_flags
            .contains(wgpu::DownlevelFlags::VIEW_FORMATS)
    {
        assert_eq!(stats.compute_passes, 2, "{stats:?}");
    }
    assert_eq!(pixel(&image, 112, 0, 0), [0; 4]);
    assert_eq!(pixel(&image, 112, 55, 51), [5, 5, 5, 255]);
    let alpha = |x, y| pixel(&image, 112, x, y)[3];
    assert_eq!(alpha(15, 51), alpha(55, 15));
    assert_eq!(alpha(95, 51), alpha(55, 87));
}

#[test]
fn staged_texture_replacements_are_published_only_after_valid_submission() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let scene = tree(
        1,
        [Command::Image {
            x: 0.0,
            y: 0.0,
            w: 4.0,
            h: 4.0,
            source: Arc::from("preview"),
            mode: ContentMode::Fill,
            required: true,
        }],
    );
    let size = PixelSize::new(4, 4);
    raster
        .submit_layers(&scene, 1.0, |_| {
            Some(ImagePixels {
                width: 1,
                height: 1,
                rgba: &[255, 0, 0, 255],
            })
        })
        .unwrap();
    let green = raster
        .texture_factory()
        .unwrap()
        .upload_rgba(1, 1, &[0, 255, 0, 255])
        .unwrap();
    raster.install_uploaded("preview", &green);
    let mut rejected = scene.clone();
    rejected.layers[0].kind = LayerKind::Picture {
        picture: PictureId(100),
    };
    assert_eq!(
        raster.submit_layers(&rejected, 1.0, |_| None).unwrap_err(),
        Status::BadScene
    );
    assert_eq!(
        pixel(&raster.render_rgba(size).unwrap(), 4, 2, 2),
        [255, 0, 0, 255]
    );
    raster.submit_layers(&scene, 1.0, |_| None).unwrap();
    assert_eq!(
        pixel(&raster.render_rgba(size).unwrap(), 4, 2, 2),
        [0, 255, 0, 255]
    );
}

mod gpu_extensions;

#[test]
fn retained_paint_slots_change_pixels_without_picture_compilation() {
    use engine::scene::SceneChanges;
    use engine::text::MonoShaper;
    use engine::tree::Tree;

    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let mut ui = Tree::new(MonoShaper::default());
    let rect = ui.decorated([255, 0, 0, 255], 0.0).unwrap();
    ui.set_size(rect, 20.0, 20.0).unwrap();
    ui.layout().unwrap();
    ui.record_layers().unwrap();
    ui.clear_paint();
    ui.set_color(rect, [0, 255, 0, 255]).unwrap();
    ui.record_layers().unwrap();
    ui.clear_paint();
    let base = ui.layer_tree();
    raster.submit_layers(&base, 1.0, |_| None).unwrap();
    let size = PixelSize::new(64, 64);
    let green = raster.render_rgba(size).unwrap();
    assert_eq!(&green[(4 * 64 + 4) * 4..][..4], &[0, 255, 0, 255]);

    ui.set_color(rect, [0, 0, 255, 255]).unwrap();
    ui.record_layers().unwrap();
    let delta = ui.take_scene_delta().unwrap();
    assert_eq!(delta.changes, SceneChanges::LayerValues);
    assert_eq!(ui.layer_record_visits(), 0);
    let next = Arc::new(ui.layer_tree());
    let stats = raster
        .submit_layer_delta(&next, 1.0, &delta)
        .unwrap()
        .unwrap()
        .stats;
    assert_eq!(stats.picture_rebuilt, 0);
    assert!(matches!(stats.damage, crate::DamageRect::Rect(_)));
    let blue = raster.render_rgba(size).unwrap();
    assert_eq!(&blue[(4 * 64 + 4) * 4..][..4], &[0, 0, 255, 255]);
    assert!(
        raster
            .submit_layer_delta(&next, 1.0, &delta)
            .unwrap()
            .is_none()
    );
}

#[test]
fn reusable_canvas_path_keeps_even_odd_holes_through_transforms() {
    let Some(mut raster) = headless(FontLibrary::new()) else {
        return;
    };
    let asset = CanvasPath {
        verbs: Arc::from([
            PathVerb::Move(0.0, 0.0),
            PathVerb::Line(20.0, 0.0),
            PathVerb::Line(20.0, 20.0),
            PathVerb::Line(0.0, 20.0),
            PathVerb::Close,
            PathVerb::Move(6.0, 6.0),
            PathVerb::Line(14.0, 6.0),
            PathVerb::Line(14.0, 14.0),
            PathVerb::Line(6.0, 14.0),
            PathVerb::Close,
        ]),
        fill: FillRule::EvenOdd,
    };
    let size = PixelSize::new(64, 64);
    for (generation, translate, scale) in [(1, 4.0, 1.0), (2, 4.0, 1.0), (3, 10.0, 1.5)] {
        let transform = [scale, 0.0, 0.0, scale, translate, translate];
        let mapped = CanvasPath {
            verbs: asset
                .verbs
                .iter()
                .map(|verb| {
                    let coord = |value| value * scale + translate;
                    match *verb {
                        PathVerb::Move(x, y) => PathVerb::Move(coord(x), coord(y)),
                        PathVerb::Line(x, y) => PathVerb::Line(coord(x), coord(y)),
                        other => other,
                    }
                })
                .collect(),
            fill: asset.fill,
        };
        let command = Command::Canvas {
            x: 0.0,
            y: 0.0,
            width: 64.0,
            height: 64.0,
            list: CanvasList {
                body: Arc::new(CanvasBody {
                    ops: Box::new([CanvasOp::Vector {
                        path: mapped,
                        asset: asset.clone(),
                        transform,
                        paint: CanvasPaint {
                            brush: CanvasBrush::Color([255, 0, 0, 255]),
                            alpha: 255,
                            composite: CanvasComposite::SourceOver,
                        },
                    }]),
                    images: Box::new([]),
                    isolation: CanvasIsolation::Shared,
                    bytes: 0,
                }),
            },
        };
        raster
            .submit_layers(&tree(generation, [command]), 1.0, |_| None)
            .unwrap();
        let pixels = raster.render_rgba(size).unwrap();
        let (solid, hole) = if generation < 3 { (7, 14) } else { (14, 25) };
        assert_eq!(pixels[(solid * 64 + solid) * 4 + 3], 255);
        assert_eq!(pixels[(hole * 64 + hole) * 4 + 3], 0);
    }
}
