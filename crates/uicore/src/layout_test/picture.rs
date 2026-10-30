use super::*;

#[test]
fn recorded_text_uses_the_ancestor_origin() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let pad = tree.padding(8.0)?;
    let text = tree.text("Hi", 16.0, [1, 2, 3, 4])?;
    tree.add_child(pad, text)?;
    tree.layout()?;
    let list = tree.record()?;
    assert!(matches!(
        list.commands.first(),
        Some(protocol::Command::Text {
            x,
            y,
            utf8,
            color,
            font_size,
            max_width,
            ..
        }) if *x == 8.0
            && *y == 8.0
            && utf8.as_ref() == "Hi"
            && *color == [1, 2, 3, 4]
            && *font_size == 16.0
            && *max_width >= 16.0
    ));
    Ok(())
}

#[test]
fn decorated_paints_the_box_around_its_child() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let background = tree.decorated([9, 8, 7, 6], 0.0)?;
    let text = tree.text("Hi", 16.0, [1, 2, 3, 4])?;
    tree.add_child(background, text)?;
    tree.layout()?;
    let list = tree.record()?;
    assert!(matches!(
        list.commands.first(),
        Some(protocol::Command::Rect { w, h, color, corner_radius, .. })
            if *w == 16.0 && *h == 16.0 && *color == [9, 8, 7, 6] && *corner_radius == 0.0
    ));
    Ok(())
}

#[test]
fn remove_detaches_the_subtree_from_record_and_hit() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let column = tree.column(0.0)?;
    let keep = tree.text("Keep", 16.0, [0, 0, 0, 255])?;
    let region = tree.pointer_region()?;
    let label = tree.text("-", 16.0, [0, 0, 0, 255])?;
    tree.add_child(region, label)?;
    tree.add_child(column, keep)?;
    tree.add_child(column, region)?;
    tree.layout()?;
    let spot_x = tree.rect(label)?.x + 1.0;
    let spot_y = tree.rect(label)?.y + 1.0;
    assert!(texts(&mut tree)?.contains(&"-".to_string()));
    tree.remove(column, region)?;
    assert!(!texts(&mut tree)?.contains(&"-".to_string()));
    assert!(tree.pointer(event(Phase::Down, spot_x, spot_y)).is_empty());
    tree.layout()?;
    assert!(tree.pointer(event(Phase::Down, spot_x, spot_y)).is_empty());
    assert!(tree.pointer(event(Phase::Up, spot_x, spot_y)).is_empty());
    assert!(matches!(
        tree.remove(region, keep),
        Err(crate::Error::NotAChild)
    ));
    Ok(())
}

#[test]
fn empty_text_is_omitted_from_the_draw_list() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let text = tree.text("", 16.0, [0, 0, 0, 255])?;
    tree.layout()?;
    assert!(tree.record()?.commands.is_empty());
    tree.set_text(text, "Hi")?;
    tree.layout()?;
    assert_eq!(texts(&mut tree)?, vec!["Hi".to_string()]);
    Ok(())
}

#[test]
fn decorated_records_its_corner_radius() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let background = tree.decorated([1, 2, 3, 4], 6.0)?;
    let child = tree.sized(40.0, 20.0)?;
    tree.add_child(background, child)?;
    tree.layout()?;
    assert!(matches!(
        tree.record()?.commands.first(),
        Some(protocol::Command::Rect { corner_radius, .. }) if *corner_radius == 6.0
    ));
    Ok(())
}

#[test]
fn clip_wraps_an_image_and_nested_clips_stay_balanced() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let clip = tree.clip(4.0)?;
    let image = tree.image("asset:logo", 10.0, 12.0)?;
    tree.add_child(clip, image)?;
    tree.layout()?;
    assert!(matches!(
        tree.record()?.commands.as_slice(),
        [
            protocol::Command::PushClip {
                corner_radius,
                w,
                h,
                ..
            },
            protocol::Command::Image { source, .. },
            protocol::Command::Pop,
        ] if *corner_radius == 4.0 && *w == 10.0 && *h == 12.0 && source.as_ref() == "asset:logo"
    ));

    let mut tree = Tree::new(MonoShaper::default());
    let outer = tree.clip(3.0)?;
    let inner = tree.clip(1.0)?;
    let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
    tree.add_child(outer, inner)?;
    tree.add_child(inner, text)?;
    tree.layout()?;
    assert!(matches!(
        tree.record()?.commands.as_slice(),
        [
            protocol::Command::PushClip { corner_radius: outer_radius, .. },
            protocol::Command::PushClip { corner_radius: inner_radius, .. },
            protocol::Command::Text { utf8, .. },
            protocol::Command::Pop,
            protocol::Command::Pop,
        ] if *outer_radius == 3.0 && *inner_radius == 1.0 && utf8.as_ref() == "Hi"
    ));
    Ok(())
}

