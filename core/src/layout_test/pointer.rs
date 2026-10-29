use super::*;

#[test]
fn click_fires_once_inside_and_not_after_leaving() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let region = tree.pointer_region()?;
    let child = tree.sized(40.0, 20.0)?;
    tree.add_child(region, child)?;
    tree.layout()?;

    assert_eq!(
        tree.pointer(event(Phase::Down, 10.0, 10.0)),
        vec![Gesture::Press(region)]
    );
    assert_eq!(
        tree.pointer(event(Phase::Up, 10.0, 10.0)),
        vec![Gesture::Release(region), Gesture::Click(region)]
    );

    assert_eq!(
        tree.pointer(event(Phase::Down, 10.0, 10.0)),
        vec![Gesture::Press(region)]
    );
    assert_eq!(
        tree.pointer(event(Phase::Move, 80.0, 80.0)),
        vec![Gesture::Release(region)]
    );
    assert!(tree.pointer(event(Phase::Up, 80.0, 80.0)).is_empty());

    assert_eq!(
        tree.pointer(event(Phase::Down, 10.0, 10.0)),
        vec![Gesture::Press(region)]
    );
    assert_eq!(
        tree.pointer(event(Phase::Cancel, 10.0, 10.0)),
        vec![Gesture::Release(region)]
    );
    assert!(tree.pointer(event(Phase::Up, 10.0, 10.0)).is_empty());
    Ok(())
}

#[test]
fn click_outside_a_clip_misses_the_child() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let clip = tree.clip(0.0)?;
    let region = tree.pointer_region()?;
    let child = tree.sized(30.0, 20.0)?;
    tree.add_child(clip, region)?;
    tree.add_child(region, child)?;
    tree.layout()?;
    let rect = tree.rect(clip)?;
    assert!(
        tree.pointer(event(Phase::Down, rect.x + 1.0, rect.y + 1.0))
            == vec![Gesture::Press(region)]
    );
    assert_eq!(
        tree.pointer(event(Phase::Up, rect.x + 1.0, rect.y + 1.0)),
        vec![Gesture::Release(region), Gesture::Click(region)]
    );
    let outside_x = rect.x + rect.w + 4.0;
    assert!(
        tree.pointer(event(Phase::Down, outside_x, rect.y + 1.0))
            .is_empty()
    );
    assert!(
        tree.pointer(event(Phase::Up, outside_x, rect.y + 1.0))
            .is_empty()
    );
    Ok(())
}

#[test]
fn inserting_during_a_press_keeps_the_route() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let region = tree.pointer_region()?;
    let child = tree.sized(20.0, 10.0)?;
    tree.add_child(region, child)?;
    tree.layout()?;
    assert_eq!(
        tree.pointer(event(Phase::Down, 1.0, 1.0)),
        vec![Gesture::Press(region)]
    );
    let extra = tree.sized(4.0, 4.0)?;
    tree.add_child(region, extra)?;
    tree.layout()?;
    assert_eq!(
        tree.pointer(event(Phase::Up, 1.0, 1.0)),
        vec![Gesture::Release(region), Gesture::Click(region)]
    );
    Ok(())
}

#[test]
fn removing_the_pressed_subtree_drops_the_route() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let column = tree.column(0.0)?;
    let region = tree.pointer_region()?;
    let child = tree.sized(20.0, 10.0)?;
    tree.add_child(region, child)?;
    tree.add_child(column, region)?;
    tree.layout()?;
    assert_eq!(
        tree.pointer(event(Phase::Down, 1.0, 1.0)),
        vec![Gesture::Press(region)]
    );
    tree.remove(column, region)?;
    tree.layout()?;
    assert!(tree.pointer(event(Phase::Up, 1.0, 1.0)).is_empty());
    Ok(())
}

