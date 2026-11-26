//! Render PNG comparisons using the same layer tree and font library as the session.
use std::{fs, io::BufReader, path::Path, sync::Arc};

use anyhow::{Context, Result, bail};
use engine::scene::LayerTree;
use raster::{FontLibrary, PixelSize, RasterOptions, Rasterizer};

use super::SnapshotOptions;

pub(super) struct Snapshots {
    options: SnapshotOptions,
    raster: Option<Rasterizer>,
    pub(super) fonts: FontLibrary,
    pub(super) layers: Option<Arc<LayerTree>>,
}

impl Snapshots {
    pub(super) fn new(options: SnapshotOptions) -> Self {
        Self {
            options,
            raster: None,
            fonts: FontLibrary::new(),
            layers: None,
        }
    }

    pub(super) fn compare(&mut self, name: &Path) -> Result<()> {
        let name = name
            .to_str()
            .filter(|name| {
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            })
            .context("snapshot names use letters, digits, dashes and underscores")?;
        let layers = self
            .layers
            .as_ref()
            .context("mount a screen before taking a snapshot")?;
        if self.raster.is_none() {
            self.raster = Some(Rasterizer::headless(
                RasterOptions::default(),
                self.fonts.clone(),
            )?);
        }
        let raster = self.raster.as_mut().context("no headless raster device")?;
        raster.submit_shared_layers(Arc::clone(layers), 2., |_| None)?;
        let size = PixelSize::new(720, 1280);
        let rgba = raster.render_rgba(size)?;
        let expected = self.options.root.join(format!("{name}.png"));
        if self.options.update {
            fs::create_dir_all(&self.options.root)?;
            return write_png(&expected, size, &rgba);
        }
        let reference = fs::File::open(&expected)
            .with_context(|| format!("read reference {}", expected.display()))?;
        let mut png = png::Decoder::new(BufReader::new(reference)).read_info()?;
        let info = png.info();
        if (info.width, info.height, info.color_type, info.bit_depth)
            != (
                size.width,
                size.height,
                png::ColorType::Rgba,
                png::BitDepth::Eight,
            )
        {
            bail!(
                "reference {} must be {}×{} RGBA8",
                expected.display(),
                size.width,
                size.height
            );
        }
        let mut reference = vec![0; rgba.len()];
        let result = png.next_frame(&mut reference)?;
        if reference[..result.buffer_size()] != rgba {
            let actual = self.options.root.join(format!("{name}.actual.png"));
            write_png(&actual, size, &rgba)?;
            bail!(
                "pixels differ from {}; actual frame saved to {}",
                expected.display(),
                actual.display()
            );
        }
        Ok(())
    }
}

fn write_png(path: &Path, size: PixelSize, rgba: &[u8]) -> Result<()> {
    let file = fs::File::create(path)?;
    let mut encoder = png::Encoder::new(file, size.width, size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(())
}
