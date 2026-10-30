use super::*;

#[test]
fn repeating_scroll_offset_keeps_the_layer_epoch() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let scroll = tree.scroll()?;
    tree.set_max_size(scroll, 40.0, 40.0)?;
    let body = tree.sized(20.0, 200.0)?;
    tree.add_child(scroll, body)?;
    tree.layout()?;
    tree.set_scroll_offset(scroll, 0.0, 30.0)?;
    let epoch = tree.layer_epoch();
    tree.set_scroll_offset(scroll, 0.0, 30.0)?;
    assert_eq!(tree.layer_epoch(), epoch);
    assert!(!tree.needs_layout());
    assert_eq!(tree.scroll_offset(scroll)?, (0.0, 30.0));
    Ok(())
}

#[test]
fn record_omits_a_row_outside_the_scroll_viewport() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 40.0, 40.0)?;
    let scroll = tree.scroll()?;
    tree.set_max_size(scroll, 40.0, 40.0)?;
    let column = tree.column(0.0)?;
    let near = tree.text("near", 16.0, [0, 0, 0, 255])?;
    let gap = tree.sized(20.0, 40.0)?;
    let far = tree.text("far", 16.0, [0, 0, 0, 255])?;
    tree.add_child(column, near)?;
    tree.add_child(column, gap)?;
    tree.add_child(column, far)?;
    tree.add_child(scroll, column)?;
    tree.layout()?;
    tree.set_scroll_offset(scroll, 0.0, 0.0)?;
    let list = tree.record()?;
    let labels: Vec<_> = list
        .commands
        .iter()
        .filter_map(|command| match command {
            protocol::Command::Text { utf8, .. } => Some(utf8.as_ref()),
            _ => None,
        })
        .collect();
    assert!(labels.contains(&"near"), "{labels:?}");
    assert!(!labels.contains(&"far"), "{labels:?}");
    Ok(())
}

#[test]
fn fling_changes_paint_without_another_layout() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 40.0, 40.0)?;
    let scroll = tree.scroll()?;
    let body = tree.sized(40.0, 200.0)?;
    tree.set_max_size(scroll, 40.0, 40.0)?;
    tree.add_child(scroll, body)?;
    tree.layout()?;
    let passes = tree.layout_passes();
    tree.set_scroll_offset(scroll, 0.0, 80.0)?;
    tree.arm_fling(scroll, 0.0, 1.0)?;
    assert!(tree.tick_fling(16.0));
    assert!(tree.scroll_offset(scroll)?.1 < 80.0);
    assert_eq!(tree.layout_passes(), passes);
    assert!(tree.needs_paint());
    let _ = tree.record()?;
    tree.clear_paint();
    assert!(!tree.needs_paint());
    Ok(())
}

#[test]
fn scroll_offset_moves_paint_and_hits_without_moving_layout() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 80.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let header = tree.sized(20.0, 20.0)?;
    let scroll = tree.scroll()?;
    let content = tree.column(0.0)?;
    let spacer = tree.sized(20.0, 40.0)?;
    let label = tree.text("B", 16.0, [1, 2, 3, 4])?;
    let tail = tree.sized(20.0, 200.0)?;
    tree.add_child(content, spacer)?;
    tree.add_child(content, label)?;
    tree.add_child(content, tail)?;
    tree.add_child(scroll, content)?;
    tree.add_child(root, header)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;

    let view = tree.rect(scroll)?;
    let body = tree.rect(content)?;
    let header_box = tree.rect(header)?;
    let label_box = tree.rect(label)?;
    assert!(
        view.h + 1.0 < body.h,
        "viewport {} should be shorter than content {}",
        view.h,
        body.h
    );
    assert!(view.h <= 80.0, "viewport grew past the window: {}", view.h);

    let passes = tree.layout_passes();
    tree.set_scroll_offset(scroll, 0.0, 30.0)?;
    assert!(tree.needs_paint());
    assert!(!tree.needs_layout());
    tree.layout()?;
    assert_eq!(tree.layout_passes(), passes);
    assert_eq!(tree.scroll_offset(scroll)?, (0.0, 30.0));
    tree.layout()?;
    assert_eq!(tree.rect(header)?, header_box);
    assert_eq!(tree.rect(label)?, label_box);

    let list = tree.record()?;
    let text = list.commands.iter().find_map(|command| match command {
        protocol::Command::Text { utf8, x, y, .. } if utf8.as_ref() == "B" => Some((*x, *y)),
        _ => None,
    });
    assert_eq!(text, Some((label_box.x, label_box.y - 30.0)));
    assert!(
        list.commands.iter().any(|command| matches!(
            command,
            protocol::Command::PushClip { x, y, w, h, corner_radius }
                if *corner_radius == 0.0
                    && *x == view.x
                    && *y == view.y
                    && *w == view.w
                    && *h == view.h
        )),
        "clip should be the viewport, commands: {:?}",
        list.commands
    );
    Ok(())
}