#[test]
fn a_move_inside_the_slop_still_clicks() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let region = tree.pointer_region()?;
    let child = tree.sized(40.0, 40.0)?;
    tree.add_child(region, child)?;
    tree.layout()?;
    assert_eq!(
        tree.pointer(timed(Phase::Down, 10.0, 10.0, 0.0)),
        vec![Gesture::Press(region)]
    );
    assert!(
        tree.pointer(timed(Phase::Move, 10.0, 14.0, 10.0))
            .is_empty()
    );
    assert_eq!(
        tree.pointer(timed(Phase::Up, 10.0, 14.0, 20.0)),
        vec![Gesture::Release(region), Gesture::Click(region)]
    );
    Ok(())
}

#[test]
fn a_move_past_the_slop_without_a_scroll_still_clicks() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let region = tree.pointer_region()?;
    let child = tree.sized(80.0, 80.0)?;
    tree.add_child(region, child)?;
    tree.layout()?;
    assert_eq!(
        tree.pointer(event(Phase::Down, 10.0, 10.0)),
        vec![Gesture::Press(region)]
    );
    assert!(
        tree.pointer(event(Phase::Move, 10.0, 30.0)).is_empty(),
        "no scroll means the press stays"
    );
    assert_eq!(
        tree.pointer(event(Phase::Up, 10.0, 30.0)),
        vec![Gesture::Release(region), Gesture::Click(region)]
    );
    Ok(())
}

#[test]
fn a_drag_on_empty_scroll_space_scrolls() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 120.0, 80.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll()?;
    let content = tree.column(0.0)?;
    let label = tree.text("Item", 16.0, [0, 0, 0, 255])?;
    let tail = tree.sized(20.0, 400.0)?;
    tree.add_child(content, label)?;
    tree.add_child(content, tail)?;
    tree.add_child(scroll, content)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;
    let view = tree.rect(scroll)?;
    let label_box = tree.rect(label)?;
    let x = label_box.x + label_box.w + 8.0;
    let y = view.y + 8.0;
    assert!(x < view.x + view.w, "empty space x {x} view {view:?}");
    assert!(
        tree.pointer(timed(Phase::Down, x, y, 0.0)).is_empty(),
        "empty scroll space is not a press"
    );
    let moved = tree.pointer(timed(Phase::Move, x, y + 24.0, 20.0));
    assert!(
        moved
            .iter()
            .any(|gesture| matches!(gesture, Gesture::DragBegin(id) if *id == scroll)),
        "drag {moved:?}"
    );
    Ok(())
}

#[test]
fn passing_the_slop_over_a_scroll_locks_a_drag() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll()?;
    let region = tree.pointer_region()?;
    let child = tree.sized(80.0, 200.0)?;
    tree.add_child(region, child)?;
    tree.add_child(scroll, region)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;

    assert_eq!(
        tree.pointer(timed(Phase::Down, 10.0, 10.0, 0.0)),
        vec![Gesture::Press(region)]
    );
    assert!(
        tree.pointer(timed(Phase::Move, 10.0, 14.0, 10.0))
            .is_empty()
    );
    assert_eq!(
        tree.pointer(timed(Phase::Move, 10.0, 30.0, 26.0)),
        vec![
            Gesture::Release(region),
            Gesture::DragBegin(scroll),
            Gesture::DragBy(scroll, 0.0, 16.0),
        ]
    );
    assert_eq!(
        tree.pointer(timed(Phase::Move, 10.0, 46.0, 42.0)),
        vec![Gesture::DragBy(scroll, 0.0, 16.0)]
    );
    assert_eq!(
        tree.pointer(timed(Phase::Up, 10.0, 46.0, 42.0)),
        vec![Gesture::DragEnd(scroll, 0.0, 1.0)]
    );
    Ok(())
}

