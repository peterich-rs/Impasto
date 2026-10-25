//! Window pixels and density become scene points. A missing density is not replaced.

#[cfg_attr(not(test), allow(dead_code))]
const DENSITY_ANY: u32 = 0xfffe;
#[cfg_attr(not(test), allow(dead_code))]
const DENSITY_NONE: u32 = 0xffff;

/// Dots per inch divided by the mdpi baseline.
///
/// `0`, `DENSITY_ANY`, and `DENSITY_NONE` are unread values.
#[cfg_attr(not(test), allow(dead_code))]
pub fn density_scale(dpi: u32) -> Option<f32> {
    if dpi == 0 || dpi == DENSITY_ANY || dpi == DENSITY_NONE {
        return None;
    }
    let scale = dpi as f32 / 160.0;
    (scale.is_finite() && scale > 0.0).then_some(scale)
}

pub fn scene_point(x_px: f32, y_px: f32, density: f32) -> Option<(f32, f32)> {
    if !(density.is_finite() && density > 0.0) {
        return None;
    }
    let x = x_px / density;
    let y = y_px / density;
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

/// Insets in pixels: the content rect is the area inside the window.
#[cfg_attr(not(test), allow(dead_code))]
pub fn content_insets(
    window_width: i32,
    window_height: i32,
    content_left: i32,
    content_top: i32,
    content_right: i32,
    content_bottom: i32,
) -> Option<(i32, i32, i32, i32)> {
    if window_width <= 0
        || window_height <= 0
        || content_left < 0
        || content_top < 0
        || content_right < content_left
        || content_bottom < content_top
        || content_right > window_width
        || content_bottom > window_height
    {
        return None;
    }
    Some((
        content_left,
        content_top,
        window_width - content_right,
        window_height - content_bottom,
    ))
}

pub fn insets_in_points(
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    density: f32,
) -> Option<(f32, f32, f32, f32)> {
    let (left, top) = scene_point(left as f32, top as f32, density)?;
    let (right, bottom) = scene_point(right as f32, bottom as f32, density)?;
    (left >= 0.0 && top >= 0.0 && right >= 0.0 && bottom >= 0.0)
        .then_some((left, top, right, bottom))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_480_is_a_scale_of_three_and_sentinels_fail() {
        assert_eq!(density_scale(480), Some(3.0));
        assert_eq!(density_scale(160), Some(1.0));
        assert_eq!(density_scale(0), None);
        assert_eq!(density_scale(DENSITY_ANY), None);
        assert_eq!(density_scale(DENSITY_NONE), None);
    }

    #[test]
    fn pixels_divide_by_density() {
        assert_eq!(scene_point(1080.0, 2400.0, 3.0), Some((360.0, 800.0)));
        assert_eq!(scene_point(10.0, 10.0, 0.0), None);
    }

    #[test]
    fn status_and_navigation_insets_become_points() {
        let (left, top, right, bottom) =
            content_insets(1080, 2400, 0, 120, 1080, 2250).expect("insets");
        assert_eq!((left, top, right, bottom), (0, 120, 0, 150));
        assert_eq!(
            insets_in_points(left, top, right, bottom, 3.0),
            Some((0.0, 40.0, 0.0, 50.0))
        );
    }
}
