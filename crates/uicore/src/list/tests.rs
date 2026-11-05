use super::diff::{ShownRow, diff_rows};
use super::prefix::PrefixHeights;
use super::window::{
    FLING_VIEW_FACTOR, REFRESH_THRESHOLD, drag_origin, estimate_height, near_end, pull_delta,
    stabilize_window, sticky_translate, thumb_length, thumb_offset, window_range,
    window_range_for_offset,
};
use super::{DriverSet, ListItem, ListNodes, ListOp};
use crate::{MonoShaper, Tree};

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
fn repeating_the_same_items_does_not_force_another_window() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let scroll = tree.scroll()?;
    tree.set_max_size(scroll, 120.0, 80.0)?;
    let content = tree.column(0.0)?;
    tree.add_child(scroll, content)?;
    tree.layout()?;
    let mut drivers = DriverSet::new();
    let id = drivers.list_create(1, 20.0);
    assert!(drivers.list_bind(
        id,
        ListNodes {
            viewport: Some(scroll),
            content: Some(content),
            ..ListNodes::default()
        }
    ));
    let submit = |drivers: &mut DriverSet| {
        assert!(drivers.list_begin(id));
        assert!(drivers.list_item(
            id,
            ListItem {
                key: "a".to_owned(),
                ty: "row".to_owned(),
                sticky: false,
            }
        ));
        assert!(drivers.list_end(id));
    };
    submit(&mut drivers);
    drivers.tick(&mut tree);
    let _ = drivers.take_events();
    submit(&mut drivers);
    drivers.tick(&mut tree);
    let again = drivers.take_events();
    assert!(
        again.iter().all(|event| !matches!(
            event,
            super::DriverEvent::ListOp {
                op: ListOp::Adopt { .. } | ListOp::Set { .. } | ListOp::Detach { .. },
                ..
            }
        )),
        "same keys replayed the window: {again:?}"
    );
    Ok(())
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
fn a_pinned_header_is_not_painted_and_keeps_its_height() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let scroll = tree.scroll()?;
    tree.set_max_size(scroll, 120.0, 40.0)?;
    let content = tree.column(0.0)?;
    let top = tree.sized(1.0, 0.0)?;
    let rows = tree.column(0.0)?;
    let bottom = tree.sized(1.0, 0.0)?;
    tree.add_child(scroll, content)?;
    tree.add_child(content, top)?;
    tree.add_child(content, rows)?;
    tree.add_child(content, bottom)?;

    let mut drivers = DriverSet::new();
    let id = drivers.list_create(1, 20.0);
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
    assert!(drivers.list_item(
        id,
        ListItem {
            key: "h".to_owned(),
            ty: "header".to_owned(),
            sticky: true,
        }
    ));
    for index in 0..6 {
        assert!(drivers.list_item(
            id,
            ListItem {
                key: index.to_string(),
                ty: "row".to_owned(),
                sticky: false,
            }
        ));
    }
    assert!(drivers.list_end(id));

    let header = tree.column(0.0)?;
    let label = tree.text("Group", 16.0, [0, 0, 0, 255])?;
    tree.add_child(header, label)?;
    tree.add_child(rows, header)?;
    assert!(drivers.list_note_row(id, "h", Some(header)));
    tree.layout()?;
    let height = tree.rect(header)?.h;
    assert!(
        paints_group(&mut tree),
        "the header paints before it sticks"
    );
    drivers.tick(&mut tree);
    assert!(
        (tree.rect(header)?.h - height).abs() < 0.5,
        "cull changed the height from {height}"
    );
    assert!(
        !paints_group(&mut tree),
        "the in-window copy is hidden while the overlay shows it"
    );
    Ok(())
}

fn paints_group(tree: &mut Tree) -> bool {
    let Ok(_) = tree.record() else {
        return false;
    };
    tree.layer_tree().pictures.iter().any(|picture| {
        picture.commands.iter().any(|command| {
            matches!(command, protocol::Command::Text { utf8, .. } if utf8.as_ref() == "Group")
        })
    })
}