#[test]
fn opacity_wraps_text() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let fade = tree.opacity(128)?;
    let text = tree.text("Hi", 16.0, [1, 2, 3, 4])?;
    tree.add_child(fade, text)?;
    tree.layout()?;
    assert!(matches!(
        tree.record()?.commands.as_slice(),
        [
            protocol::Command::PushOpacity { alpha, .. },
            protocol::Command::Text { utf8, .. },
            protocol::Command::Pop,
        ] if *alpha == 128 && utf8.as_ref() == "Hi"
    ));
    Ok(())
}

#[test]
fn non_finite_gap_does_not_mark_the_tree() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let column = tree.column(0.0)?;
    tree.layout()?;
    assert!(matches!(
        tree.set_gap(column, f32::NAN),
        Err(crate::Error::BadNumber)
    ));
    assert!(!tree.is_dirty());
    Ok(())
}

#[test]
fn text_scale_multiplies_measurement_and_the_recorded_size() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
    tree.layout()?;
    let logical = tree.rect(text)?.w;
    tree.set_text_scale(2.0)?;
    tree.layout()?;
    assert!((tree.rect(text)?.w - logical * 2.0).abs() < 0.01);
    let list = tree.record()?;
    let size = list.commands.iter().find_map(|command| match command {
        protocol::Command::Text { font_size, .. } => Some(*font_size),
        _ => None,
    });
    assert_eq!(size, Some(32.0));
    Ok(())
}

#[test]
fn text_scale_grows_the_ancestor_boxes_of_the_button() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 360.0, 640.0)?;
    let root = tree.column(16.0)?;
    tree.set_fill(root)?;
    let pointer = tree.pointer_region()?;
    let decorated = tree.decorated([9, 9, 9, 255], 999.0)?;
    let pad = tree.padding(16.0)?;
    let row = tree.row(4.0)?;
    let text = tree.text("微任务改字", 14.0, [0, 0, 0, 255])?;
    tree.add_child(row, text)?;
    tree.add_child(pad, row)?;
    tree.add_child(decorated, pad)?;
    tree.add_child(pointer, decorated)?;
    tree.add_child(root, pointer)?;
    tree.set_min_size(decorated, 0.0, 44.0)?;
    tree.layout()?;
    let before = tree.rect(decorated)?;
    assert_eq!((before.w, before.h), (67.0, 46.0));
    tree.set_text_scale(2.0)?;
    tree.layout()?;
    let after = tree.rect(decorated)?;
    // The wrapper boxes grow with the scaled label. An orphan subtree must not
    // become a relayout boundary, or Taffy's new sizes never reach the ancestors.
    assert_eq!((after.w, after.h), (102.0, 60.0));
    let text_rect = tree.rect(text)?;
    assert_eq!((text_rect.w, text_rect.h), (70.0, 28.0));
    Ok(())
}

#[test]
fn transform_wraps_children_and_hit_uses_the_inverse() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let clip = tree.clip(0.0)?;
    let shifted = tree.transform()?;
    let text = tree.text("Hi", 16.0, [1, 2, 3, 4])?;
    tree.add_child(shifted, text)?;
    tree.add_child(clip, shifted)?;
    tree.layout()?;
    let before = tree.rect(text)?;
    tree.set_transform(shifted, 40.0, 0.0, 0.0, 2.0)?;
    assert!(tree.needs_paint());
    assert!(!tree.needs_layout());
    assert_eq!(tree.rect(text)?, before);
    let list = tree.record()?;
    assert!(matches!(
        list.commands.as_slice(),
        [
            protocol::Command::PushClip { .. },
            protocol::Command::PushTransform {
                translate_x,
                translate_y,
                rotate_rad,
                scale_x,
                scale_y,
            },
            protocol::Command::Text { utf8, x, y, .. },
            protocol::Command::Pop,
            protocol::Command::Pop,
        ] if *translate_x == 40.0
            && *translate_y == 0.0
            && *rotate_rad == 0.0
            && *scale_x == 2.0
            && *scale_y == 2.0
            && utf8.as_ref() == "Hi"
            && *x == before.x
            && *y == before.y
    ));
    assert!(matches!(
        tree.set_transform(text, 0.0, 0.0, 0.0, 1.0),
        Err(crate::Error::WrongKind)
    ));

    let mut tree = Tree::new(MonoShaper::default());
    let shifted = tree.transform()?;
    let region = tree.pointer_region()?;
    let target = tree.sized(16.0, 16.0)?;
    tree.add_child(region, target)?;
    tree.add_child(shifted, region)?;
    tree.set_transform(shifted, 40.0, 0.0, 0.0, 2.0)?;
    tree.layout()?;
    let spot = tree.rect(target)?;
    let visual_x = spot.x * 2.0 + 40.0;
    let visual_y = spot.y * 2.0;
    assert!(
        tree.pointer(event(Phase::Down, spot.x + 1.0, spot.y + 1.0))
            .is_empty(),
        "the untransformed layout point is not the painted point"
    );
    assert_eq!(
        tree.pointer(event(Phase::Down, visual_x + 1.0, visual_y + 1.0)),
        vec![Gesture::Press(region)]
    );
    Ok(())
}

