use super::controller::ScrollController;
use super::diff::{ShownRow, diff_rows};
use super::prefix::PrefixHeights;
use super::window::{
    FLING_VIEW_FACTOR, REFRESH_THRESHOLD, drag_origin, estimate_height, near_end, pull_delta,
    stabilize_window, sticky_translate, thumb_length, thumb_offset, window_range,
    window_range_for_offset,
};
use super::{DriverSet, ListItem, ListNodes, ListOp, PullMode, ScrollConfig};
use crate::{MonoShaper, ScrollAxis, Tree};

fn close(left: f32, right: f32) -> bool {
    (left - right).abs() < 1e-4
}

#[test]
fn fenwick_matches_the_naive_prefix() {
    let mut prefix = PrefixHeights::default();
    prefix.rebuild(&[10.0, 30.0]);
    assert_eq!(prefix.find_index(0.0), 0);
    assert_eq!(prefix.find_index(10.0), 1);
    assert_eq!(prefix.find_index(100.0), 2);
    assert!(close(prefix.item_start(0), 0.0));
    assert!(close(prefix.item_start(1), 10.0));
    assert!(close(prefix.total(), 40.0));

    let mut heights = Vec::new();
    let mut naive = 0.0;
    let mut naive_prefix = vec![0.0];
    for index in 0..1000 {
        let height = if index % 2 == 0 { 12.0 } else { 28.0 };
        heights.push(height);
        naive += height;
        naive_prefix.push(naive);
    }
    let mut alternating = PrefixHeights::default();
    alternating.rebuild(&heights);
    let mut index = 0;
    while index <= 1000 {
        assert!(close(alternating.item_start(index), naive_prefix[index]));
        index += 17;
    }
    assert_eq!(alternating.find_index(naive_prefix[400]), 400);
    let reversed: Vec<f32> = heights.iter().rev().copied().collect();
    alternating.rebuild(&reversed);
    assert!(close(alternating.height_at(0), heights[999]));
    assert!(close(alternating.item_start(1), heights[999]));
    alternating.add(0, 5.0);
    assert!(close(alternating.height_at(0), heights[999] + 5.0));
}