#[test]
fn cancel_during_a_drag_ends_with_zero_velocity() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll()?;
    let text = tree.image("asset:a", 40.0, 200.0)?;
    tree.add_child(scroll, text)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;
    assert!(tree.pointer(timed(Phase::Down, 1.0, 1.0, 0.0)).is_empty());
    let locked = tree.pointer(timed(Phase::Move, 1.0, 20.0, 10.0));
    assert_eq!(
        locked,
        vec![
            Gesture::DragBegin(scroll),
            Gesture::DragBy(scroll, 0.0, 19.0)
        ]
    );
    assert_eq!(
        tree.pointer(timed(Phase::Cancel, 1.0, 20.0, 12.0)),
        vec![Gesture::DragEnd(scroll, 0.0, 0.0)]
    );
    assert!(tree.pointer(event(Phase::Up, 1.0, 20.0)).is_empty());
    Ok(())
}

#[test]
fn structure_change_drops_a_locked_drag() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll()?;
    let region = tree.pointer_region()?;
    let child = tree.sized(40.0, 200.0)?;
    tree.add_child(region, child)?;
    tree.add_child(scroll, region)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;
    let _ = tree.pointer(event(Phase::Down, 4.0, 4.0));
    let _ = tree.pointer(event(Phase::Move, 4.0, 20.0));
    let extra = tree.sized(4.0, 4.0)?;
    tree.add_child(scroll, extra)?;
    tree.layout()?;
    let up = tree.pointer(event(Phase::Up, 4.0, 20.0));
    assert!(
        up.iter()
            .any(|gesture| matches!(gesture, Gesture::DragEnd(id, _, _) if *id == scroll)),
        "inserting a row keeps the scroll drag, got {up:?}"
    );
    Ok(())
}

#[test]
fn a_filled_page_pointer_keeps_the_list_shorter_than_its_content() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 360.0, 640.0)?;
    let root = tree.stack()?;
    tree.set_fill(root)?;
    let content = tree.column(0.0)?;
    tree.set_fill(content)?;
    let app_stack = tree.stack()?;
    tree.set_fill(app_stack)?;
    let app = tree.column(8.0)?;
    tree.set_fill(app)?;
    let nav = tree.sized(100.0, 44.0)?;
    let router = tree.clip(0.0)?;
    tree.set_fill(router)?;
    let route = tree.stack()?;
    tree.set_fill(route)?;
    let fade = tree.opacity(255)?;
    tree.set_fill(fade)?;
    let mover = tree.transform()?;
    tree.set_fill(mover)?;
    let page_pointer = tree.pointer_region()?;
    tree.set_fill(page_pointer)?;
    let backdrop = tree.decorated([255, 255, 255, 255], 0.0)?;
    tree.set_fill(backdrop)?;
    let page = tree.column(8.0)?;
    tree.set_fill(page)?;
    let header = tree.sized(100.0, 40.0)?;
    let list = tree.stack()?;
    tree.set_fill(list)?;
    let scroll = tree.scroll()?;
    let body = tree.column(0.0)?;
    let sample = tree.text("Item 0", 16.0, [0, 0, 0, 255])?;
    let spacer = tree.sized(1.0, 4000.0)?;
    tree.add_child(body, sample)?;
    tree.add_child(body, spacer)?;
    tree.add_child(scroll, body)?;
    tree.add_child(list, scroll)?;
    tree.add_child(page, header)?;
    tree.add_child(page, list)?;
    tree.add_child(backdrop, page)?;
    tree.add_child(page_pointer, backdrop)?;
    tree.add_child(mover, page_pointer)?;
    tree.add_child(fade, mover)?;
    tree.add_child(route, fade)?;
    tree.add_child(router, route)?;
    tree.add_child(app, nav)?;
    tree.add_child(app, router)?;
    tree.add_child(app_stack, app)?;
    tree.add_child(content, app_stack)?;
    tree.add_child(root, content)?;
    tree.layout()?;
    let scroll_box = tree.rect(scroll)?;
    let body_box = tree.rect(body)?;
    let page_box = tree.rect(page)?;
    assert!(
        scroll_box.h + 1.0 < body_box.h,
        "scroll {scroll_box:?} body {body_box:?} page {page_box:?}"
    );
    let sample_box = tree.rect(sample)?;
    let _ = tree.pointer(event(Phase::Down, sample_box.x + 2.0, sample_box.y + 2.0));
    let moved = tree.pointer(timed(
        Phase::Move,
        sample_box.x + 2.0,
        sample_box.y + 2.0 - 40.0,
        16.0,
    ));
    assert!(
        moved
            .iter()
            .any(|gesture| matches!(gesture, Gesture::DragBy(_, _, dy) if *dy < 0.0)),
        "drag {moved:?} scroll {scroll_box:?} body {body_box:?}"
    );
    Ok(())
}

