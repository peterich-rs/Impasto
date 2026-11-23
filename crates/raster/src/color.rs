//! Color conversion. Scene colors are sRGB bytes; the pipeline works in
//! linear, premultiplied floats and writes to sRGB targets.

/// Linear premultiplied `[r, g, b, a]` from an sRGB `[u8; 4]`, with an extra
/// coverage multiplier on alpha. `None` when a channel is out of range.
pub fn linear_premultiplied(color: [u8; 4], coverage: u8) -> Option<[f32; 4]> {
    let alpha = scale_u8(color[3], coverage);
    let a = alpha as f32 / 255.0;
    Some([
        srgb_to_linear(color[0]) * a,
        srgb_to_linear(color[1]) * a,
        srgb_to_linear(color[2]) * a,
        a,
    ])
}

/// Encodes linear premultiplied pixels for an sRGB texture. Filtering then
/// interpolates premultiplied texels, including at transparent image edges.
pub fn premultiply_rgba(rgba: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if rgba.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 255) {
        return std::borrow::Cow::Borrowed(rgba);
    }
    let mut pixels = rgba.to_vec();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        if pixel[3] == 0 {
            *pixel = [0; 4];
        } else if pixel[3] != 255 {
            let alpha = f32::from(pixel[3]) / 255.0;
            for channel in &mut pixel[..3] {
                *channel = srgb_byte(srgb_to_linear(*channel) * alpha);
            }
        }
    }
    std::borrow::Cow::Owned(pixels)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the linear channel is clamped and rounded to a byte"
)]
fn srgb_byte(linear: f32) -> u8 {
    (linear_to_srgb(linear.clamp(0.0, 1.0)) * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

/// A straight-alpha linear color, for shaders that multiply later.
#[cfg(test)]
fn linear_straight(color: [u8; 4]) -> [f32; 4] {
    [
        srgb_to_linear(color[0]),
        srgb_to_linear(color[1]),
        srgb_to_linear(color[2]),
        color[3] as f32 / 255.0,
    ]
}

/// Linear premultiplied back to straight sRGB bytes, for readback.
#[expect(
    clippy::cast_possible_truncation,
    reason = "normalized channels are clamped and rounded to RGBA8"
)]
pub fn straight_srgb(premultiplied: [f32; 4]) -> [u8; 4] {
    let a = premultiplied[3].clamp(0.0, 1.0);
    let unpremultiply = |channel: f32| -> u8 {
        if a <= f32::EPSILON {
            return 0;
        }
        srgb_byte(channel / a)
    };
    [
        unpremultiply(premultiplied[0]),
        unpremultiply(premultiplied[1]),
        unpremultiply(premultiplied[2]),
        (a * 255.0 + 0.5).clamp(0.0, 255.0) as u8,
    ]
}

pub(crate) fn srgb_to_linear(channel: u8) -> f32 {
    let value = channel as f32 / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

/// Alpha of a canvas draw: its coverage times the group opacity, in 0..1.
pub fn coverage_alpha(alpha: u8, opacity: f32) -> f32 {
    alpha as f32 / 255.0 * opacity
}

fn scale_u8(channel: u8, coverage: u8) -> u8 {
    let product = u16::from(channel) * u16::from(coverage) + 127;
    u8::try_from(product / 255).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use super::{linear_premultiplied, linear_straight, straight_srgb};

    #[test]
    fn opaque_white_is_identity_in_both_directions() {
        let linear = linear_premultiplied([255, 255, 255, 255], 255).expect("white");
        assert_eq!(linear, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(straight_srgb(linear), [255, 255, 255, 255]);
        let half = linear_premultiplied([255, 0, 0, 128], 255).expect("half red");
        assert!((half[3] - 128.0 / 255.0).abs() < 1e-6);
        assert!((half[0] - half[3]).abs() < 1e-6, "premultiplied");
        let back = straight_srgb(half);
        assert_eq!(back, [255, 0, 0, 128]);
    }

    #[test]
    fn coverage_multiplies_alpha_like_the_canvas_did() {
        assert_eq!(
            linear_premultiplied([10, 20, 30, 255], 255).map(|c| c[3]),
            Some(1.0)
        );
        let scaled = linear_premultiplied([10, 20, 30, 255], 128).expect("scaled");
        assert!((scaled[3] - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(linear_straight([0, 0, 0, 0])[3], 0.0);
    }
}