#[test]
fn non_finite_margin_does_not_mark_the_tree() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let column = tree.column(0.0)?;
    tree.layout()?;
    assert!(matches!(
        tree.set_margin(column, Edges::all(f32::NAN)),
        Err(crate::Error::BadNumber)
    ));
    assert!(!tree.is_dirty());
    assert!(matches!(
        tree.set_shrink(column, -1.0),
        Err(crate::Error::BadNumber)
    ));
    assert!(!tree.is_dirty());
    assert!(matches!(
        tree.set_viewport(f32::NAN, 0.0, 10.0, 10.0),
        Err(crate::Error::BadNumber)
    ));
    assert!(!tree.is_dirty());
    Ok(())
}

#[test]
fn a_rounded_clip_misses_its_corner() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let clip = tree.clip(8.0)?;
    let region = tree.pointer_region()?;
    let child = tree.sized(20.0, 20.0)?;
    tree.add_child(region, child)?;
    tree.add_child(clip, region)?;
    tree.layout()?;
    assert!(
        tree.pointer(event(Phase::Down, 1.0, 1.0)).is_empty(),
        "the corner outside the radius is clipped"
    );
    assert_eq!(
        tree.pointer(event(Phase::Down, 10.0, 10.0)),
        vec![Gesture::Press(region)]
    );
    Ok(())
}

#[test]
fn discard_drops_the_picture_cache() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 40.0, 40.0)?;
    let scroll = tree.scroll()?;
    tree.set_max_size(scroll, 40.0, 40.0)?;
    let text = tree.text("row", 16.0, [0, 0, 0, 255])?;
    tree.add_child(scroll, text)?;
    tree.layout()?;
    let _ = tree.record()?;
    assert!(tree.picture_cache_len() > 0);
    tree.remove(scroll, text)?;
    assert!(tree.picture_cache_len() > 0);
    tree.discard(text)?;
    tree.discard(scroll)?;
    assert_eq!(tree.picture_cache_len(), 0);
    Ok(())
}

#[test]
fn a_wrapped_tab_stays_inside_the_viewport() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 200.0, 400.0)?;
    let column = tree.column(8.0)?;
    tree.set_fill(column)?;
    let row = tree.row(8.0)?;
    tree.set_axis_size(row, AxisSize::Fraction(1.0), AxisSize::Auto)?;
    tree.set_wrap(row, true)?;
    let mut buttons = Vec::new();
    for _ in 0..3 {
        let button = tree.pointer_region()?;
        tree.set_min_size(button, 80.0, 44.0)?;
        tree.add_child(row, button)?;
        buttons.push(button);
    }
    tree.add_child(column, row)?;
    tree.layout()?;
    let list = buttons[2];
    let list_box = tree.rect(list)?;
    assert!(
        list_box.x >= 0.0 && list_box.x + list_box.w <= 200.5 && list_box.y >= 44.0,
        "third tab should wrap onto the next line: {list_box:?}"
    );
    let hit = tree.pointer(event(
        Phase::Down,
        list_box.x + list_box.w * 0.5,
        list_box.y + list_box.h * 0.5,
    ));
    assert!(
        hit.iter()
            .any(|gesture| matches!(gesture, Gesture::Press(id) if *id == list)),
        "wrapped tab was not hit: {hit:?} {list_box:?}"
    );
    Ok(())
}

