use crate::{Gesture, MonoShaper, Phase, PointerEvent, Shaper, Tree};

fn event(phase: Phase, x: f32, y: f32) -> PointerEvent {
    PointerEvent {
        phase,
        x,
        y,
        time_ms: 0.0,
    }
}

fn timed(phase: Phase, x: f32, y: f32, time_ms: f64) -> PointerEvent {
    PointerEvent {
        phase,
        x,
        y,
        time_ms,
    }
}

#[test]
fn column_places_the_second_child_after_the_gap() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let column = tree.column(10.0)?;
    let top = tree.sized(20.0, 20.0)?;
    let bottom = tree.sized(20.0, 20.0)?;
    tree.add_child(column, top)?;
    tree.add_child(column, bottom)?;
    tree.layout()?;
    assert_eq!(tree.rect(bottom)?.y, 30.0);
    Ok(())
}

#[test]
fn padding_offsets_the_child_and_grows_the_parent() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let pad = tree.padding(8.0)?;
    let child = tree.sized(10.0, 12.0)?;
    tree.add_child(pad, child)?;
    tree.layout()?;
    let child_rect = tree.rect(child)?;
    let parent_rect = tree.rect(pad)?;
    assert_eq!((child_rect.x, child_rect.y), (8.0, 8.0));
    assert_eq!((parent_rect.w, parent_rect.h), (26.0, 28.0));
    Ok(())
}

#[test]
fn grid_puts_two_children_in_two_columns() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let grid = tree.grid(2)?;
    let left = tree.sized(30.0, 10.0)?;
    let right = tree.sized(14.0, 10.0)?;
    tree.add_child(grid, left)?;
    tree.add_child(grid, right)?;
    tree.layout()?;
    let left_rect = tree.rect(left)?;
    let right_rect = tree.rect(right)?;
    assert_eq!(left_rect.x, 0.0);
    assert_eq!(right_rect.x, left_rect.w);
    assert_eq!(right_rect.y, left_rect.y);
    Ok(())
}

#[test]
fn mono_shaper_measures_hi_at_font_16() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let text = tree.text("Hi", 16.0, [255, 255, 255, 255])?;
    tree.layout()?;
    let rect = tree.rect(text)?;
    assert_eq!((rect.w, rect.h), (16.0, 16.0));
    Ok(())
}

#[test]
fn click_fires_once_inside_and_not_after_leaving() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
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
fn recorded_text_uses_the_ancestor_origin() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
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
            && utf8 == "Hi"
            && *color == [1, 2, 3, 4]
            && *font_size == 16.0
            && *max_width >= 16.0
    ));
    Ok(())
}

#[test]
fn viewport_wraps_text_to_the_available_width() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    tree.set_viewport(0.0, 0.0, 10.0, 40.0);
    let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
    tree.layout()?;
    assert_eq!((tree.rect(text)?.w, tree.rect(text)?.h), (8.0, 32.0));
    let list = tree.record()?;
    assert!(matches!(
        list.commands.first(),
        Some(protocol::Command::Text { max_width, .. }) if *max_width == 10.0
    ));
    Ok(())
}

#[test]
fn decorated_paints_the_box_around_its_child() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
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
fn set_text_keeps_the_hit_box_until_the_next_layout() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let region = tree.pointer_region()?;
    let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
    tree.add_child(region, text)?;
    tree.layout()?;
    assert!(!tree.is_dirty());
    let before = tree.rect(text)?;
    assert_eq!(
        tree.pointer(event(Phase::Down, 1.0, 1.0)),
        vec![Gesture::Press(region)]
    );
    // A node created off to the side is not in the laid-out tree.
    let _detached = tree.text("extra", 16.0, [0, 0, 0, 255])?;
    assert!(!tree.is_dirty());
    tree.set_text(text, "Hello")?;
    assert!(tree.is_dirty());
    assert_eq!(tree.rect(text)?, before);
    assert_eq!(
        tree.pointer(event(Phase::Up, 1.0, 1.0)),
        vec![Gesture::Release(region), Gesture::Click(region)]
    );
    tree.layout()?;
    assert!(!tree.is_dirty());
    let list = tree.record()?;
    assert!(matches!(
        list.commands.first(),
        Some(protocol::Command::Text { utf8, .. }) if utf8 == "Hello"
    ));
    Ok(())
}

