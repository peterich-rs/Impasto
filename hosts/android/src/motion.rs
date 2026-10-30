//! Pointer samples. Each finger keeps its pointer id. Historical samples stay out.

use uicore::Phase;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
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

/// Android `AXIS_VSCROLL` is positive when the wheel moves content up.
/// The returned pair uses scene points with Y downward.
pub fn android_wheel(h_detents: f32, v_detents: f32) -> Option<(f32, f32)> {
    wheel_scene_delta(h_detents, -v_detents)
}

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

/// `PointerDown` is another finger going down. `PointerUp` is that finger lifting.
/// `Move` and `Cancel` apply to every pointer in the sample.
#[cfg_attr(not(test), allow(dead_code))]
pub fn phase_for(action: Action) -> Option<Phase> {
    match action {
        Action::Down | Action::PointerDown => Some(Phase::Down),
        Action::Move => Some(Phase::Move),
        Action::Up | Action::PointerUp => Some(Phase::Up),
        Action::Cancel => Some(Phase::Cancel),
        Action::Other => None,
    }
}

/// Indexes that carry `phase_for` for this action. Move and cancel report every pointer.
#[cfg_attr(not(test), allow(dead_code))]
pub fn pointer_indexes(action: Action, count: usize, action_index: usize) -> Vec<usize> {
    match action {
        Action::Down | Action::Up if count > 0 => vec![0],
        Action::Down | Action::Up => Vec::new(),
        Action::PointerDown | Action::PointerUp => {
            if action_index < count {
                vec![action_index]
            } else {
                Vec::new()
            }
        }
        Action::Move | Action::Cancel => (0..count).collect(),
        Action::Other => Vec::new(),
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
        assert_eq!(phase_for(Action::PointerDown), Some(Phase::Down));
        assert_eq!(phase_for(Action::PointerUp), Some(Phase::Up));
        assert_eq!(pointer_indexes(Action::Move, 2, 0), vec![0, 1]);
        assert_eq!(pointer_indexes(Action::PointerDown, 2, 1), vec![1]);
        assert_eq!(phase_for(Action::Other), None);
    }

    #[test]
    fn one_vertical_detent_scrolls_down_48_points() {
        assert_eq!(wheel_scene_delta(0.0, 1.0), Some((0.0, 48.0)));
        assert_eq!(wheel_scene_delta(0.0, 0.0), None);
        assert_eq!(wheel_scene_delta(f32::NAN, 1.0), None);
        assert_eq!(android_wheel(0.0, 1.0), Some((0.0, -48.0)));
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
