use super::*;

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
fn mono_shaper_measures_hi_at_font_16() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let text = tree.text("Hi", 16.0, [255, 255, 255, 255])?;
    tree.layout()?;
    let rect = tree.rect(text)?;
    assert_eq!((rect.w, rect.h), (16.0, 16.0));
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