#[test]
fn insert_before_places_the_child_in_order() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let column = tree.column(0.0)?;
    let top = tree.sized(10.0, 10.0)?;
    let middle = tree.sized(10.0, 10.0)?;
    let bottom = tree.sized(10.0, 10.0)?;
    tree.add_child(column, top)?;
    tree.add_child(column, bottom)?;
    tree.insert(column, middle, Some(bottom))?;
    tree.layout()?;
    assert_eq!(tree.rect(top)?.y, 0.0);
    assert_eq!(tree.rect(middle)?.y, 10.0);
    assert_eq!(tree.rect(bottom)?.y, 20.0);
    Ok(())
}

#[test]
fn remove_detaches_the_subtree_from_record_and_hit() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
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
    assert!(texts(&tree)?.contains(&"-".to_string()));
    tree.remove(column, region)?;
    assert!(!texts(&tree)?.contains(&"-".to_string()));
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
fn stale_id_after_remove_is_unknown_node() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let column = tree.column(0.0)?;
    let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
    tree.add_child(column, text)?;
    tree.remove(column, text)?;
    // The detached node keeps its id so it can be inserted again.
    tree.set_text(text, "Yo")?;
    let stale = crate::NodeId::from_raw(text.to_raw() + (1 << 32));
    assert!(matches!(
        tree.set_text(stale, "no"),
        Err(crate::Error::UnknownNode)
    ));
    Ok(())
}

#[test]
fn insert_moves_a_child_between_parents() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let root = tree.column(0.0)?;
    let left = tree.column(0.0)?;
    let spacer = tree.sized(10.0, 40.0)?;
    let right = tree.column(0.0)?;
    let child = tree.sized(10.0, 10.0)?;
    tree.add_child(root, left)?;
    tree.add_child(root, spacer)?;
    tree.add_child(root, right)?;
    tree.add_child(left, child)?;
    tree.layout()?;
    assert_eq!(tree.rect(child)?.y, 0.0);
    tree.insert(right, child, None)?;
    tree.layout()?;
    assert_eq!(tree.parent(child)?, Some(right));
    assert_eq!(tree.first_child(left)?, None);
    assert_eq!(tree.rect(child)?.y, 40.0);
    Ok(())
}

#[test]
fn discard_retires_a_detached_subtree() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let column = tree.column(0.0)?;
    let region = tree.pointer_region()?;
    let label = tree.text("Go", 16.0, [0, 0, 0, 255])?;
    tree.add_child(region, label)?;
    tree.add_child(column, region)?;
    assert!(matches!(tree.discard(region), Err(crate::Error::NotAChild)));
    tree.remove(column, region)?;
    let retired = tree.discard(region)?;
    assert!(retired.gestures.is_empty());
    assert_eq!(retired.ids, vec![label, region]);
    assert!(matches!(
        tree.set_text(label, "no"),
        Err(crate::Error::UnknownNode)
    ));
    assert!(matches!(
        tree.insert(column, region, None),
        Err(crate::Error::UnknownNode)
    ));
    Ok(())
}

#[test]
fn removed_subtree_can_be_inserted_again() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let column = tree.column(0.0)?;
    let text = tree.text("Hi", 16.0, [1, 2, 3, 4])?;
    tree.add_child(column, text)?;
    tree.layout()?;
    tree.remove(column, text)?;
    assert!(!texts(&tree)?.contains(&"Hi".to_string()));
    tree.insert(column, text, None)?;
    tree.layout()?;
    assert_eq!(tree.rect(text)?.w, 16.0);
    assert!(texts(&tree)?.contains(&"Hi".to_string()));
    Ok(())
}

