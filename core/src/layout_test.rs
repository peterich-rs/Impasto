use crate::tree::Affine;
use crate::{
    AxisSize, Button, DragAxis, Edges, Gesture, MonoShaper, Phase, PointerEvent, PointerId,
    RecognizerSet, Shaper, TrackSize, Tree,
};

fn event(phase: Phase, x: f32, y: f32) -> PointerEvent {
    PointerEvent::new(phase, x, y, 0.0)
}

fn button_event(phase: Phase, x: f32, y: f32, button: Button) -> PointerEvent {
    let mut event = PointerEvent::new(phase, x, y, 0.0);
    event.button = button;
    event.buttons = crate::Buttons::from_button(button);
    event
}

fn timed(phase: Phase, x: f32, y: f32, time_ms: f64) -> PointerEvent {
    PointerEvent::new(phase, x, y, time_ms)
}

#[test]
fn same_size_does_not_request_layout() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let sized = tree.sized(10.0, 20.0)?;
    tree.layout()?;
    let _ = tree.take_layout_trace();
    assert!(!tree.needs_layout());
    tree.set_size(sized, 10.0, 20.0)?;
    assert!(!tree.needs_layout());
    assert_eq!(tree.take_layout_trace().cause, crate::LayoutCause::None);
    tree.set_size(sized, 12.0, 20.0)?;
    assert!(tree.needs_layout());
    assert_eq!(tree.take_layout_trace().cause, crate::LayoutCause::Style);
    Ok(())
}

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
fn column_places_the_second_child_after_the_gap() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
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
    let mut tree = Tree::new(MonoShaper::default());
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
    let mut tree = Tree::new(MonoShaper::default());
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
    let mut tree = Tree::new(MonoShaper::default());
    let text = tree.text("Hi", 16.0, [255, 255, 255, 255])?;
    tree.layout()?;
    let rect = tree.rect(text)?;
    assert_eq!((rect.w, rect.h), (16.0, 16.0));
    Ok(())
}

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
fn viewport_wraps_text_to_the_available_width() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 10.0, 40.0)?;
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
fn set_text_keeps_the_hit_box_until_the_next_layout() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
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
        Some(protocol::Command::Text { utf8, .. }) if utf8.as_ref() == "Hello"
    ));
    Ok(())
}

#[test]
fn insert_before_places_the_child_in_order() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
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
fn stale_id_after_remove_is_unknown_node() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
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
    let mut tree = Tree::new(MonoShaper::default());
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
    let mut tree = Tree::new(MonoShaper::default());
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
    let mut tree = Tree::new(MonoShaper::default());
    let column = tree.column(0.0)?;
    let text = tree.text("Hi", 16.0, [1, 2, 3, 4])?;
    tree.add_child(column, text)?;
    tree.layout()?;
    tree.remove(column, text)?;
    assert!(!texts(&mut tree)?.contains(&"Hi".to_string()));
    tree.insert(column, text, None)?;
    tree.layout()?;
    assert_eq!(tree.rect(text)?.w, 16.0);
    assert!(texts(&mut tree)?.contains(&"Hi".to_string()));
    Ok(())
}