#[test]
fn flat_and_layer_trees_agree() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 200.0, 200.0)?;
    let root = tree.column(0.0)?;
    let chrome = tree.decorated([20, 20, 20, 255], 0.0)?;
    let label = tree.text("chrome", 16.0, [0, 0, 0, 255])?;
    tree.add_child(chrome, label)?;
    tree.add_child(root, chrome)?;
    let fade = tree.opacity(200)?;
    let faded = tree.text("fade", 16.0, [0, 0, 0, 255])?;
    tree.add_child(fade, faded)?;
    tree.add_child(root, fade)?;
    let frame = tree.clip(4.0)?;
    let clipped = tree.text("clip", 16.0, [0, 0, 0, 255])?;
    tree.add_child(frame, clipped)?;
    tree.add_child(root, frame)?;
    let shifted = tree.transform()?;
    tree.set_transform(shifted, 50.0, 0.0, 0.0, 1.0)?;
    let moved = tree.text("move", 16.0, [0, 0, 0, 255])?;
    tree.add_child(shifted, moved)?;
    tree.add_child(root, shifted)?;
    let scroll = tree.scroll()?;
    tree.set_max_size(scroll, 180.0, 40.0)?;
    let row = tree.text("row", 16.0, [0, 0, 0, 255])?;
    let image = tree.image("asset:logo", 12.0, 8.0)?;
    tree.add_child(scroll, row)?;
    tree.add_child(scroll, image)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;
    agree(&mut tree)?;

    tree.set_scroll_offset(scroll, 0.0, 8.0)?;
    assert_eq!(record_writes(&mut tree)?, 0);
    agree(&mut tree)?;

    tree.set_text(label, "CHROME")?;
    tree.layout()?;
    agree(&mut tree)?;

    tree.set_alpha(fade, 128)?;
    assert_eq!(record_writes(&mut tree)?, 0);
    agree(&mut tree)?;

    tree.set_transform(shifted, 50.0, 10.0, 0.0, 1.0)?;
    assert_eq!(record_writes(&mut tree)?, 0);
    agree(&mut tree)?;

    let mut shifted_origin = Tree::new(MonoShaper::default());
    shifted_origin.set_viewport(12.0, 30.0, 80.0, 40.0)?;
    let origin_text = shifted_origin.text("origin", 16.0, [0, 0, 0, 255])?;
    shifted_origin.layout()?;
    agree(&mut shifted_origin)?;
    let list = shifted_origin.record()?;
    let spot = list.commands.iter().find_map(|command| match command {
        protocol::Command::Text { x, y, utf8, .. } if utf8.as_ref() == "origin" => Some((*x, *y)),
        _ => None,
    });
    let rect = shifted_origin.rect(origin_text)?;
    assert_eq!(spot, Some((rect.x, rect.y)));
    Ok(())
}

#[test]
fn tab_bar_merges_into_one_run() -> Result<(), crate::Error> {
    let mut counts = Vec::new();
    for buttons in [4usize, 12] {
        let mut tree = Tree::new(MonoShaper::default());
        tree.set_viewport(0.0, 0.0, 360.0, 640.0)?;
        let root = tree.column(0.0)?;
        for index in 0..buttons {
            let button = tree.decorated([index as u8, 0, 0, 255], 0.0)?;
            let label = tree.text(format!("tab{index}"), 16.0, [0, 0, 0, 255])?;
            tree.add_child(button, label)?;
            tree.add_child(root, button)?;
        }
        let scroll = tree.scroll()?;
        tree.set_max_size(scroll, 360.0, 400.0)?;
        let row = tree.text("row", 16.0, [0, 0, 0, 255])?;
        tree.add_child(scroll, row)?;
        tree.add_child(root, scroll)?;
        tree.layout()?;
        let _ = tree.record()?;
        assert_eq!(tree.picture_cache_len(), 2, "buttons={buttons}");
        counts.push(tree.layer_tree().layers.len());
    }
    assert_eq!(counts[0], counts[1]);
    Ok(())
}