#[test]
fn setters_relayout_on_the_next_frame() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let column = tree.column(0.0)?;
    let top = tree.sized(10.0, 10.0)?;
    let bottom = tree.sized(10.0, 10.0)?;
    tree.add_child(column, top)?;
    tree.add_child(column, bottom)?;
    tree.layout()?;
    let y = tree.rect(bottom)?.y;
    tree.set_gap(column, 6.0)?;
    assert!(tree.is_dirty());
    assert_eq!(tree.rect(bottom)?.y, y);
    tree.layout()?;
    assert!(!tree.is_dirty());
    assert_eq!(tree.rect(bottom)?.y, 16.0);
    assert!(matches!(
        tree.set_font_size(column, 12.0),
        Err(crate::Error::NotText)
    ));
    assert!(matches!(
        tree.set_padding(column, 1.0),
        Err(crate::Error::WrongKind)
    ));

    let mut tree = Tree::new(MonoShaper);
    let pad = tree.padding(0.0)?;
    let child = tree.sized(10.0, 12.0)?;
    tree.add_child(pad, child)?;
    tree.layout()?;
    tree.set_padding(pad, 3.0)?;
    assert!(tree.is_dirty());
    assert_eq!(tree.rect(child)?.x, 0.0);
    tree.layout()?;
    assert_eq!((tree.rect(child)?.x, tree.rect(child)?.y), (3.0, 3.0));

    let mut tree = Tree::new(MonoShaper);
    let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
    tree.layout()?;
    tree.set_font_size(text, 32.0)?;
    assert!(tree.is_dirty());
    assert_eq!(tree.rect(text)?.h, 16.0);
    tree.layout()?;
    assert_eq!((tree.rect(text)?.w, tree.rect(text)?.h), (32.0, 32.0));
    tree.set_color(text, [9, 8, 7, 6])?;
    tree.layout()?;
    assert!(matches!(
        tree.record()?.commands.first(),
        Some(protocol::Command::Text { color, .. }) if *color == [9, 8, 7, 6]
    ));

    let mut tree = Tree::new(MonoShaper);
    let background = tree.decorated([1, 2, 3, 4], 0.0)?;
    let box_ = tree.sized(8.0, 8.0)?;
    tree.add_child(background, box_)?;
    tree.layout()?;
    tree.set_color(background, [9, 8, 7, 6])?;
    assert!(tree.is_dirty());
    assert_eq!(tree.rect(box_)?.w, 8.0);
    tree.layout()?;
    assert!(matches!(
        tree.record()?.commands.first(),
        Some(protocol::Command::Rect { color, .. }) if *color == [9, 8, 7, 6]
    ));
    Ok(())
}

#[test]
fn queries_walk_the_tree() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let column = tree.column(0.0)?;
    let first = tree.sized(1.0, 1.0)?;
    let second = tree.sized(1.0, 1.0)?;
    tree.add_child(column, first)?;
    tree.add_child(column, second)?;
    assert_eq!(tree.parent(column)?, None);
    assert_eq!(tree.parent(first)?, Some(column));
    assert_eq!(tree.first_child(column)?, Some(first));
    assert_eq!(tree.first_child(first)?, None);
    assert_eq!(tree.next_sibling(first)?, Some(second));
    assert_eq!(tree.next_sibling(second)?, None);
    assert!(!tree.is_text(column)?);
    let label = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
    assert!(tree.is_text(label)?);
    Ok(())
}

#[test]
fn empty_text_is_omitted_from_the_draw_list() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let text = tree.text("", 16.0, [0, 0, 0, 255])?;
    tree.layout()?;
    assert!(tree.record()?.commands.is_empty());
    tree.set_text(text, "Hi")?;
    tree.layout()?;
    assert_eq!(texts(&tree)?, vec!["Hi".to_string()]);
    Ok(())
}

#[test]
fn stack_places_children_at_the_same_origin() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let stack = tree.stack()?;
    let back = tree.sized(30.0, 20.0)?;
    let front = tree.sized(10.0, 8.0)?;
    tree.add_child(stack, back)?;
    tree.add_child(stack, front)?;
    tree.layout()?;
    assert_eq!(
        (tree.rect(back)?.x, tree.rect(back)?.y),
        (tree.rect(front)?.x, tree.rect(front)?.y)
    );
    assert_eq!((tree.rect(stack)?.w, tree.rect(stack)?.h), (30.0, 20.0));
    Ok(())
}

#[test]
fn decorated_records_its_corner_radius() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
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
    let mut tree = Tree::new(MonoShaper);
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
        ] if *corner_radius == 4.0 && *w == 10.0 && *h == 12.0 && source == "asset:logo"
    ));

    let mut tree = Tree::new(MonoShaper);
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
        ] if *outer_radius == 3.0 && *inner_radius == 1.0 && utf8 == "Hi"
    ));
    Ok(())
}

#[test]
fn opacity_wraps_text() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
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
        ] if *alpha == 128 && utf8 == "Hi"
    ));
    Ok(())
}