#[test]
fn setters_relayout_on_the_next_frame() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
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

    let mut tree = Tree::new(MonoShaper::default());
    let pad = tree.padding(0.0)?;
    let child = tree.sized(10.0, 12.0)?;
    tree.add_child(pad, child)?;
    tree.layout()?;
    tree.set_padding(pad, 3.0)?;
    assert!(tree.is_dirty());
    assert_eq!(tree.rect(child)?.x, 0.0);
    tree.layout()?;
    assert_eq!((tree.rect(child)?.x, tree.rect(child)?.y), (3.0, 3.0));

    let mut tree = Tree::new(MonoShaper::default());
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

    let mut tree = Tree::new(MonoShaper::default());
    let background = tree.decorated([1, 2, 3, 4], 0.0)?;
    let box_ = tree.sized(8.0, 8.0)?;
    tree.add_child(background, box_)?;
    tree.layout()?;
    tree.set_color(background, [9, 8, 7, 6])?;
    assert!(tree.needs_paint());
    assert!(!tree.needs_layout());
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
    let mut tree = Tree::new(MonoShaper::default());
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
fn stack_places_children_at_the_same_origin() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
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
fn grow_fills_free_space_on_the_next_layout() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
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
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(12.0, 30.0, 80.0, 100.0)?;
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
fn setters_update_paint_without_moving_the_previous_box() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let clip = tree.clip(1.0)?;
    let child = tree.sized(20.0, 10.0)?;
    tree.add_child(clip, child)?;
    tree.layout()?;
    let before = tree.rect(child)?;
    tree.set_radius(clip, 6.0)?;
    assert!(tree.needs_paint());
    assert!(!tree.needs_layout());
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

    let mut tree = Tree::new(MonoShaper::default());
    let fade = tree.opacity(255)?;
    let box_ = tree.sized(8.0, 8.0)?;
    tree.add_child(fade, box_)?;
    tree.layout()?;
    let width = tree.rect(box_)?.w;
    tree.set_alpha(fade, 40)?;
    assert!(tree.needs_paint());
    assert!(!tree.needs_layout());
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

    let mut tree = Tree::new(MonoShaper::default());
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

    let mut tree = Tree::new(MonoShaper::default());
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

    let mut tree = Tree::new(MonoShaper::default());
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
        Some(protocol::Command::Image { source, .. }) if source.as_ref() == "asset:mark"
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
fn mono_shaper_counts_scalars() {
    let shaper = MonoShaper::default();
    let size = shaper
        .metrics(
            &crate::paint_key(
                std::sync::Arc::from("中"),
                std::sync::Arc::new(protocol::TextStyle::default()),
                &[],
                16.0,
                1_000_000.0,
                [0, 0, 0, 255],
            )
            .metrics,
        )
        .size;
    assert_eq!(
        size,
        crate::TextSize {
            width: 8.0,
            height: 16.0
        }
    );
    let latin = shaper
        .metrics(
            &crate::paint_key(
                std::sync::Arc::from("Hi"),
                std::sync::Arc::new(protocol::TextStyle::default()),
                &[],
                16.0,
                1_000_000.0,
                [0, 0, 0, 255],
            )
            .metrics,
        )
        .size;
    assert_eq!(latin.width, 16.0);
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
fn justify_and_wrap_use_the_flex_line() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
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

    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 50.0, 80.0)?;
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
fn padding_edges_offset_the_child() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let pad = tree.padding(0.0)?;
    tree.set_padding_edges(
        pad,
        Edges {
            top: 6.0,
            right: 0.0,
            bottom: 0.0,
            left: 4.0,
        },
    )?;
    let child = tree.sized(10.0, 10.0)?;
    tree.add_child(pad, child)?;
    tree.layout()?;
    let rect = tree.rect(child)?;
    assert_eq!((rect.x, rect.y), (4.0, 6.0));
    Ok(())
}

#[test]
fn margin_offsets_the_child() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let column = tree.column(0.0)?;
    let child = tree.sized(10.0, 10.0)?;
    tree.add_child(column, child)?;
    tree.set_margin(
        child,
        Edges {
            top: 6.0,
            right: 0.0,
            bottom: 0.0,
            left: 4.0,
        },
    )?;
    tree.layout()?;
    let rect = tree.rect(child)?;
    assert_eq!((rect.x, rect.y), (4.0, 6.0));
    Ok(())
}