#[test]
fn editing_text_rewrites_only_the_owner_run() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 80.0, 80.0)?;
    let root = tree.column(0.0)?;
    let label = tree.text("chrome", 16.0, [0, 0, 0, 255])?;
    let scroll = tree.scroll()?;
    tree.set_max_size(scroll, 80.0, 40.0)?;
    let row = tree.text("row", 16.0, [0, 0, 0, 255])?;
    tree.add_child(scroll, row)?;
    tree.add_child(root, label)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;
    let _ = tree.record()?;
    let before = tree.layer_tree();
    let kept = before
        .picture(crate::layer::child_picture_id(scroll))
        .expect("scroll picture");
    let generation = kept.generation;
    let commands = std::sync::Arc::clone(&kept.commands);
    tree.set_text(label, "edited")?;
    tree.layout()?;
    let _ = tree.record()?;
    let after = tree.layer_tree();
    let chrome = after
        .picture(crate::layer::self_picture_id(label))
        .expect("chrome run");
    assert_eq!(tree.picture_writes(), chrome.commands.len());
    let again = after
        .picture(crate::layer::child_picture_id(scroll))
        .expect("scroll picture");
    assert_eq!(again.generation, generation);
    assert!(std::sync::Arc::ptr_eq(&again.commands, &commands));
    let _ = tree.record()?;
    assert_eq!(tree.picture_writes(), 0);
    Ok(())
}

#[test]
fn settled_clip_is_a_command() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let clip = tree.clip(4.0)?;
    let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
    tree.add_child(clip, text)?;
    tree.layout()?;
    let _ = tree.record()?;
    let layers = tree.layer_tree();
    assert!(
        layers
            .layers
            .iter()
            .all(|layer| !matches!(layer.kind, protocol::LayerKind::Clip { .. }))
    );
    assert_eq!(tree.picture_cache_len(), 1);
    let picture = layers.pictures.iter().next().expect("picture");
    assert!(
        picture
            .commands
            .iter()
            .any(|command| matches!(command, protocol::Command::PushClip { .. }))
    );
    Ok(())
}

#[test]
fn clip_promotes_over_compositing_subtree() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 80.0, 80.0)?;
    let root = tree.column(0.0)?;
    let stay = tree.text("stay", 16.0, [0, 0, 0, 255])?;
    let clip = tree.clip(2.0)?;
    let fade = tree.opacity(128)?;
    let body = tree.text("body", 16.0, [0, 0, 0, 255])?;
    tree.add_child(fade, body)?;
    tree.add_child(clip, fade)?;
    tree.add_child(root, stay)?;
    tree.add_child(root, clip)?;
    tree.layout()?;
    let _ = tree.record()?;
    let before = tree.layer_tree();
    assert!(
        before
            .layers
            .iter()
            .any(|layer| matches!(layer.kind, protocol::LayerKind::Clip { .. }))
    );
    assert!(
        before
            .layers
            .iter()
            .any(|layer| matches!(layer.kind, protocol::LayerKind::Opacity { .. }))
    );
    let root_generation = before
        .picture(crate::layer::self_picture_id(stay))
        .expect("root run")
        .generation;
    tree.set_text(body, "BODY")?;
    tree.layout()?;
    let _ = tree.record()?;
    let after = tree.layer_tree();
    assert_eq!(
        after
            .picture(crate::layer::self_picture_id(stay))
            .expect("root run")
            .generation,
        root_generation
    );
    let rewritten = after
        .picture(crate::layer::self_picture_id(body))
        .expect("opacity run");
    assert_eq!(tree.picture_writes(), rewritten.commands.len());
    Ok(())
}

#[test]
fn translate_transform_matches_flat() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let shifted = tree.transform()?;
    let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
    tree.add_child(shifted, text)?;
    tree.set_transform(shifted, 50.0, 0.0, 0.0, 1.0)?;
    tree.layout()?;
    let list = tree.record()?;
    agree_list(&tree, &list)?;
    assert!(list.commands.iter().any(|command| matches!(
        command,
        protocol::Command::PushTransform {
            translate_x,
            translate_y,
            rotate_rad,
            scale_x,
            scale_y,
            ..
        } if *translate_x == 50.0
            && *translate_y == 0.0
            && *rotate_rad == 0.0
            && *scale_x == 1.0
            && *scale_y == 1.0
    )));
    let layers = tree.layer_tree();
    assert!(
        layers
            .layers
            .iter()
            .any(|layer| matches!(layer.kind, protocol::LayerKind::Transform { .. }))
    );
    let picture = layers.pictures.iter().next().expect("picture");
    let local = picture.commands.iter().find_map(|command| match command {
        protocol::Command::Text { x, .. } => Some(*x),
        _ => None,
    });
    assert_eq!(local, Some(0.0));
    Ok(())
}