#[test]
fn click_outside_a_clip_misses_the_child() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
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
fn grow_fills_free_space_on_the_next_layout() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let row = tree.sized(100.0, 40.0)?;
    let child = tree.sized(20.0, 10.0)?;
    let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
    tree.add_child(row, child)?;
    tree.add_child(row, text)?;
    tree.layout()?;
    let before = tree.rect(text)?;
    assert!(matches!(
        tree.set_grow(child, -1.0),
        Err(crate::Error::NegativeGrow)
    ));
    assert!(!tree.is_dirty());
    tree.set_grow(child, 1.0)?;
    assert!(tree.is_dirty());
    tree.set_text(text, "Hello")?;
    assert_eq!(tree.rect(text)?, before);
    tree.layout()?;
    assert!(tree.rect(row)?.w >= 100.0);
    assert!(tree.rect(child)?.w > 20.0);
    Ok(())
}

#[test]
fn viewport_origin_shifts_hit_testing() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    tree.set_viewport(12.0, 30.0, 80.0, 100.0);
    let region = tree.pointer_region()?;
    let child = tree.sized(20.0, 10.0)?;
    tree.add_child(region, child)?;
    tree.layout()?;
    assert_eq!((tree.rect(region)?.x, tree.rect(region)?.y), (12.0, 30.0));
    assert_eq!(
        tree.pointer(event(Phase::Down, 14.0, 32.0)),
        vec![Gesture::Press(region)]
    );
    assert!(tree.pointer(event(Phase::Cancel, 14.0, 32.0)) == vec![Gesture::Release(region)]);
    assert!(tree.pointer(event(Phase::Down, 1.0, 1.0)).is_empty());
    Ok(())
}

#[test]
fn structure_change_drops_a_press() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
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
    assert!(tree.pointer(event(Phase::Up, 1.0, 1.0)).is_empty());
    Ok(())
}

#[test]
fn setters_update_paint_without_moving_the_previous_box() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let clip = tree.clip(1.0)?;
    let child = tree.sized(20.0, 10.0)?;
    tree.add_child(clip, child)?;
    tree.layout()?;
    let before = tree.rect(child)?;
    tree.set_radius(clip, 6.0)?;
    assert!(tree.is_dirty());
    assert_eq!(tree.rect(child)?.x, before.x);
    let list = tree.record()?;
    assert!(matches!(
        list.commands.first(),
        Some(protocol::Command::PushClip { corner_radius, .. }) if *corner_radius == 6.0
    ));
    assert!(matches!(
        tree.set_radius(child, 1.0),
        Err(crate::Error::WrongKind)
    ));

    let mut tree = Tree::new(MonoShaper);
    let fade = tree.opacity(255)?;
    let box_ = tree.sized(8.0, 8.0)?;
    tree.add_child(fade, box_)?;
    tree.layout()?;
    let width = tree.rect(box_)?.w;
    tree.set_alpha(fade, 40)?;
    assert!(tree.is_dirty());
    assert_eq!(tree.rect(box_)?.w, width);
    let list = tree.record()?;
    assert!(matches!(
        list.commands.first(),
        Some(protocol::Command::PushOpacity { alpha, .. }) if *alpha == 40
    ));
    assert!(matches!(
        tree.set_alpha(box_, 1),
        Err(crate::Error::WrongKind)
    ));

    let mut tree = Tree::new(MonoShaper);
    let frame = tree.align(crate::Alignment::Start)?;
    let inner = tree.sized(4.0, 4.0)?;
    tree.add_child(frame, inner)?;
    tree.layout()?;
    tree.set_align(frame, crate::Alignment::End)?;
    assert!(tree.is_dirty());
    tree.layout()?;
    assert!(!tree.is_dirty());
    assert!(matches!(
        tree.set_align(inner, crate::Alignment::Center),
        Err(crate::Error::WrongKind)
    ));

    let mut tree = Tree::new(MonoShaper);
    let box_ = tree.sized(10.0, 12.0)?;
    tree.layout()?;
    tree.set_size(box_, 30.0, 8.0)?;
    assert!(tree.is_dirty());
    assert_eq!(tree.rect(box_)?.w, 10.0);
    tree.layout()?;
    assert_eq!((tree.rect(box_)?.w, tree.rect(box_)?.h), (30.0, 8.0));
    let label = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
    assert!(matches!(
        tree.set_size(label, 1.0, 1.0),
        Err(crate::Error::WrongKind)
    ));

    let mut tree = Tree::new(MonoShaper);
    let image = tree.image("asset:logo", 10.0, 10.0)?;
    tree.layout()?;
    let hit = tree.rect(image)?;
    tree.set_image(image, "asset:mark", 24.0, 18.0)?;
    assert!(tree.is_dirty());
    assert_eq!((tree.rect(image)?.w, tree.rect(image)?.h), (hit.w, hit.h));
    tree.layout()?;
    assert_eq!((tree.rect(image)?.w, tree.rect(image)?.h), (24.0, 18.0));
    let list = tree.record()?;
    assert!(matches!(
        list.commands.first(),
        Some(protocol::Command::Image { source, .. }) if source == "asset:mark"
    ));
    let plain = tree.sized(1.0, 1.0)?;
    assert!(matches!(
        tree.set_image(plain, "asset:x", 1.0, 1.0),
        Err(crate::Error::WrongKind)
    ));
    Ok(())
}