#[test]
fn window_thumb_and_pull_match_the_old_module() {
    let mut prefix = PrefixHeights::default();
    prefix.rebuild(&[10.0, 30.0]);
    let view = window_range(&prefix, 0.0, 10.0, 4);
    assert_eq!(view.start, 0);
    assert!(view.end <= 2);
    let held = window_range_for_offset(&prefix, 0.0, 10.0, 0, None);
    let plain = window_range(&prefix, 0.0, 10.0, 0);
    assert_eq!(held.start, plain.start);
    assert_eq!(held.end, plain.end);
    let flung = window_range_for_offset(&prefix, 0.0, 10.0, 0, Some(40.0));
    assert!(flung.end > held.end);

    let mut rows = PrefixHeights::default();
    rows.rebuild(&vec![10.0; 80]);
    let plain = window_range_for_offset(&rows, 100.0, 200.0, 4, None);
    let led = window_range_for_offset(&rows, 100.0, 200.0, 4, Some(450.0));
    assert!(led.end > plain.end);
    let grown = stabilize_window(
        &rows,
        plain,
        super::window::WindowRange {
            start: plain.start + 1,
            ..plain
        },
        4,
    );
    assert_eq!(grown.start, plain.start);
    let shifted = stabilize_window(
        &rows,
        plain,
        super::window::WindowRange {
            start: plain.start + 5,
            ..plain
        },
        4,
    );
    assert_eq!(shifted.start, plain.start + 5);

    let past = window_range(&prefix, 100.0, 10.0, 4);
    assert_eq!(past.start, 2);
    assert_eq!(past.end, 2);
    assert!(close(past.top, 40.0));
    assert!(close(past.bottom, 0.0));

    assert!(close(estimate_height(28.0), 28.0));
    assert!(close(estimate_height(0.0), 1.0));
    assert!(close(estimate_height(f32::NAN), 1.0));
    assert!(close(estimate_height(-4.0), 1.0));
    assert!(close(sticky_translate(None, 40.0, 20.0), 0.0));
    assert!(close(sticky_translate(Some(100.0), 50.0, 30.0), 0.0));
    assert!(close(sticky_translate(Some(80.0), 50.0, 40.0), -10.0));
    assert!(close(pull_delta(0.0, -12.0, 0.0), 12.0));
    assert!(close(pull_delta(8.0, -4.0, 0.0), 0.0));
    assert!(close(pull_delta(0.0, 4.0, 4.0), 0.0));
    let pulled = drag_origin(0.0, 0.0, 0.0, -22.0, 0.55);
    assert!(close(pulled.1, -40.0));
    assert_eq!(drag_origin(3.0, 4.0, 9.0, 9.0, 0.0), (3.0, 4.0));
    assert!(close(REFRESH_THRESHOLD, 64.0));
    assert!(close(thumb_length(100.0, 400.0), 25.0));
    assert!(close(thumb_offset(100.0, 400.0, 300.0), 75.0));
    assert!(close(thumb_length(40.0, 1000.0), 24.0));
    assert!(!near_end(96, 100, 4));
    assert!(near_end(97, 100, 4));

    let mut long_rows = PrefixHeights::default();
    long_rows.rebuild(&vec![10.0; 200]);
    let capped = window_range_for_offset(&long_rows, 0.0, 100.0, 2, Some(5000.0));
    assert!(close(FLING_VIEW_FACTOR, 3.0));
    #[expect(
        clippy::cast_possible_truncation,
        reason = "10 * FLING_VIEW_FACTOR is 30 rows"
    )]
    let window = (10.0 * FLING_VIEW_FACTOR) as usize;
    assert!(capped.end - capped.start <= window + 4);
    assert_eq!(capped.start, 0);
    let upward = window_range_for_offset(&long_rows, 1000.0, 100.0, 2, Some(0.0));
    assert!(upward.end - upward.start <= window + 4);
    assert!(upward.start <= long_rows.find_index(1000.0));
}

#[test]
fn diff_detaches_adopts_and_places_from_the_end() {
    let first = rows(&[(0, "even"), (1, "odd"), (2, "even")]);
    let ops = diff_rows(&[], &first, false);
    assert!(
        ops.iter()
            .any(|op| matches!(op, ListOp::Adopt { key, .. } if key == "0"))
    );
    let place = ops.iter().find(|op| matches!(op, ListOp::Place { .. }));
    assert!(matches!(place, Some(ListOp::Place { before, .. }) if before.is_empty()));

    let slide = rows(&[(1, "odd"), (2, "even"), (3, "odd")]);
    let ops = diff_rows(&first, &slide, false);
    assert!(
        ops.iter()
            .any(|op| matches!(op, ListOp::Detach { key } if key == "0"))
    );
    assert!(
        ops.iter()
            .any(|op| matches!(op, ListOp::Adopt { key, .. } if key == "3"))
    );
    assert!(
        !ops.iter()
            .any(|op| matches!(op, ListOp::Detach { key } if key == "1"))
    );

    let retype = rows(&[(1, "even")]);
    let stayed = rows(&[(1, "odd")]);
    let ops = diff_rows(&stayed, &retype, false);
    assert!(
        ops.iter()
            .any(|op| matches!(op, ListOp::Detach { key } if key == "1"))
    );
    assert!(
        ops.iter()
            .any(|op| matches!(op, ListOp::Adopt { ty, .. } if ty == "even"))
    );

    let again = diff_rows(&first, &first, false);
    assert!(again.is_empty());
    let forced = diff_rows(&first, &first, true);
    assert_eq!(
        forced
            .iter()
            .filter(|op| matches!(op, ListOp::Set { .. }))
            .count(),
        3
    );
    assert!(!forced.iter().any(|op| matches!(op, ListOp::Place { .. })));
}