#[test]
fn grid_tracks_assign_fixed_and_fraction_columns() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let grid = tree.grid_tracks(
        &[TrackSize::Points(40.0), TrackSize::Fraction(1.0)],
        &[TrackSize::MinContent],
        0.0,
    )?;
    let left = tree.column(0.0)?;
    let right = tree.column(0.0)?;
    tree.add_child(grid, left)?;
    tree.add_child(grid, right)?;
    tree.set_fill(grid)?;
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    tree.layout()?;
    let second = tree.rect(right)?;
    assert_eq!(second.x, 40.0);
    assert_eq!(second.w, 60.0);
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
fn measure_text_matches_the_laid_out_paragraph() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let color = [9, 8, 7, 6];
    let text = tree.text("Hello", 10.0, color)?;
    tree.set_viewport(0.0, 0.0, 10.0, 40.0)?;
    tree.layout()?;
    let rect = tree.rect(text)?;
    let measured = tree.measure_text("Hello", 10.0, 10.0, color, &protocol::TextStyle::default());
    assert_eq!((measured.width, measured.height), (rect.w, rect.h));
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
fn caret_at_a_point_uses_mono_advances() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
    tree.layout()?;
    let rect = tree.rect(text)?;
    let caret = tree.caret_at(text, rect.x + 12.0, rect.y + 1.0)?;
    assert_eq!(caret.unit, 2);
    assert!((caret.x - 16.0).abs() < 0.1, "caret x {}", caret.x);
    let word = tree.word_range(text, 0)?;
    assert_eq!(word, (0, 2));
    Ok(())
}

#[test]
fn vertical_motion_moves_between_wrapped_lines() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 20.0, 80.0)?;
    let root = tree.column(0.0)?;
    let text = tree.text("abcd", 16.0, [0, 0, 0, 255])?;
    tree.add_child(root, text)?;
    tree.layout()?;
    let loaded = tree.text_edit(text, crate::Edit::Load("abcd".to_owned()))?;
    assert_eq!(loaded.selection.extent, 4);
    let up = tree.text_edit(text, crate::Edit::Up)?;
    assert!(
        up.selection.extent < 4,
        "up stays on the first line, extent {}",
        up.selection.extent
    );
    let down = tree.text_edit(text, crate::Edit::Down)?;
    assert_eq!(down.selection.extent, 4);
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
fn reinsert_keeps_the_measured_text() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 200.0, 400.0)?;
    let column = tree.column(0.0)?;
    let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
    tree.add_child(column, text)?;
    tree.layout()?;
    let height = tree.rect(text)?.h;
    let _ = tree.take_layout_trace();
    tree.remove(column, text)?;
    assert!(tree.parent(text)?.is_none());
    assert_eq!(tree.rect(text)?.h, height);
    tree.insert(column, text, None)?;
    tree.layout()?;
    let trace = tree.take_layout_trace();
    assert_eq!(trace.text_shaped, 0, "shaped {}", trace.text_shaped);
    assert_eq!(tree.rect(text)?.h, height);
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

fn texts(tree: &mut Tree) -> Result<Vec<String>, crate::Error> {
    Ok(tree
        .record()?
        .commands
        .iter()
        .filter_map(|command| match command {
            protocol::Command::Text { utf8, .. } => Some(utf8.to_string()),
            _ => None,
        })
        .collect())
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

fn picture_generation(tree: &protocol::LayerTree, id: crate::NodeId) -> u64 {
    tree.picture(crate::layer::child_picture_id(id))
        .map(|picture| picture.generation)
        .expect("picture")
}

fn reachable(tree: &protocol::LayerTree) -> usize {
    fn walk(layers: &[protocol::Layer], id: protocol::LayerId, seen: &mut [bool]) {
        let index = id.0 as usize;
        if index >= layers.len() || seen[index] {
            return;
        }
        seen[index] = true;
        if let Some(child) = layers[index].first_child {
            walk(layers, child, seen);
        }
        if let Some(next) = layers[index].next {
            walk(layers, next, seen);
        }
    }
    if tree.layers.is_empty() {
        return 0;
    }
    let mut seen = vec![false; tree.layers.len()];
    walk(&tree.layers, tree.root, &mut seen);
    seen.into_iter().filter(|hit| *hit).count()
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
fn align_centers_a_shrink_wrapped_column() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 200.0, 100.0)?;
    let frame = tree.align(crate::Alignment::Center)?;
    tree.set_fill(frame)?;
    let column = tree.column(0.0)?;
    let box_ = tree.sized(40.0, 20.0)?;
    tree.add_child(column, box_)?;
    tree.add_child(frame, column)?;
    tree.layout()?;
    let child = tree.rect(box_)?;
    assert!((child.x - 80.0).abs() < 0.5, "x={}", child.x);
    assert!((child.y - 40.0).abs() < 0.5, "y={}", child.y);
    Ok(())
}