#[test]
fn non_finite_gap_does_not_mark_the_tree() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
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
fn mono_shaper_counts_utf8_bytes() {
    let size = MonoShaper.measure(
        "你",
        10.0,
        1_000_000.0,
        [0, 0, 0, 255],
        &protocol::TextStyle::default(),
    );
    assert_eq!(
        size,
        crate::TextSize {
            width: 15.0,
            height: 10.0
        }
    );
}

#[test]
fn a_move_inside_the_slop_still_clicks() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
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
    let mut tree = Tree::new(MonoShaper);
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
fn passing_the_slop_over_a_scroll_locks_a_drag() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let scroll = tree.scroll()?;
    let region = tree.pointer_region()?;
    let child = tree.sized(80.0, 80.0)?;
    tree.add_child(region, child)?;
    tree.add_child(scroll, region)?;
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
    let mut tree = Tree::new(MonoShaper);
    let scroll = tree.scroll()?;
    let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
    tree.add_child(scroll, text)?;
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
    let mut tree = Tree::new(MonoShaper);
    let scroll = tree.scroll()?;
    let region = tree.pointer_region()?;
    let child = tree.sized(40.0, 40.0)?;
    tree.add_child(region, child)?;
    tree.add_child(scroll, region)?;
    tree.layout()?;
    let _ = tree.pointer(event(Phase::Down, 4.0, 4.0));
    let _ = tree.pointer(event(Phase::Move, 4.0, 20.0));
    let extra = tree.sized(4.0, 4.0)?;
    tree.add_child(scroll, extra)?;
    tree.layout()?;
    assert!(tree.pointer(event(Phase::Up, 4.0, 20.0)).is_empty());
    Ok(())
}

