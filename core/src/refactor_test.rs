use crate::{
    AxisSize, DoubleTapConfig, Edit, FocusDirection, Gesture, MonoShaper, Phase, PointerEvent,
    RecognizerSet, RowCrossAlign, ScrollEdge, SemanticsRole, TextEditor, Tree,
};

fn color() -> [u8; 4] {
    [0, 0, 0, 255]
}

#[test]
fn baseline_row_shifts_16_28_40_and_a_column_is_unchanged() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let row = tree.row(0.0)?;
    tree.set_row_cross(row, RowCrossAlign::Baseline)?;
    let small = tree.text("A", 16.0, color())?;
    let mid = tree.text("A", 28.0, color())?;
    let large = tree.text("A", 40.0, color())?;
    tree.add_child(row, small)?;
    tree.add_child(row, mid)?;
    tree.add_child(row, large)?;
    tree.layout()?;
    let small = tree.rect(small)?;
    let mid = tree.rect(mid)?;
    let large = tree.rect(large)?;
    assert!((small.y - 19.2).abs() < 0.05, "y={}", small.y);
    assert!((mid.y - 9.6).abs() < 0.05, "y={}", mid.y);
    assert!(large.y.abs() < 0.05, "y={}", large.y);

    let mut plain = Tree::new(MonoShaper::default());
    let column = plain.column(0.0)?;
    let child = plain.text("A", 16.0, color())?;
    plain.add_child(column, child)?;
    plain.layout()?;
    let before = plain.rect(child)?;
    plain.set_row_cross(column, RowCrossAlign::Baseline)?;
    plain.layout()?;
    assert_eq!(plain.rect(child)?, before);
    plain.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    plain.set_fill(column)?;
    plain.set_row_cross(column, RowCrossAlign::Center)?;
    plain.layout()?;
    let centered = plain.rect(child)?;
    assert!(
        (centered.x - (100.0 - centered.w) / 2.0).abs() < 0.5,
        "x={} w={}",
        centered.x,
        centered.w
    );
    plain.set_row_cross(column, RowCrossAlign::Stretch)?;
    plain.layout()?;
    let stretched = plain.rect(child)?;
    assert!(
        (stretched.w - 100.0).abs() < 0.5,
        "stretched w={}",
        stretched.w
    );
    Ok(())
}

#[test]
fn commit_replaces_composing_and_insert_keeps_it() {
    let mut editor = TextEditor::new("");
    editor.apply(Edit::Composing(Some("ni".to_owned())));
    let mut inserted = editor.clone();
    inserted.apply(Edit::Insert("!".to_owned()));
    assert_eq!(inserted.value().text, "ni!");
    assert!(inserted.value().composing.is_none());
    editor.apply(Edit::Commit("你".to_owned()));
    assert_eq!(editor.value().text, "你");
    assert!(editor.value().composing.is_none());
}

#[test]
fn set_text_clamps_composing_and_undo_still_restores() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let text = tree.text("hello", 16.0, color())?;
    tree.text_edit(text, Edit::Insert("hello".to_owned()))?;
    tree.text_edit(text, Edit::Composing(Some("xx".to_owned())))?;
    tree.set_text(text, "ab")?;
    let (_, value) = tree.editor_snapshot(text).unwrap();
    assert_eq!(value.text, "ab");
    assert!(value.composing.is_some());
    assert!(value.composing.unwrap().end <= 2);
    tree.text_edit(text, Edit::Undo)?;
    assert_eq!(tree.editor_text(text).as_deref(), Some("hello"));
    Ok(())
}

#[test]
fn caret_is_reported_only_for_the_focused_editor() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let column = tree.column(0.0)?;
    let text = tree.text("hi", 16.0, color())?;
    let other = tree.text("no", 16.0, color())?;
    tree.add_child(column, text)?;
    tree.add_child(column, other)?;
    tree.set_focusable(text, true)?;
    tree.focus(text)?;
    tree.text_edit(text, Edit::Load("hi".to_owned()))?;
    tree.layout()?;
    let update = tree.semantics().expect("semantics");
    let focused = update
        .nodes
        .iter()
        .find(|node| node.id == crate::SemanticsId::from_node(text))
        .expect("focused");
    let extent = tree.editor_snapshot(text).unwrap().1.selection.extent;
    assert_eq!(focused.caret, Some(extent));
    let idle = update
        .nodes
        .iter()
        .find(|node| node.id == crate::SemanticsId::from_node(other))
        .expect("other");
    assert_eq!(idle.caret, None);
    let _ = SemanticsRole::Text;
    Ok(())
}

#[test]
fn overscroll_of_50_is_27_5_and_does_not_rewrite_pictures() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    let scroll = tree.scroll()?;
    let child = tree.sized(20.0, 20.0)?;
    tree.add_child(scroll, child)?;
    tree.layout()?;
    tree.record()?;
    tree.set_scroll_offset(scroll, 0.0, 50.0)?;
    let (_, overscroll) = tree.overscroll(scroll)?;
    assert!((overscroll - 27.5).abs() < 0.01, "overscroll={overscroll}");
    tree.record()?;
    assert_eq!(tree.picture_writes(), 0);
    Ok(())
}

