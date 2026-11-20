//! Pointer samples stay in scene points. AppKit already reports those points.

use uicore::{Button, Modifiers, Phase, PointerEvent};

/// One AppKit sample before it is flipped into scene points.
pub struct PointerSample {
    pub phase: Phase,
    pub local_x: f64,
    pub local_y: f64,
    pub view_height: f64,
    pub time_ms: f64,
}

/// `local_x` and `local_y` are inside the content view, with the origin at the
/// bottom left. The scene uses the same point size and puts the origin at the
/// top left. `backingScaleFactor` converts points to device pixels, so it is
/// not applied here.
pub fn scene_point(local_x: f64, local_y: f64, view_height: f64) -> (f32, f32) {
    (scene_f32(local_x), scene_f32(view_height - local_y))
}

/// AppKit reports points and scales as `f64`. The scene stores the same quantity as `f32`.
pub(crate) fn scene_f32(value: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "AppKit points and scales are f64; the scene stores them as f32"
    )]
    {
        value as f32
    }
}

pub fn pointer_event(
    sample: PointerSample,
    button: Button,
    modifiers: Modifiers,
    dx: f32,
    dy: f32,
) -> PointerEvent {
    let (x, y) = scene_point(sample.local_x, sample.local_y, sample.view_height);
    PointerEvent {
        pointer: uicore::PointerId(0),
        kind: uicore::PointerKind::Mouse,
        phase: sample.phase,
        x,
        y,
        time_ms: sample.time_ms,
        button,
        buttons: uicore::Buttons::from_button(button),
        modifiers,
        dx,
        dy,
        zoom: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::{PointerSample, pointer_event, scene_point};
    use uicore::{Button, Modifiers, Phase};

    #[test]
    fn top_left_of_the_view_is_the_scene_origin() {
        assert_eq!(scene_point(24.0, 180.0, 180.0), (24.0, 0.0));
    }

    #[test]
    fn bottom_left_of_the_view_is_the_bottom_of_the_scene() {
        assert_eq!(scene_point(0.0, 0.0, 180.0), (0.0, 180.0));
    }

    #[test]
    fn a_point_below_the_top_keeps_the_scene_distance() {
        // 72 points down from the top of a 180-point view is local y 108.
        let event = pointer_event(
            PointerSample {
                phase: Phase::Down,
                local_x: 24.0,
                local_y: 108.0,
                view_height: 180.0,
                time_ms: 12.0,
            },
            Button::Primary,
            Modifiers::empty(),
            0.0,
            0.0,
        );
        assert_eq!(event.time_ms, 12.0);
        assert_eq!(event.phase, Phase::Down);
        assert_eq!((event.x, event.y), (24.0, 72.0));
    }
}