#[test]
fn scroll_hit_uses_the_offset_and_stays_inside_the_viewport() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 80.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll()?;
    let content = tree.column(0.0)?;
    let spacer = tree.sized(20.0, 40.0)?;
    let label = tree.text("B", 16.0, [1, 2, 3, 4])?;
    let tail = tree.sized(20.0, 200.0)?;
    tree.add_child(content, spacer)?;
    tree.add_child(content, label)?;
    tree.add_child(content, tail)?;
    tree.add_child(scroll, content)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;
    let label_box = tree.rect(label)?;
    assert!(tree.rect(content)?.h > tree.rect(scroll)?.h);
    tree.set_scroll_offset(scroll, 0.0, 30.0)?;
    tree.layout()?;

    let painted_y = label_box.y - 30.0;
    assert!(
        tree.pointer(event(Phase::Down, label_box.x + 1.0, painted_y + 1.0))
            .is_empty(),
        "the label has no pointer region, so a hit is not a press"
    );
    assert_eq!(
        tree.pointer(event(Phase::Move, label_box.x + 1.0, painted_y + 12.0)),
        vec![
            Gesture::DragBegin(scroll),
            Gesture::DragBy(scroll, 0.0, 11.0)
        ]
    );

    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 80.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll()?;
    let content = tree.column(0.0)?;
    let spacer = tree.sized(20.0, 40.0)?;
    let label = tree.text("B", 16.0, [1, 2, 3, 4])?;
    let tail = tree.sized(20.0, 200.0)?;
    tree.add_child(content, spacer)?;
    tree.add_child(content, label)?;
    tree.add_child(content, tail)?;
    tree.add_child(scroll, content)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;
    let view = tree.rect(scroll)?;
    let label_box = tree.rect(label)?;
    tree.set_scroll_offset(scroll, 0.0, 30.0)?;
    tree.layout()?;
    assert!(
        tree.pointer(event(Phase::Down, label_box.x + 1.0, view.y + view.h + 4.0))
            .is_empty(),
        "content below the viewport is not a hit"
    );
    assert!(
        tree.pointer(event(Phase::Down, label_box.x + 1.0, label_box.y + 1.0))
            .is_empty(),
        "the unshifted layout point is not where the label is painted"
    );
    Ok(())
}

#[test]
fn nested_fill_keeps_the_scroll_inside_the_viewport() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 320.0, 180.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let app = tree.column(8.0)?;
    tree.set_fill(app)?;
    let header = tree.sized(100.0, 40.0)?;
    let scroll = tree.scroll()?;
    let content = tree.sized(100.0, 800.0)?;
    tree.add_child(scroll, content)?;
    tree.add_child(app, header)?;
    tree.add_child(app, scroll)?;
    tree.add_child(root, app)?;
    tree.layout()?;
    let root_h = tree.rect(root)?.h;
    let app_h = tree.rect(app)?.h;
    let scroll_h = tree.rect(scroll)?.h;
    let content_h = tree.rect(content)?.h;
    assert!(
        root_h <= 180.5 && app_h <= 180.5 && scroll_h + 1.0 < content_h,
        "root {root_h} app {app_h} scroll {scroll_h} content {content_h}"
    );
    Ok(())
}

#[test]
fn a_filled_stack_keeps_its_scroll_inside_the_parent() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 320.0, 180.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let app = tree.column(8.0)?;
    tree.set_fill(app)?;
    let header = tree.sized(100.0, 40.0)?;
    let stack = tree.stack()?;
    tree.set_fill(stack)?;
    let scroll = tree.scroll()?;
    let content = tree.sized(100.0, 800.0)?;
    tree.add_child(scroll, content)?;
    tree.add_child(stack, scroll)?;
    tree.add_child(app, header)?;
    tree.add_child(app, stack)?;
    tree.add_child(root, app)?;
    tree.layout()?;
    let stack_h = tree.rect(stack)?.h;
    let scroll_h = tree.rect(scroll)?.h;
    let content_h = tree.rect(content)?.h;
    assert!(
        stack_h + 1.0 < content_h && scroll_h + 1.0 < content_h,
        "stack {stack_h} scroll {scroll_h} content {content_h}"
    );
    Ok(())
}