#[test]
fn click_moves_focus_and_a_locked_drag_does_not() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let scroll = tree.scroll()?;
    let column = tree.column(0.0)?;
    let field = tree.pointer_region()?;
    let plain = tree.pointer_region()?;
    let field_box = tree.sized(40.0, 20.0)?;
    let plain_box = tree.sized(40.0, 20.0)?;
    let tail = tree.sized(40.0, 200.0)?;
    tree.set_focusable(field, true)?;
    tree.set_fill(scroll)?;
    tree.add_child(field, field_box)?;
    tree.add_child(plain, plain_box)?;
    tree.add_child(column, field)?;
    tree.add_child(column, plain)?;
    tree.add_child(column, tail)?;
    tree.add_child(scroll, column)?;
    tree.set_viewport(0.0, 0.0, 40.0, 80.0)?;
    tree.layout()?;
    let field_rect = tree.rect(field)?;
    let plain_rect = tree.rect(plain)?;
    assert!(
        tree.pointer(event(Phase::Down, field_rect.x + 1.0, field_rect.y + 1.0))
            == vec![Gesture::Press(field)]
    );
    assert_eq!(
        tree.pointer(event(Phase::Up, field_rect.x + 1.0, field_rect.y + 1.0)),
        vec![
            Gesture::Release(field),
            Gesture::Click(field),
            Gesture::Focus(field)
        ]
    );
    assert_eq!(tree.focused(), Some(field));
    assert_eq!(
        tree.pointer(event(Phase::Down, field_rect.x + 1.0, field_rect.y + 1.0)),
        vec![Gesture::Press(field)]
    );
    assert_eq!(
        tree.pointer(event(Phase::Up, field_rect.x + 1.0, field_rect.y + 1.0)),
        vec![Gesture::Release(field), Gesture::Click(field)]
    );
    assert_eq!(
        tree.pointer(event(Phase::Down, plain_rect.x + 1.0, plain_rect.y + 1.0)),
        vec![Gesture::Press(plain)]
    );
    assert_eq!(
        tree.pointer(event(Phase::Up, plain_rect.x + 1.0, plain_rect.y + 1.0)),
        vec![
            Gesture::Release(plain),
            Gesture::Click(plain),
            Gesture::Blur(field)
        ]
    );
    assert_eq!(tree.focused(), None);
    tree.focus(field)?;
    assert_eq!(tree.focused(), Some(field));
    let dragged = tree.pointer(timed(
        Phase::Down,
        field_rect.x + 1.0,
        field_rect.y + 1.0,
        0.0,
    ));
    assert_eq!(dragged, vec![Gesture::Press(field)]);
    let locked = tree.pointer(timed(
        Phase::Move,
        field_rect.x + 1.0,
        field_rect.y + 20.0,
        10.0,
    ));
    assert!(
        locked
            .iter()
            .any(|gesture| matches!(gesture, Gesture::DragBegin(_)))
    );
    assert!(
        locked
            .iter()
            .all(|gesture| !matches!(gesture, Gesture::Focus(_) | Gesture::Blur(_)))
    );
    assert_eq!(tree.focused(), Some(field));
    let ended = tree.pointer(timed(
        Phase::Up,
        field_rect.x + 1.0,
        field_rect.y + 20.0,
        20.0,
    ));
    assert!(
        ended
            .iter()
            .all(|gesture| matches!(gesture, Gesture::DragEnd(_, _, _)))
    );
    assert_eq!(tree.focused(), Some(field));
    Ok(())
}