fn rows(items: &[(u32, &str)]) -> Vec<ShownRow> {
    items
        .iter()
        .map(|(index, ty)| ShownRow {
            key: index.to_string(),
            ty: (*ty).to_owned(),
            index: *index,
        })
        .collect()
}

#[test]
fn driver_tick_mounts_a_window_and_measures_rows() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let scroll = tree.scroll()?;
    tree.set_max_size(scroll, 120.0, 80.0)?;
    let content = tree.column(0.0)?;
    let top = tree.sized(1.0, 1.0)?;
    let rows = tree.column(0.0)?;
    let bottom = tree.sized(1.0, 1.0)?;
    tree.add_child(scroll, content)?;
    tree.add_child(content, top)?;
    tree.add_child(content, rows)?;
    tree.add_child(content, bottom)?;
    tree.layout()?;

    let mut drivers = DriverSet::new();
    let id = drivers.list_create(2, 20.0);
    assert!(drivers.list_bind(
        id,
        ListNodes {
            viewport: Some(scroll),
            content: Some(content),
            top: Some(top),
            bottom: Some(bottom),
            rows: Some(rows),
            ..ListNodes::default()
        }
    ));
    assert!(drivers.list_begin(id));
    for index in 0..12 {
        assert!(drivers.list_item(
            id,
            ListItem {
                key: index.to_string(),
                ty: "row".to_owned(),
                sticky: index == 0,
            }
        ));
    }
    assert!(drivers.list_end(id));
    drivers.tick(&mut tree);
    let events = drivers.take_events();
    let adopts = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                super::DriverEvent::ListOp {
                    op: ListOp::Adopt { .. },
                    ..
                }
            )
        })
        .count();
    assert!(adopts >= 2, "window should adopt rows, events {events:?}");
    assert!(drivers.wants_frame());

    let mut nodes = Vec::new();
    for index in 0..adopts.min(6) {
        let row = tree.sized(40.0, 20.0)?;
        tree.add_child(rows, row)?;
        assert!(drivers.list_note_row(id, &index.to_string(), Some(row)));
        nodes.push(row);
    }
    tree.layout()?;
    drivers.tick(&mut tree);
    let _ = drivers.take_events();
    let top_box = tree.rect(top)?;
    assert!(top_box.h >= 0.0);
    assert!(tree.rect(content)?.h >= 20.0);
    let _ = nodes;
    Ok(())
}

#[test]
fn edge_pull_commits_on_drag_end_and_clamp_keeps_the_offset() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let scroll = tree.scroll()?;
    tree.set_max_size(scroll, 40.0, 40.0)?;
    let body = tree.sized(20.0, 200.0)?;
    tree.add_child(scroll, body)?;
    tree.layout()?;
    let mut drivers = DriverSet::new();
    let id = drivers.scroll_create();
    assert!(drivers.scroll_bind(id, Some(scroll), Some(body), &tree));
    assert!(drivers.scroll_configure(
        id,
        ScrollConfig {
            content_min: 0.0,
            pull_mode: PullMode::Edge,
            threshold: 64.0,
            rubber: false,
            factor: 0.0,
        },
        None,
    ));
    let Some(step) = drivers.scroll_drag_by(id, &mut tree, 0.0, 80.0) else {
        return Err(crate::Error::UnknownNode);
    };
    if step.pending {
        let _ = drivers.scroll_finish(id, &mut tree, false);
    }
    let offset = tree.scroll_offset(scroll)?;
    assert!(offset.1 >= 0.0);
    assert!(offset.1 <= 160.0 + 1.0);
    let Some(fired) = drivers.scroll_drag_end(id) else {
        return Err(crate::Error::UnknownNode);
    };
    assert!(fired, "top-edge pull of 80 passes the threshold");
    let events = drivers.take_events();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, super::DriverEvent::Refresh { .. }))
    );
    Ok(())
}

