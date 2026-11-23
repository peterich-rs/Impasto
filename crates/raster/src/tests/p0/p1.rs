use super::*;

#[test]
#[ignore = "GPU experiment matrix"]
fn blur_matrix() {
    let size = PixelSize::new(640, 480);
    let mut raster = benchmark::headless(FontLibrary::new());
    for [width, height] in [[64u16, 64u16], [208, 160], [560, 408]] {
        for sigma in [2u16, 8, 24] {
            let (w, h) = (f32::from(width), f32::from(height));
            let name = format!("blur_{width}x{height}_sigma_{sigma}");
            benchmark::run(&mut raster, &name, 31, size, |frame| {
                tree(
                    u64::from(frame),
                    [
                        Command::Fill {
                            color: [26, 28, 32, 255],
                        },
                        rect([0.0, 0.0, 320.0, 480.0], [245, 223, 180, 255]),
                        Command::PushBackdropBlur {
                            x: (640.0 - w) * 0.5,
                            y: (480.0 - h) * 0.5,
                            w,
                            h,
                            sigma: f32::from(sigma),
                        },
                        Command::Pop,
                    ],
                )
            });
        }
    }
}

#[test]
#[ignore = "GPU experiment"]
fn opaque_overlap() {
    let size = PixelSize::new(1024, 768);
    let commands: Arc<[Command]> = (0..64u8)
        .map(|index| {
            let x = f32::from(index % 8) * 32.0;
            let y = f32::from(index / 8) * 24.0;
            let color = [index * 3, 85, 190, 255];
            rect([x, y, 800.0, 600.0], color)
        })
        .collect();
    let mut raster = benchmark::headless(FontLibrary::new());
    benchmark::run(&mut raster, "opaque_overlap", 61, size, |frame| {
        tree(u64::from(frame), Arc::clone(&commands))
    });
}

#[test]
#[ignore = "GPU experiment"]
fn gradient_wall() {
    let size = PixelSize::new(1024, 768);
    let mut raster = benchmark::headless(FontLibrary::new());
    for count in [8u8, 32] {
        let color = |index: u8| [index.wrapping_mul(42), index.wrapping_mul(5), 125, 255];
        let gradient = Gradient::Linear {
            start: [0.0, 0.0],
            end: [1.0, 0.0],
            stops: (0..count)
                .map(|i| ColorStop::srgb(f32::from(i) / f32::from(count - 1), color(i)))
                .collect(),
        };
        let commands: Arc<[Command]> = (0..24u16)
            .map(|i| {
                let x = f32::from(i % 4) * 256.0;
                let y = f32::from(i / 4) * 128.0;
                let mut command = rect([x, y, 256.0, 128.0], [255; 4]);
                if let Command::Rect {
                    gradient: brush, ..
                } = &mut command
                {
                    *brush = Some(gradient.clone());
                }
                command
            })
            .collect();
        benchmark::run(
            &mut raster,
            &format!("gradient_{count}_stops"),
            31,
            size,
            |frame| tree(u64::from(frame), Arc::clone(&commands)),
        );
    }
}

#[test]
#[ignore = "GPU experiment"]
fn upload_image() {
    let [width, height] = [2049u32, 1537u32];
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            rgba.extend([
                x.to_le_bytes()[0],
                y.to_le_bytes()[0],
                127,
                (x ^ y).to_le_bytes()[0],
            ]);
        }
    }
    let mut raster = benchmark::headless(FontLibrary::new());
    benchmark::run_with_images(
        &mut raster,
        "image_upload_2049x1537",
        13,
        PixelSize::new(512, 384),
        raster_image_scene,
        ImagePixels {
            width,
            height,
            rgba: &rgba,
        },
    );
}

fn raster_image_scene(frame: u32) -> LayerTree {
    tree(
        u64::from(frame),
        [Command::Image {
            x: 0.0,
            y: 0.0,
            w: 512.0,
            h: 384.0,
            source: Arc::from(format!("upload-{frame}")),
            mode: ContentMode::Fill,
            required: true,
        }],
    )
}