#[test]
fn programmatic_focus_remove_and_insert() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let column = tree.column(0.0)?;
    let field = tree.pointer_region()?;
    tree.set_focusable(field, true)?;
    tree.set_focusable(column, true)?;
    tree.add_child(column, field)?;
    tree.layout()?;
    let plain = tree.sized(1.0, 1.0)?;
    assert!(matches!(tree.focus(plain), Err(crate::Error::NotFocusable)));
    assert_eq!(tree.focus(field)?, vec![Gesture::Focus(field)]);
    assert!(tree.focus(field)?.is_empty());
    let extra = tree.sized(4.0, 4.0)?;
    tree.add_child(column, extra)?;
    assert_eq!(tree.focused(), Some(field));
    assert_eq!(tree.remove(column, field)?, vec![Gesture::Blur(field)]);
    assert_eq!(tree.focused(), None);
    assert_eq!(tree.focus(column)?, vec![Gesture::Focus(column)]);
    let discarded = tree.discard(column)?;
    assert_eq!(discarded.gestures, vec![Gesture::Blur(column)]);
    assert!(discarded.ids.contains(&column));
    assert_eq!(tree.focused(), None);
    Ok(())
}

#[test]
fn hover_enters_and_leaves_and_a_wheel_does_not_scroll() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let scroll = tree.scroll()?;
    let child = tree.image("asset:a", 40.0, 80.0)?;
    tree.set_max_size(scroll, 40.0, 40.0)?;
    tree.add_child(scroll, child)?;
    tree.set_viewport(0.0, 0.0, 40.0, 40.0)?;
    tree.layout()?;
    assert_eq!(
        tree.pointer(event(Phase::Move, 10.0, 10.0)),
        vec![Gesture::Hover(child)]
    );
    assert_eq!(
        tree.pointer(event(Phase::Move, 80.0, 80.0)),
        vec![Gesture::Leave(child)]
    );
    assert_eq!(
        tree.pointer(PointerEvent::wheel(10.0, 10.0, 0.0, 0.0, 12.0)),
        vec![Gesture::Wheel(scroll, 0.0, 12.0)]
    );
    assert_eq!(tree.scroll_offset(scroll)?, (0.0, 0.0));
    Ok(())
}

#[test]
fn secondary_click_does_not_press_or_focus() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let region = tree.pointer_region()?;
    let child = tree.sized(40.0, 20.0)?;
    tree.set_focusable(region, true)?;
    tree.add_child(region, child)?;
    tree.layout()?;
    let down = button_event(Phase::Down, 10.0, 10.0, Button::Secondary);
    let up = button_event(Phase::Up, 10.0, 10.0, Button::Secondary);
    assert!(tree.pointer(down).is_empty());
    assert_eq!(tree.pointer(up), vec![Gesture::SecondaryClick(region)]);
    assert_eq!(tree.focused(), None);
    Ok(())
}

#[test]
fn secondary_move_past_the_slop_cancels_the_click() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let region = tree.pointer_region()?;
    let child = tree.sized(80.0, 80.0)?;
    tree.add_child(region, child)?;
    tree.layout()?;
    assert!(
        tree.pointer(button_event(Phase::Down, 10.0, 10.0, Button::Secondary))
            .is_empty()
    );
    assert!(
        tree.pointer(button_event(Phase::Move, 10.0, 19.0, Button::Secondary))
            .is_empty()
    );
    assert!(
        tree.pointer(button_event(Phase::Up, 10.0, 19.0, Button::Secondary))
            .is_empty()
    );
    Ok(())
}