#[test]
fn scroll_offset_moves_paint_and_hits_without_moving_layout() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    tree.set_viewport(0.0, 0.0, 100.0, 80.0);
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

    tree.set_scroll_offset(scroll, 0.0, 30.0)?;
    assert!(tree.is_dirty());
    assert_eq!(tree.scroll_offset(scroll)?, (0.0, 30.0));
    tree.layout()?;
    assert_eq!(tree.rect(header)?, header_box);
    assert_eq!(tree.rect(label)?, label_box);

    let list = tree.record()?;
    let text = list.commands.iter().find_map(|command| match command {
        protocol::Command::Text { utf8, x, y, .. } if utf8 == "B" => Some((*x, *y)),
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
    let mut tree = Tree::new(MonoShaper);
    tree.set_viewport(0.0, 0.0, 100.0, 80.0);
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

    let mut tree = Tree::new(MonoShaper);
    tree.set_viewport(0.0, 0.0, 100.0, 80.0);
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
    let mut tree = Tree::new(MonoShaper);
    tree.set_viewport(0.0, 0.0, 320.0, 180.0);
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
fn justify_and_wrap_use_the_flex_line() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    tree.set_viewport(0.0, 0.0, 100.0, 40.0);
    let row = tree.row(0.0)?;
    tree.set_fill(row)?;
    tree.set_justify(row, crate::Justify::SpaceBetween)?;
    let left = tree.sized(20.0, 10.0)?;
    let right = tree.sized(20.0, 10.0)?;
    tree.add_child(row, left)?;
    tree.add_child(row, right)?;
    tree.layout()?;
    assert_eq!(tree.rect(left)?.x, 0.0);
    let right_x = tree.rect(right)?.x;
    assert!(
        (right_x - 80.0).abs() < 0.5,
        "space-between right edge landed at {right_x}"
    );
    assert!(matches!(
        tree.set_justify(left, crate::Justify::Center),
        Err(crate::Error::WrongKind)
    ));

    let mut tree = Tree::new(MonoShaper);
    tree.set_viewport(0.0, 0.0, 50.0, 80.0);
    let row = tree.row(0.0)?;
    tree.set_fill(row)?;
    tree.set_wrap(row, true)?;
    let first = tree.sized(30.0, 10.0)?;
    let second = tree.sized(30.0, 10.0)?;
    tree.add_child(row, first)?;
    tree.add_child(row, second)?;
    tree.layout()?;
    assert_eq!(tree.rect(first)?.y, 0.0);
    assert!(
        tree.rect(second)?.y >= 10.0,
        "wrapped child y is {}",
        tree.rect(second)?.y
    );
    Ok(())
}

#[test]
fn transform_wraps_children_and_hit_ignores_it() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let clip = tree.clip(0.0)?;
    let shifted = tree.transform()?;
    let text = tree.text("Hi", 16.0, [1, 2, 3, 4])?;
    tree.add_child(shifted, text)?;
    tree.add_child(clip, shifted)?;
    tree.layout()?;
    let before = tree.rect(text)?;
    tree.set_transform(shifted, 40.0, 0.0, 0.0, 2.0)?;
    assert!(tree.is_dirty());
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
                scale,
            },
            protocol::Command::Text { utf8, x, y, .. },
            protocol::Command::Pop,
            protocol::Command::Pop,
        ] if *translate_x == 40.0
            && *translate_y == 0.0
            && *rotate_rad == 0.0
            && *scale == 2.0
            && utf8 == "Hi"
            && *x == before.x
            && *y == before.y
    ));
    let region = tree.pointer_region()?;
    let target = tree.sized(16.0, 16.0)?;
    tree.add_child(region, target)?;
    tree.add_child(shifted, region)?;
    tree.layout()?;
    let spot = tree.rect(target)?;
    assert_eq!(
        tree.pointer(event(Phase::Down, spot.x + 1.0, spot.y + 1.0)),
        vec![Gesture::Press(region)]
    );
    assert!(
        tree.pointer(event(Phase::Down, spot.x + 40.0, spot.y + 1.0))
            .is_empty()
    );
    assert!(matches!(
        tree.set_transform(text, 0.0, 0.0, 0.0, 1.0),
        Err(crate::Error::WrongKind)
    ));
    Ok(())
}

#[test]
fn click_moves_focus_and_a_locked_drag_does_not() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let scroll = tree.scroll()?;
    let column = tree.column(0.0)?;
    let field = tree.pointer_region()?;
    let plain = tree.pointer_region()?;
    let field_box = tree.sized(40.0, 20.0)?;
    let plain_box = tree.sized(40.0, 20.0)?;
    tree.set_focusable(field, true)?;
    tree.add_child(field, field_box)?;
    tree.add_child(plain, plain_box)?;
    tree.add_child(column, field)?;
    tree.add_child(column, plain)?;
    tree.add_child(scroll, column)?;
    tree.set_viewport(0.0, 0.0, 40.0, 80.0);
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
    let mut tree = Tree::new(MonoShaper);
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
fn measure_text_matches_the_laid_out_paragraph() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper);
    let color = [9, 8, 7, 6];
    let text = tree.text("Hello", 10.0, color)?;
    tree.set_viewport(0.0, 0.0, 10.0, 40.0);
    tree.layout()?;
    let rect = tree.rect(text)?;
    let measured = tree.measure_text("Hello", 10.0, 10.0, color, &protocol::TextStyle::default());
    assert_eq!((measured.width, measured.height), (rect.w, rect.h));
    Ok(())
}

fn texts(tree: &Tree) -> Result<Vec<String>, crate::Error> {
    Ok(tree
        .record()?
        .commands
        .iter()
        .filter_map(|command| match command {
            protocol::Command::Text { utf8, .. } => Some(utf8.clone()),
            _ => None,
        })
        .collect())
}