#[test]
fn a_row_end_cross_axis_pins_a_short_child() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 80.0, 40.0)?;
    let row = tree.row(0.0)?;
    tree.set_fill(row)?;
    let child = tree.column(0.0)?;
    let inner = tree.sized(10.0, 10.0)?;
    tree.add_child(child, inner)?;
    tree.add_child(row, child)?;
    tree.set_row_cross(row, crate::RowCrossAlign::End)?;
    tree.layout()?;
    let end = tree.rect(child)?;
    assert!((end.y - 30.0).abs() < 0.5, "y={}", end.y);
    tree.set_row_cross(row, crate::RowCrossAlign::Stretch)?;
    tree.layout()?;
    let stretched = tree.rect(child)?;
    assert!((stretched.h - 40.0).abs() < 0.5, "h={}", stretched.h);
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

fn agree(tree: &mut Tree) -> Result<(), crate::Error> {
    let list = tree.record()?;
    agree_list(tree, &list)
}

fn record_writes(tree: &mut Tree) -> Result<usize, crate::Error> {
    let _ = tree.record()?;
    Ok(tree.picture_writes())
}

fn agree_list(tree: &Tree, list: &protocol::DrawList) -> Result<(), crate::Error> {
    let layers = tree.layer_tree();
    let flat = scene_marks_in_commands(&list.commands, Affine::identity(), &[]);
    let layered = scene_marks_in_layers(&layers);
    assert_eq!(
        flat.len(),
        layered.len(),
        "flat {flat:?}\nlayers {layered:?}"
    );
    for (left, right) in flat.iter().zip(layered.iter()) {
        assert_eq!(left.label, right.label, "{flat:?} vs {layered:?}");
        assert!(
            (left.x - right.x).abs() < 0.05 && (left.y - right.y).abs() < 0.05,
            "{left:?} vs {right:?}"
        );
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct Mark {
    label: String,
    x: f32,
    y: f32,
}

#[derive(Clone, Copy)]
struct SceneClip {
    ctm: Affine,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

fn inside_clips(point: (f32, f32), clips: &[SceneClip]) -> bool {
    clips.iter().all(|clip| {
        let (x0, y0) = clip.ctm.apply(clip.x, clip.y);
        let (x1, y1) = clip.ctm.apply(clip.x + clip.w, clip.y + clip.h);
        let (min_x, max_x) = (x0.min(x1), x0.max(x1));
        let (min_y, max_y) = (y0.min(y1), y0.max(y1));
        point.0 >= min_x - 0.01
            && point.0 <= max_x + 0.01
            && point.1 >= min_y - 0.01
            && point.1 <= max_y + 0.01
    })
}

fn push_mark(
    marks: &mut Vec<Mark>,
    label: String,
    x: f32,
    y: f32,
    ctm: Affine,
    clips: &[SceneClip],
) {
    let point = ctm.apply(x, y);
    if inside_clips(point, clips) {
        marks.push(Mark {
            label,
            x: point.0,
            y: point.1,
        });
    }
}

fn scene_marks_in_commands(
    commands: &[protocol::Command],
    ctm: Affine,
    clips: &[SceneClip],
) -> Vec<Mark> {
    let mut marks = Vec::new();
    let mut stack = vec![(ctm, clips.to_vec())];
    for command in commands {
        let (ctm, clips) = stack.last().expect("frame").clone();
        match command {
            protocol::Command::Text { utf8, x, y, .. } => {
                push_mark(&mut marks, utf8.to_string(), *x, *y, ctm, &clips);
            }
            protocol::Command::Image { source, x, y, .. } => {
                push_mark(&mut marks, source.to_string(), *x, *y, ctm, &clips);
            }
            protocol::Command::Rect { x, y, color, .. } => {
                push_mark(&mut marks, format!("rect:{color:?}"), *x, *y, ctm, &clips);
            }
            protocol::Command::PushClip { x, y, w, h, .. } => {
                let mut clips = clips;
                clips.push(SceneClip {
                    ctm,
                    x: *x,
                    y: *y,
                    w: *w,
                    h: *h,
                });
                stack.push((ctm, clips));
            }
            protocol::Command::PushTransform {
                translate_x,
                translate_y,
                rotate_rad,
                scale_x,
                scale_y,
            } => {
                stack.push((
                    Affine::paint(*translate_x, *translate_y, *rotate_rad, *scale_x, *scale_y)
                        .around(ctm),
                    clips,
                ));
            }
            protocol::Command::PushOpacity { .. }
            | protocol::Command::PushBlur { .. }
            | protocol::Command::PushBackdropBlur { .. }
            | protocol::Command::PushBlend { .. } => {
                stack.push((ctm, clips));
            }
            protocol::Command::Pop => {
                stack.pop();
            }
            protocol::Command::Fill { .. } => {}
        }
    }
    marks
}

fn scene_marks_in_layers(tree: &protocol::LayerTree) -> Vec<Mark> {
    fn walk(
        tree: &protocol::LayerTree,
        id: protocol::LayerId,
        ctm: Affine,
        clips: &[SceneClip],
        marks: &mut Vec<Mark>,
    ) {
        let mut current = Some(id);
        while let Some(layer_id) = current {
            let Some(layer) = tree.layers.get(layer_id.0 as usize) else {
                return;
            };
            let next = layer.next;
            let child = layer.first_child;
            match layer.kind {
                protocol::LayerKind::Offset { x, y } => {
                    if let Some(child) = child {
                        walk(
                            tree,
                            child,
                            Affine::translate(x, y).around(ctm),
                            clips,
                            marks,
                        );
                    }
                }
                protocol::LayerKind::Transform {
                    translate_x,
                    translate_y,
                    rotate_rad,
                    scale_x,
                    scale_y,
                } => {
                    if let Some(child) = child {
                        walk(
                            tree,
                            child,
                            Affine::paint(translate_x, translate_y, rotate_rad, scale_x, scale_y)
                                .around(ctm),
                            clips,
                            marks,
                        );
                    }
                }
                protocol::LayerKind::Clip { x, y, w, h, .. } => {
                    if let Some(child) = child {
                        let mut clips = clips.to_vec();
                        clips.push(SceneClip { ctm, x, y, w, h });
                        walk(tree, child, ctm, &clips, marks);
                    }
                }
                protocol::LayerKind::Opacity { .. }
                | protocol::LayerKind::Blur { .. }
                | protocol::LayerKind::BackdropBlur { .. }
                | protocol::LayerKind::Blend { .. } => {
                    if let Some(child) = child {
                        walk(tree, child, ctm, clips, marks);
                    }
                }
                protocol::LayerKind::Picture { picture } => {
                    if let Some(picture) = tree.picture(picture) {
                        let local =
                            Affine::translate(picture.origin.0, picture.origin.1).around(ctm);
                        marks.extend(scene_marks_in_commands(
                            picture.commands.as_ref(),
                            local,
                            clips,
                        ));
                    }
                }
            }
            current = next;
        }
    }
    let mut marks = Vec::new();
    if !tree.layers.is_empty() {
        walk(tree, tree.root, Affine::identity(), &[], &mut marks);
    }
    marks
}

#[test]
fn remove_then_discard_rejects_set_text() {
    let mut tree = Tree::new(MonoShaper::default());
    let root = tree.column(0.0).unwrap();
    let label = tree.text("Go", 16.0, [255, 255, 255, 255]).unwrap();
    tree.add_child(root, label).unwrap();
    tree.remove(root, label).unwrap();
    tree.discard(label).unwrap();
    assert!(tree.set_text(label, "no").is_err());
}