#[test]
fn a_vertical_controller_discards_horizontal_drag() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let scroll = tree.scroll_with(ScrollAxis::Vertical)?;
    tree.set_max_size(scroll, 80.0, 40.0)?;
    let body = tree.sized(400.0, 400.0)?;
    tree.add_child(scroll, body)?;
    tree.layout()?;
    tree.set_scroll_offset(scroll, 0.0, 40.0)?;
    let mut drivers = DriverSet::new();
    let id = drivers.scroll_create();
    assert!(drivers.scroll_bind(id, Some(scroll), Some(body), &tree));
    assert!(drivers.scroll_configure(
        id,
        ScrollConfig {
            content_min: 0.0,
            pull_mode: PullMode::Off,
            threshold: 64.0,
            rubber: false,
            factor: 0.0,
        },
        None,
    ));
    let Some(step) = drivers.scroll_drag_by(id, &mut tree, 30.0, 10.0) else {
        return Err(crate::Error::UnknownNode);
    };
    assert_eq!(step.overflow_x, 0.0);
    assert!(
        !step.pending,
        "cross-axis drag must not look like overflow: {step:?}"
    );
    let (x, y) = tree.scroll_offset(scroll)?;
    assert_eq!(x, 0.0);
    assert!(y < 39.0 && y > 1.0, "y moved, x did not: {x} {y}");

    let mut tree = Tree::new(MonoShaper::default());
    let both = tree.scroll()?;
    tree.set_max_size(both, 80.0, 40.0)?;
    let wide = tree.sized(400.0, 200.0)?;
    tree.add_child(both, wide)?;
    tree.layout()?;
    tree.set_scroll_offset(both, 40.0, 40.0)?;
    let mut drivers = DriverSet::new();
    let both_id = drivers.scroll_create();
    assert!(drivers.scroll_bind(both_id, Some(both), Some(wide), &tree));
    assert!(drivers.scroll_configure(
        both_id,
        ScrollConfig {
            content_min: 0.0,
            pull_mode: PullMode::Off,
            threshold: 64.0,
            rubber: false,
            factor: 0.0,
        },
        None,
    ));
    let Some(sideways) = drivers.scroll_drag_by(both_id, &mut tree, 10.0, 0.0) else {
        return Err(crate::Error::UnknownNode);
    };
    assert_eq!(sideways.overflow_x, 0.0);
    let (x, y) = tree.scroll_offset(both)?;
    assert!(x < 39.0 && x > 1.0, "both still scrolls x: {x}");
    assert!((y - 40.0).abs() < 0.5, "y stayed {y}");
    Ok(())
}

#[test]
fn consume_applies_overflow_when_the_axis_has_room() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let scroll = tree.scroll()?;
    tree.set_max_size(scroll, 80.0, 40.0)?;
    let body = tree.sized(80.0, 400.0)?;
    tree.add_child(scroll, body)?;
    tree.layout()?;
    let mut controller = ScrollController::default();
    controller.bind(Some(scroll), Some(body), &tree);
    controller.configure(0.0, PullMode::Off, 64.0, false, 0.0, None);
    assert!(!controller.consume(&mut tree, 0.0, 0.0));
    assert!(controller.consume(&mut tree, 0.0, 12.0));
    assert!(tree.scroll_offset(scroll)?.1 > 11.0);
    Ok(())
}

#[test]
fn overscroll_pull_commits_when_the_scroll_returns_to_idle() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let scroll = tree.scroll()?;
    tree.set_max_size(scroll, 80.0, 40.0)?;
    let body = tree.sized(80.0, 400.0)?;
    tree.add_child(scroll, body)?;
    tree.layout()?;
    tree.set_scroll_offset(scroll, 0.0, -80.0)?;
    let mut controller = ScrollController::default();
    controller.bind(Some(scroll), Some(body), &tree);
    controller.configure(0.0, PullMode::Overscroll, 1.0, true, 0.55, None);
    let step = controller.drag_by(&mut tree, 0.0, 1.0);
    if step.pending {
        let _ = controller.finish_drag(&mut tree, false);
    }
    let _ = controller.drag_end();
    assert!(
        controller.tick(&mut tree),
        "armed overscroll should commit on idle tick, overscroll {:?}",
        tree.overscroll(scroll)
    );
    Ok(())
}
