//! Pointer samples stay in scene points. AppKit already reports those points.

use core::{Button, Modifiers, Phase, PointerEvent};

/// `local_x` and `local_y` are inside the content view, with the origin at the
/// bottom left. The scene uses the same point size and puts the origin at the
/// top left. `backingScaleFactor` converts points to device pixels, so it is
/// not applied here.
pub fn scene_point(local_x: f64, local_y: f64, view_height: f64) -> (f32, f32) {
    (local_x as f32, (view_height - local_y) as f32)
}

pub fn pointer_event(
    phase: Phase,
    local_x: f64,
    local_y: f64,
    view_height: f64,
    time_ms: f64,
    button: Button,
    modifiers: Modifiers,
    dx: f32,
    dy: f32,
) -> PointerEvent {
    let (x, y) = scene_point(local_x, local_y, view_height);
    PointerEvent {
        phase,
        x,
        y,
        time_ms,
        button,
        modifiers,
        dx,
        dy,
    }
}

#[cfg(test)]
mod tests {
    use super::{pointer_event, scene_point};
    use core::{Button, Modifiers, Phase};

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
            Phase::Down,
            24.0,
            108.0,
            180.0,
            12.0,
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