#[test]
fn a_nested_filled_stack_scroll_keeps_a_viewport() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 320.0, 180.0)?;
    let app = tree.column(8.0)?;
    tree.set_fill(app)?;
    let nav = tree.sized(100.0, 40.0)?;
    let router = tree.stack()?;
    tree.set_fill(router)?;
    let fade = tree.opacity(255)?;
    tree.set_fill(fade)?;
    let mover = tree.transform()?;
    tree.set_fill(mover)?;
    let page = tree.column(8.0)?;
    tree.set_fill(page)?;
    let header = tree.sized(100.0, 30.0)?;
    let list = tree.stack()?;
    tree.set_fill(list)?;
    let scroll = tree.scroll()?;
    let content = tree.sized(100.0, 800.0)?;
    tree.add_child(scroll, content)?;
    tree.add_child(list, scroll)?;
    tree.add_child(page, header)?;
    tree.add_child(page, list)?;
    tree.add_child(mover, page)?;
    tree.add_child(fade, mover)?;
    tree.add_child(router, fade)?;
    tree.add_child(app, nav)?;
    tree.add_child(app, router)?;
    tree.layout()?;
    let scroll_h = tree.rect(scroll)?.h;
    let app_h = tree.rect(app)?.h;
    assert!(
        app_h <= 180.5 && scroll_h > 40.0 && scroll_h + 1.0 < 800.0,
        "app {app_h} scroll {scroll_h}"
    );
    Ok(())
}

#[test]
fn a_spacer_taller_than_the_viewport_overflows_the_scroll() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 360.0, 800.0)?;
    let root = tree.stack()?;
    tree.set_fill(root)?;
    let content = tree.column(0.0)?;
    tree.set_fill(content)?;
    let app_stack = tree.stack()?;
    tree.set_fill(app_stack)?;
    let app = tree.column(8.0)?;
    tree.set_fill(app)?;
    let nav = tree.sized(100.0, 40.0)?;
    let router = tree.stack()?;
    tree.set_fill(router)?;
    let route = tree.stack()?;
    tree.set_fill(route)?;
    let fade = tree.opacity(255)?;
    tree.set_fill(fade)?;
    let mover = tree.transform()?;
    tree.set_fill(mover)?;
    let page = tree.column(8.0)?;
    tree.set_fill(page)?;
    let header = tree.sized(100.0, 30.0)?;
    let list = tree.stack()?;
    tree.set_fill(list)?;
    let scroll = tree.scroll()?;
    let body = tree.column(0.0)?;
    let sample = tree.text("Item", 16.0, [0, 0, 0, 255])?;
    let spacer = tree.sized(1.0, 5488.0)?;
    tree.add_child(body, sample)?;
    tree.add_child(body, spacer)?;
    tree.add_child(scroll, body)?;
    tree.add_child(list, scroll)?;
    tree.add_child(page, header)?;
    tree.add_child(page, list)?;
    tree.add_child(mover, page)?;
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
    let list_box = tree.rect(list)?;
    let page_box = tree.rect(page)?;
    let fade_box = tree.rect(fade)?;
    let sample_box = tree.rect(sample)?;
    let _ = tree.pointer(event(Phase::Down, sample_box.x + 1.0, sample_box.y + 1.0));
    let moved = tree.pointer(timed(
        Phase::Move,
        sample_box.x + 1.0,
        sample_box.y + 1.0 - 40.0,
        16.0,
    ));
    assert!(
        moved
            .iter()
            .any(|gesture| matches!(gesture, Gesture::DragBy(_, _, dy) if *dy < 0.0)),
        "drag {moved:?} scroll {scroll_box:?} list {list_box:?} page {page_box:?} fade {fade_box:?} sample {sample_box:?}"
    );
    Ok(())
}

#[test]
fn a_content_min_height_keeps_the_scroll_viewport() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 360.0, 640.0)?;
    let page = tree.column(8.0)?;
    tree.set_fill(page)?;
    let header = tree.sized(100.0, 40.0)?;
    let list = tree.stack()?;
    tree.set_fill(list)?;
    let scroll = tree.scroll()?;
    let body = tree.column(0.0)?;
    tree.set_min_size(body, 0.0, 5600.0)?;
    let sample = tree.text("Item 0", 16.0, [0, 0, 0, 255])?;
    tree.add_child(body, sample)?;
    tree.add_child(scroll, body)?;
    tree.add_child(list, scroll)?;
    tree.add_child(page, header)?;
    tree.add_child(page, list)?;
    tree.layout()?;
    let scroll_box = tree.rect(scroll)?;
    let body_box = tree.rect(body)?;
    assert!(
        scroll_box.h + 1.0 < body_box.h,
        "scroll {scroll_box:?} body {body_box:?}"
    );
    let sample_box = tree.rect(sample)?;
    let _ = tree.pointer(event(Phase::Down, sample_box.x + 1.0, sample_box.y + 1.0));
    let moved = tree.pointer(timed(
        Phase::Move,
        sample_box.x + 1.0,
        sample_box.y - 30.0,
        16.0,
    ));
    assert!(
        moved.iter().any(
            |gesture| matches!(gesture, Gesture::DragBy(id, _, dy) if *id == scroll && *dy < 0.0)
        ),
        "drag {moved:?} scroll {scroll_box:?} body {body_box:?}"
    );
    Ok(())
}