#[test]
fn secondary_during_a_primary_drag_leaves_the_drag_end() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll()?;
    let region = tree.pointer_region()?;
    let child = tree.sized(80.0, 200.0)?;
    tree.add_child(region, child)?;
    tree.add_child(scroll, region)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;
    assert_eq!(
        tree.pointer(timed(Phase::Down, 10.0, 10.0, 0.0)),
        vec![Gesture::Press(region)]
    );
    let locked = tree.pointer(timed(Phase::Move, 10.0, 30.0, 26.0));
    assert!(
        locked
            .iter()
            .any(|gesture| matches!(gesture, Gesture::DragBegin(_)))
    );
    assert!(
        tree.pointer(button_event(Phase::Down, 12.0, 32.0, Button::Secondary))
            .is_empty()
    );
    assert_eq!(
        tree.pointer(button_event(Phase::Up, 12.0, 32.0, Button::Secondary)),
        vec![Gesture::SecondaryClick(region)]
    );
    assert!(
        tree.pointer(timed(Phase::Up, 10.0, 30.0, 40.0))
            .iter()
            .any(|gesture| matches!(gesture, Gesture::DragEnd(_, _, _)))
    );
    Ok(())
}

#[test]
fn auxiliary_click_is_ignored() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let region = tree.pointer_region()?;
    let child = tree.sized(40.0, 20.0)?;
    tree.set_focusable(region, true)?;
    tree.add_child(region, child)?;
    tree.layout()?;
    assert!(
        tree.pointer(button_event(Phase::Down, 10.0, 10.0, Button::Auxiliary))
            .is_empty()
    );
    assert!(
        tree.pointer(button_event(Phase::Up, 10.0, 10.0, Button::Auxiliary))
            .is_empty()
    );
    assert_eq!(tree.focused(), None);
    Ok(())
}

#[test]
fn a_switch_hint_marks_checked_and_activate_clicks() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let region = tree.pointer_region()?;
    let child = tree.sized(40.0, 20.0)?;
    tree.add_child(region, child)?;
    tree.set_semantics(
        region,
        crate::SemanticsHint {
            role: Some(crate::SemanticsRole::Switch),
            label: Some("On".to_string()),
            checked: Some(true),
            ..crate::SemanticsHint::default()
        },
    )?;
    tree.layout()?;
    let update = tree.semantics().expect("semantics");
    let node = update
        .nodes
        .iter()
        .find(|node| node.id == crate::SemanticsId::from_node(region))
        .expect("switch");
    assert_eq!(node.role, crate::SemanticsRole::Switch);
    assert!(node.flags.contains(crate::SemanticsFlags::CHECKED));
    assert!(
        tree.semantics_action(crate::SemanticsAction::Activate(node.id))
            .contains(&Gesture::Click(region))
    );
    Ok(())
}

#[test]
fn a_second_pointer_does_not_cancel_the_first() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let region = tree.pointer_region()?;
    let child = tree.sized(40.0, 20.0)?;
    tree.add_child(region, child)?;
    tree.layout()?;
    let mut second = event(Phase::Down, 10.0, 10.0);
    second.pointer = PointerId(1);
    assert_eq!(
        tree.pointer(event(Phase::Down, 10.0, 10.0)),
        vec![Gesture::Press(region)]
    );
    assert_eq!(tree.pointer(second), vec![Gesture::Press(region)]);
    assert_eq!(
        tree.pointer(event(Phase::Up, 10.0, 10.0)),
        vec![Gesture::Release(region), Gesture::Click(region)]
    );
    second.phase = Phase::Up;
    assert_eq!(
        tree.pointer(second),
        vec![Gesture::Release(region), Gesture::Click(region)]
    );
    Ok(())
}

