//! ImageIO paths that match the previous macOS host.
//!
//! Logo pixels are tightly packed RGBA (`PremultipliedLast` + 32-bit big endian).
//! Frame dumps are the Metal texture's BGRA bytes (`PremultipliedFirst` + 32-bit little endian).

use std::path::Path;
use std::ptr;

use anyhow::Context;
use objc2_core_foundation::{CFData, CFString, CFURL, CFURLPathStyle, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGContext,
    CGDataProvider, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo,
};
use objc2_image_io::{CGImageDestination, CGImageSource};

pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Channel tolerance against a reference image. A pixel whose reference value
/// differs from a 4-neighbor by more than 32 in any channel is an edge and
/// allows ±6. Every other pixel allows ±2. A size mismatch fails.
#[cfg_attr(not(test), allow(dead_code))]
pub fn pixels_close(actual: &[u8], expected: &[u8], width: u32, height: u32) -> bool {
    let Some(count) = (width as usize).checked_mul(height as usize) else {
        return false;
    };
    let Some(len) = count.checked_mul(4) else {
        return false;
    };
    if width == 0 || height == 0 || actual.len() != len || expected.len() != len {
        return false;
    }
    for index in 0..count {
        let allowance = if edge_pixel(expected, width, index) {
            6
        } else {
            2
        };
        let at = index * 4;
        for channel in 0..4 {
            let delta = (i16::from(actual[at + channel]) - i16::from(expected[at + channel]))
                .unsigned_abs();
            if delta > allowance {
                return false;
            }
        }
    }
    true
}

#[cfg_attr(not(test), allow(dead_code))]
fn edge_pixel(expected: &[u8], width: u32, index: usize) -> bool {
    let x = index % width as usize;
    let y = index / width as usize;
    let center = index * 4;
    let neighbors = [
        x.checked_sub(1).map(|nx| (nx, y)),
        Some((x + 1, y)),
        y.checked_sub(1).map(|ny| (x, ny)),
        Some((x, y + 1)),
    ];
    for neighbor in neighbors.into_iter().flatten() {
        let (nx, ny) = neighbor;
        if nx >= width as usize {
            continue;
        }
        let at = ny * width as usize + nx;
        if at * 4 + 4 > expected.len() {
            continue;
        }
        let at = at * 4;
        for channel in 0..4 {
            let delta = (i16::from(expected[center + channel]) - i16::from(expected[at + channel]))
                .unsigned_abs();
            if delta > 32 {
                return true;
            }
        }
    }
    false
}

pub fn load_rgba_png(path: &Path) -> anyhow::Result<Rgba> {
    let url = file_url(path)?;
    // SAFETY: `url` is a file URL. A null options dictionary is the documented default.
    let source = unsafe { CGImageSource::with_url(&url, None) }
        .with_context(|| format!("failed to read logo {}", path.display()))?;
    rgba_from_source(&source, &path.display().to_string())
}

/// Decode PNG, JPEG, or WebP bytes the same way as a file: device RGB, premultiplied.
pub fn load_rgba_bytes(bytes: &[u8]) -> anyhow::Result<Rgba> {
    let data = CFData::from_bytes(bytes);
    // SAFETY: `data` retains `bytes` for the source. A null options dictionary is the default.
    let source =
        unsafe { CGImageSource::with_data(&data, None) }.context("image bytes were not decoded")?;
    rgba_from_source(&source, "image bytes")
}

fn rgba_from_source(source: &CGImageSource, label: &str) -> anyhow::Result<Rgba> {
    // SAFETY: index 0 is the first image. A null options dictionary keeps the source pixels.
    let image = unsafe { source.image_at_index(0, None) }
        .with_context(|| format!("failed to read {label}"))?;
    let width = CGImage::width(Some(&image));
    let height = CGImage::height(Some(&image));
    if width == 0 || height == 0 {
        anyhow::bail!("failed to read {label}");
    }
    let row = width.checked_mul(4).context("logo is too wide")?;
    let len = row.checked_mul(height).context("logo is too large")?;
    let mut pixels = vec![0u8; len];
    let space = CGColorSpace::new_device_rgb().context("device RGB is unavailable")?;
    let info = CGImageAlphaInfo::PremultipliedLast.0 | CGImageByteOrderInfo::Order32Big.0;
    // SAFETY: `pixels` is `len` bytes and outlives `context`. The bitmap is 8 bits per channel.
    let context = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            width,
            height,
            8,
            row,
            Some(&space),
            info,
        )
    }
    .with_context(|| format!("failed to read {label}"))?;
    let rect = CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(width as f64, height as f64),
    );
    CGContext::draw_image(Some(&context), rect, Some(&image));
    drop(context);
    Ok(Rgba {
        width: u32::try_from(width).context("logo width does not fit u32")?,
        height: u32::try_from(height).context("logo height does not fit u32")?,
        pixels,
    })
}