#[test]
fn scroll_rerecord_does_not_grow_with_content() -> Result<(), crate::Error> {
    for count in [8usize, 64] {
        let mut tree = Tree::new(MonoShaper::default());
        tree.set_viewport(0.0, 0.0, 40.0, 40.0)?;
        let scroll = tree.scroll()?;
        tree.set_max_size(scroll, 40.0, 40.0)?;
        let column = tree.column(0.0)?;
        for index in 0..count {
            let text = tree.text(format!("row{index}"), 16.0, [0, 0, 0, 255])?;
            tree.add_child(column, text)?;
        }
        tree.add_child(scroll, column)?;
        tree.layout()?;
        let _ = tree.record()?;
        assert!(tree.picture_writes() > count / 2, "count={count}");
        tree.set_scroll_offset(scroll, 0.0, 30.0)?;
        let _ = tree.record()?;
        assert_eq!(tree.picture_writes(), 0, "count={count}");
    }
    Ok(())
}

#[test]
fn editing_one_scroll_leaves_the_other_picture() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 80.0, 80.0)?;
    let root = tree.column(0.0)?;
    let first = tree.scroll()?;
    let second = tree.scroll()?;
    tree.set_max_size(first, 80.0, 40.0)?;
    tree.set_max_size(second, 80.0, 40.0)?;
    let left = tree.text("left", 16.0, [0, 0, 0, 255])?;
    let right = tree.text("right", 16.0, [0, 0, 0, 255])?;
    tree.add_child(first, left)?;
    tree.add_child(second, right)?;
    tree.add_child(root, first)?;
    tree.add_child(root, second)?;
    tree.layout()?;
    let _ = tree.record()?;
    let before = tree.layer_tree();
    let first_gen = picture_generation(&before, first);
    let second_gen = picture_generation(&before, second);
    tree.set_text(left, "changed")?;
    tree.layout()?;
    let _ = tree.record()?;
    let after = tree.layer_tree();
    assert_ne!(picture_generation(&after, first), first_gen);
    assert_eq!(picture_generation(&after, second), second_gen);
    assert_eq!(reachable(&after), after.layers.len());
    Ok(())
}

#[test]
fn scroll_keeps_the_same_picture_arc() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 40.0, 40.0)?;
    let scroll = tree.scroll()?;
    tree.set_max_size(scroll, 40.0, 40.0)?;
    let text = tree.text("row", 16.0, [0, 0, 0, 255])?;
    tree.add_child(scroll, text)?;
    tree.layout()?;
    let _ = tree.record()?;
    let first = tree.layer_tree();
    let kept = first
        .picture(crate::layer::child_picture_id(scroll))
        .expect("scroll picture");
    let generation = kept.generation;
    let commands = std::sync::Arc::clone(&kept.commands);
    tree.set_scroll_offset(scroll, 0.0, 12.0)?;
    let _ = tree.record()?;
    assert_eq!(tree.picture_writes(), 0);
    let second = tree.layer_tree();
    let again = second
        .picture(crate::layer::child_picture_id(scroll))
        .expect("scrolled picture");
    assert_eq!(again.generation, generation);
    assert!(std::sync::Arc::ptr_eq(&again.commands, &commands));
    assert_eq!(reachable(&second), second.layers.len());
    Ok(())
}

#[test]
fn a_filled_root_paints_before_its_scroll() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 40.0, 40.0)?;
    let root = tree.decorated([20, 20, 20, 255], 0.0)?;
    let scroll = tree.scroll()?;
    tree.set_max_size(scroll, 40.0, 20.0)?;
    let text = tree.text("row", 16.0, [0, 0, 0, 255])?;
    tree.add_child(scroll, text)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;
    let _ = tree.record()?;
    let layers = tree.layer_tree();
    let root_layer = layers.layers[layers.root.0 as usize].kind.clone();
    assert!(
        matches!(root_layer, protocol::LayerKind::Picture { .. }),
        "root fill is the first layer: {root_layer:?}"
    );
    assert!(
        layers
            .layers
            .iter()
            .any(|layer| matches!(layer.kind, protocol::LayerKind::Clip { .. }))
    );
    assert_eq!(reachable(&layers), layers.layers.len());
    Ok(())
}
