//! Primary-pointer filter. Extra fingers and historical samples stay out of the queue.

use core::Phase;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Down,
    Move,
    Up,
    Cancel,
    PointerDown,
    PointerUp,
    Other,
}

/// One detent of a mouse wheel, in scene points. The axis value is a detent
/// count, not pixels, so density is not applied.
pub const SCROLL_DETENT_POINTS: f32 = 48.0;

/// Positive `v` scrolls downward. The pair is scene points with Y downward.
/// `None` when both axes are zero or either value is not finite.
pub fn wheel_scene_delta(h_detents: f32, v_detents: f32) -> Option<(f32, f32)> {
    let dx = h_detents * SCROLL_DETENT_POINTS;
    let dy = v_detents * SCROLL_DETENT_POINTS;
    if !dx.is_finite() || !dy.is_finite() || (dx == 0.0 && dy == 0.0) {
        return None;
    }
    Some((dx, dy))
}

/// Relative asset path from an intent extra. Empty segments, `.`, `..`, and
/// backslashes are rejected. An absolute path is rejected.
pub fn script_asset_path(extra: &str) -> Option<String> {
    if extra.is_empty() || extra.starts_with('/') || extra.contains('\\') {
        return None;
    }
    if extra
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return None;
    }
    Some(extra.to_owned())
}

/// `PointerDown` and `PointerUp` are dropped whole, including index 0.
pub fn phase_for(action: Action) -> Option<Phase> {
    match action {
        Action::Down => Some(Phase::Down),
        Action::Move => Some(Phase::Move),
        Action::Up => Some(Phase::Up),
        Action::Cancel => Some(Phase::Cancel),
        Action::PointerDown | Action::PointerUp | Action::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_primary_phases_pass() {
        assert_eq!(phase_for(Action::Down), Some(Phase::Down));
        assert_eq!(phase_for(Action::Move), Some(Phase::Move));
        assert_eq!(phase_for(Action::Up), Some(Phase::Up));
        assert_eq!(phase_for(Action::Cancel), Some(Phase::Cancel));
        assert_eq!(phase_for(Action::PointerDown), None);
        assert_eq!(phase_for(Action::PointerUp), None);
        assert_eq!(phase_for(Action::Other), None);
    }

    #[test]
    fn one_vertical_detent_scrolls_down_48_points() {
        assert_eq!(wheel_scene_delta(0.0, 1.0), Some((0.0, 48.0)));
        assert_eq!(wheel_scene_delta(0.0, 0.0), None);
        assert_eq!(wheel_scene_delta(f32::NAN, 1.0), None);
    }

    #[test]
    fn script_paths_reject_parents_and_empty_segments() {
        assert_eq!(
            script_asset_path("counter.js").as_deref(),
            Some("counter.js")
        );
        assert_eq!(
            script_asset_path("screens/main.js").as_deref(),
            Some("screens/main.js")
        );
        assert_eq!(script_asset_path(""), None);
        assert_eq!(script_asset_path("../counter.js"), None);
        assert_eq!(script_asset_path("a/../b.js"), None);
        assert_eq!(script_asset_path("/counter.js"), None);
        assert_eq!(script_asset_path("a\\b.js"), None);
        assert_eq!(script_asset_path("a//b.js"), None);
    }
}