pub fn write_bgra_png(path: &Path, width: usize, height: usize, bgra: &[u8]) -> anyhow::Result<()> {
    let row = width.checked_mul(4).context("frame is too wide")?;
    let len = row.checked_mul(height).context("frame is too large")?;
    if bgra.len() != len || width == 0 || height == 0 {
        anyhow::bail!("failed to write {}", path.display());
    }
    let data = CFData::from_bytes(bgra);
    let provider = CGDataProvider::with_cf_data(Some(&data))
        .with_context(|| format!("failed to write {}", path.display()))?;
    let space = CGColorSpace::new_device_rgb().context("device RGB is unavailable")?;
    let info = CGBitmapInfo(
        CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0,
    );
    // SAFETY: `provider` retains `data` for the image. `decode` is null, so every channel is used.
    let image = unsafe {
        CGImage::new(
            width,
            height,
            8,
            32,
            row,
            Some(&space),
            info,
            Some(&provider),
            ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
    .with_context(|| format!("failed to write {}", path.display()))?;
    let url = file_url(path)?;
    let uti = CFString::from_str("public.png");
    // SAFETY: the UTI is `public.png` and the count is one image. Options are null.
    let destination = unsafe { CGImageDestination::with_url(&url, &uti, 1, None) }
        .with_context(|| format!("failed to write {}", path.display()))?;
    // SAFETY: `image` is a PNG-compatible bitmap and the destination is still open.
    unsafe { destination.add_image(&image, None) };
    // SAFETY: this is the single finalize call for `destination`.
    let wrote = unsafe { destination.finalize() };
    if wrote {
        Ok(())
    } else {
        anyhow::bail!("failed to write {}", path.display())
    }
}

fn file_url(path: &Path) -> anyhow::Result<objc2_core_foundation::CFRetained<CFURL>> {
    let text = path
        .to_str()
        .with_context(|| format!("path is not utf-8: {}", path.display()))?;
    let string = CFString::from_str(text);
    CFURL::with_file_system_path(
        None,
        Some(&string),
        CFURLPathStyle::CFURLPOSIXPathStyle,
        false,
    )
    .with_context(|| format!("failed to open {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{load_rgba_png, pixels_close};
    use std::path::Path;

    #[test]
    fn logo_png_is_tightly_packed_rgba() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/logo.png");
        let image = match load_rgba_png(&path) {
            Ok(image) => image,
            Err(error) => panic!("{error:#}"),
        };
        assert_eq!(image.width, 56);
        assert_eq!(image.height, 56);
        assert_eq!(image.pixels.len(), 56 * 56 * 4);
        assert!(image.pixels.iter().any(|byte| *byte != 0));
    }

    #[test]
    fn a_flat_pixel_allows_two_and_rejects_three() {
        let expected = [10, 10, 10, 255];
        let mut within = expected;
        within[0] = 12;
        let mut outside = expected;
        outside[0] = 13;
        assert!(pixels_close(&within, &expected, 1, 1));
        assert!(!pixels_close(&outside, &expected, 1, 1));
    }

    #[test]
    fn an_edge_pixel_allows_six_and_rejects_seven() {
        let expected = [0, 0, 0, 255, 255, 255, 255, 255];
        let mut within = expected;
        within[0] = 6;
        let mut outside = expected;
        outside[0] = 7;
        assert!(pixels_close(&within, &expected, 2, 1));
        assert!(!pixels_close(&outside, &expected, 2, 1));
    }

    #[test]
    #[ignore = "needs UIFW_FRAME from just pixel-test and a local Impeller SDK"]
    fn counter_frame_matches_the_reference() {
        let frame = match std::env::var("UIFW_FRAME") {
            Ok(frame) => frame,
            Err(error) => panic!("UIFW_FRAME: {error}"),
        };
        let actual = match load_rgba_png(Path::new(&frame)) {
            Ok(image) => image,
            Err(error) => panic!("{error:#}"),
        };
        let reference = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pixels/counter.png");
        let expected = match load_rgba_png(&reference) {
            Ok(image) => image,
            Err(error) => panic!("{error:#}"),
        };
        assert_eq!(actual.width, expected.width);
        assert_eq!(actual.height, expected.height);
        assert!(
            pixels_close(
                &actual.pixels,
                &expected.pixels,
                expected.width,
                expected.height
            ),
            "frame differs from {}",
            reference.display()
        );
    }
}