#[test]
fn a_drag_at_the_inner_edge_locks_the_outer_scroll() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 80.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let outer = tree.scroll()?;
    let column = tree.column(0.0)?;
    let inner = tree.scroll()?;
    tree.set_max_size(inner, 80.0, 40.0)?;
    let inner_body = tree.image("asset:a", 80.0, 200.0)?;
    let tail = tree.sized(80.0, 200.0)?;
    tree.add_child(inner, inner_body)?;
    tree.add_child(column, inner)?;
    tree.add_child(column, tail)?;
    tree.add_child(outer, column)?;
    tree.add_child(root, outer)?;
    tree.layout()?;
    tree.set_scroll_offset(outer, 0.0, 30.0)?;
    tree.layout()?;
    assert!(tree.rect(inner)?.h + 1.0 < tree.rect(inner_body)?.h);
    let _ = tree.pointer(timed(Phase::Down, 10.0, 4.0, 0.0));
    let locked = tree.pointer(timed(Phase::Move, 10.0, 24.0, 16.0));
    assert!(
        locked.contains(&Gesture::DragBegin(outer)),
        "inner offset is zero, so a downward drag goes to the outer scroll: {locked:?}"
    );
    Ok(())
}

#[test]
fn a_second_finger_does_not_drag_the_same_scroll() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll()?;
    let body = tree.image("asset:a", 80.0, 200.0)?;
    tree.add_child(scroll, body)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;
    tree.set_scroll_offset(scroll, 0.0, 40.0)?;
    tree.layout()?;
    assert!(matches!(
        tree.pointer(timed(Phase::Down, 10.0, 10.0, 0.0)).as_slice(),
        []
    ));
    let first = tree.pointer(timed(Phase::Move, 10.0, 30.0, 16.0));
    assert!(first.contains(&Gesture::DragBegin(scroll)));
    let mut second = timed(Phase::Down, 20.0, 10.0, 20.0);
    second.pointer = PointerId(1);
    let _ = tree.pointer(second);
    second.phase = Phase::Move;
    second.y = 40.0;
    let extra = tree.pointer(second);
    assert!(
        !extra
            .iter()
            .any(|gesture| matches!(gesture, Gesture::DragBegin(_))),
        "the scroll already has a finger: {extra:?}"
    );
    Ok(())
}

#[test]
fn a_horizontal_claim_keeps_a_sideways_drag_off_the_scroll() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll()?;
    let region = tree.pointer_region()?;
    let child = tree.sized(80.0, 200.0)?;
    tree.add_child(region, child)?;
    tree.add_child(scroll, region)?;
    tree.add_child(root, scroll)?;
    tree.set_drag_claim(region, crate::DragClaim::Horizontal)?;
    tree.layout()?;
    assert_eq!(
        tree.pointer(event(Phase::Down, 10.0, 10.0)),
        vec![Gesture::Press(region)]
    );
    let sideways = tree.pointer(event(Phase::Move, 30.0, 12.0));
    assert!(
        sideways.contains(&Gesture::DragBegin(region)),
        "horizontal claim wins: {sideways:?}"
    );
    assert!(!sideways.contains(&Gesture::Release(region)));
    Ok(())
}

#[test]
fn focus_step_stays_inside_the_scope() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let outer = tree.pointer_region()?;
    let scope = tree.column(0.0)?;
    let inner = tree.pointer_region()?;
    let box_a = tree.sized(20.0, 20.0)?;
    let box_b = tree.sized(20.0, 20.0)?;
    tree.add_child(outer, box_a)?;
    tree.add_child(inner, box_b)?;
    tree.add_child(scope, inner)?;
    tree.add_child(outer, scope)?;
    tree.set_focusable(outer, true)?;
    tree.set_focusable(inner, true)?;
    tree.layout()?;
    tree.push_focus_scope(scope)?;
    assert_eq!(
        tree.focus_step(crate::FocusDirection::Next),
        vec![Gesture::Focus(inner)]
    );
    assert_eq!(
        tree.focus_step(crate::FocusDirection::Next),
        Vec::<Gesture>::new()
    );
    tree.pop_focus_scope();
    let stepped = tree.focus_step(crate::FocusDirection::Next);
    assert_eq!(stepped, vec![Gesture::Blur(inner), Gesture::Focus(outer)]);
    Ok(())
}