#[test]
fn double_tap_waits_then_clicks_or_fires_on_the_second_down() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let region = tree.pointer_region()?;
    let child = tree.sized(40.0, 40.0)?;
    tree.add_child(region, child)?;
    tree.set_recognizers(
        region,
        RecognizerSet {
            tap: true,
            double_tap: Some(DoubleTapConfig::default()),
            ..RecognizerSet::default()
        },
    )?;
    tree.layout()?;
    let down = PointerEvent::new(Phase::Down, 10.0, 10.0, 0.0);
    tree.pointer(down);
    let up = tree.pointer(PointerEvent::new(Phase::Up, 10.0, 10.0, 20.0));
    assert!(
        !up.iter()
            .any(|gesture| matches!(gesture, Gesture::Click(_))),
        "first up waits: {up:?}"
    );
    let late = tree.tick(400.0);
    assert!(
        late.iter()
            .any(|gesture| matches!(gesture, Gesture::Click(id) if *id == region)),
        "window emits click: {late:?}"
    );

    tree.pointer(PointerEvent::new(Phase::Down, 10.0, 10.0, 1000.0));
    tree.pointer(PointerEvent::new(Phase::Up, 10.0, 10.0, 1020.0));
    let second = tree.pointer(PointerEvent::new(Phase::Down, 12.0, 12.0, 1100.0));
    assert!(
        second
            .iter()
            .any(|gesture| matches!(gesture, Gesture::DoubleTap(id) if *id == region)),
        "second down: {second:?}"
    );
    Ok(())
}

#[test]
fn focus_right_stays_in_the_band_above_a_nested_row() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let column = tree.column(10.0)?;
    let row = tree.row(8.0)?;
    let nested = tree.row(0.0)?;
    let left = tree.sized(40.0, 40.0)?;
    let right = tree.sized(40.0, 40.0)?;
    let below = tree.sized(40.0, 40.0)?;
    tree.set_focusable(left, true)?;
    tree.set_focusable(right, true)?;
    tree.set_focusable(below, true)?;
    tree.add_child(row, left)?;
    tree.add_child(row, right)?;
    tree.add_child(nested, below)?;
    tree.add_child(column, row)?;
    tree.add_child(column, nested)?;
    tree.layout()?;
    tree.focus(left)?;
    let moved = tree.focus_step(FocusDirection::Right);
    assert!(
        moved
            .iter()
            .any(|gesture| matches!(gesture, Gesture::Focus(id) if *id == right)),
        "stays in the band: {moved:?}"
    );
    Ok(())
}

#[test]
fn scroll_into_view_arms_a_spring_toward_the_child() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    let scroll = tree.scroll()?;
    tree.set_axis_size(scroll, AxisSize::Points(100.0), AxisSize::Points(40.0))?;
    let child = tree.sized(20.0, 200.0)?;
    tree.add_child(scroll, child)?;
    tree.layout()?;
    assert_eq!(tree.scroll_into_view(child, ScrollEdge::End)?, true);
    tree.tick_motions(500.0);
    let (_, y) = tree.scroll_offset(scroll)?;
    assert!(y > 1.0, "scrolled {y}");
    Ok(())
}

#[test]
fn fling_target_is_clamped_to_the_scroll_limit() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    let scroll = tree.scroll()?;
    let child = tree.sized(20.0, 80.0)?;
    tree.add_child(scroll, child)?;
    tree.layout()?;
    tree.arm_fling(scroll, 0.0, 10.0)?;
    let (_, y) = tree.fling_target(scroll).expect("coasting");
    let content = tree.rect(child)?;
    let view = tree.rect(scroll)?;
    let limit = (content.h - view.h).max(0.0);
    assert!(y <= limit + 0.01, "target {y} limit {limit}");
    Ok(())
}

#[test]
fn transition_animates_without_picture_writes() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let fade = tree.opacity(254)?;
    let text = tree.text("Hi", 16.0, color())?;
    tree.add_child(fade, text)?;
    tree.layout()?;
    let _ = tree.record()?;
    let epoch = tree.layer_epoch();
    let before = held_pictures(&tree.layer_tree());
    for alpha in [200u8, 128] {
        tree.set_alpha(fade, alpha)?;
        let _ = tree.record()?;
        assert_eq!(tree.picture_writes(), 0, "alpha {alpha}");
        assert!(tree.layer_epoch() > epoch);
        let after = held_pictures(&tree.layer_tree());
        assert_eq!(after.len(), before.len());
        for ((generation, commands), (next_generation, next_commands)) in before.iter().zip(&after)
        {
            assert_eq!(generation, next_generation);
            assert!(std::sync::Arc::ptr_eq(commands, next_commands));
        }
    }
    Ok(())
}

#[test]
fn alpha_flip_255_254_switches_track() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let root = tree.column(0.0)?;
    let stay = tree.text("stay", 16.0, color())?;
    let fade = tree.opacity(255)?;
    let body = tree.text("body", 16.0, color())?;
    tree.add_child(fade, body)?;
    tree.add_child(root, stay)?;
    tree.add_child(root, fade)?;
    tree.layout()?;
    let _ = tree.record()?;
    assert_eq!(tree.picture_cache_len(), 1);

    tree.set_alpha(fade, 254)?;
    let _ = tree.record()?;
    assert!(tree.picture_writes() > 0);
    assert!(
        tree.layer_tree()
            .layers
            .iter()
            .any(|layer| matches!(layer.kind, protocol::LayerKind::Opacity { alpha: 254, .. }))
    );
    let _ = tree.record()?;
    assert_eq!(tree.picture_writes(), 0);

    tree.set_alpha(fade, 255)?;
    let _ = tree.record()?;
    assert!(tree.picture_writes() > 0);
    assert!(
        tree.layer_tree()
            .layers
            .iter()
            .all(|layer| { !matches!(layer.kind, protocol::LayerKind::Opacity { .. }) })
    );
    assert_eq!(tree.picture_cache_len(), 1);
    let _ = tree.record()?;
    assert_eq!(tree.picture_writes(), 0);
    Ok(())
}

fn held_pictures(tree: &protocol::LayerTree) -> Vec<(u64, std::sync::Arc<[protocol::Command]>)> {
    tree.pictures
        .iter()
        .map(|picture| (picture.generation, std::sync::Arc::clone(&picture.commands)))
        .collect()
}