#[test]
fn remove_pops_a_focus_scope_inside_the_subtree() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let root = tree.column(0.0)?;
    let outer = tree.pointer_region()?;
    let scope = tree.column(0.0)?;
    let inner = tree.pointer_region()?;
    let box_a = tree.sized(20.0, 20.0)?;
    let box_b = tree.sized(20.0, 20.0)?;
    tree.add_child(outer, box_a)?;
    tree.add_child(inner, box_b)?;
    tree.add_child(scope, inner)?;
    tree.add_child(outer, scope)?;
    tree.add_child(root, outer)?;
    tree.set_focusable(outer, true)?;
    tree.set_focusable(inner, true)?;
    tree.layout()?;
    tree.push_focus_scope(scope)?;
    tree.remove(outer, scope)?;
    assert_eq!(
        tree.focus_step(crate::FocusDirection::Next),
        vec![Gesture::Focus(outer)]
    );
    Ok(())
}

#[test]
fn a_long_press_wins_and_the_lift_is_not_a_click() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let region = tree.pointer_region()?;
    let child = tree.sized(40.0, 40.0)?;
    tree.add_child(region, child)?;
    tree.set_recognizers(
        region,
        RecognizerSet {
            long_press: true,
            ..RecognizerSet::default()
        },
    )?;
    tree.layout()?;
    assert_eq!(
        tree.pointer(timed(Phase::Down, 8.0, 8.0, 0.0)),
        vec![Gesture::Press(region)]
    );
    assert_eq!(
        tree.pointer(timed(Phase::Move, 8.0, 8.0, 500.0)),
        vec![Gesture::LongPress(region)]
    );
    assert_eq!(
        tree.pointer(timed(Phase::Up, 8.0, 8.0, 600.0)),
        vec![Gesture::Release(region)]
    );
    Ok(())
}

#[test]
fn a_horizontal_recognizer_rejects_a_vertical_swipe() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll()?;
    let region = tree.pointer_region()?;
    let child = tree.sized(80.0, 200.0)?;
    tree.add_child(region, child)?;
    tree.add_child(scroll, region)?;
    tree.add_child(root, scroll)?;
    tree.set_recognizers(
        region,
        RecognizerSet {
            drag: Some(DragAxis::Horizontal),
            ..RecognizerSet::default()
        },
    )?;
    tree.layout()?;
    tree.pointer(timed(Phase::Down, 10.0, 10.0, 0.0));
    let vertical = tree.pointer(timed(Phase::Move, 10.0, 40.0, 20.0));
    assert!(
        vertical.contains(&Gesture::DragBegin(scroll)),
        "vertical swipe scrolls: {vertical:?}"
    );
    assert!(
        !vertical.contains(&Gesture::DragBegin(region)),
        "horizontal recognizer rejected: {vertical:?}"
    );
    Ok(())
}

#[test]
fn a_second_finger_does_not_join_a_locked_scroll() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll()?;
    let region = tree.pointer_region()?;
    let child = tree.sized(80.0, 200.0)?;
    tree.add_child(region, child)?;
    tree.add_child(scroll, region)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;
    tree.pointer(timed(Phase::Down, 10.0, 10.0, 0.0));
    let locked = tree.pointer(timed(Phase::Move, 10.0, 40.0, 20.0));
    assert!(locked.contains(&Gesture::DragBegin(scroll)));
    let mut second = timed(Phase::Down, 12.0, 12.0, 30.0);
    second.pointer = PointerId(2);
    tree.pointer(second);
    let mut moved = timed(Phase::Move, 12.0, 50.0, 50.0);
    moved.pointer = PointerId(2);
    let gestures = tree.pointer(moved);
    assert!(
        gestures
            .iter()
            .all(|gesture| !matches!(gesture, Gesture::DragBegin(_))),
        "second finger stays out: {gestures:?}"
    );
    Ok(())
}
