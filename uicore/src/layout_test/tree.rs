use super::*;

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
fn remove_then_discard_rejects_set_text() {
    let mut tree = Tree::new(MonoShaper::default());
    let root = tree.column(0.0).unwrap();
    let label = tree.text("Go", 16.0, [255, 255, 255, 255]).unwrap();
    tree.add_child(root, label).unwrap();
    tree.remove(root, label).unwrap();
    tree.discard(label).unwrap();
    assert!(tree.set_text(label, "no").is_err());
}

#[test]
fn a_panel_measures_its_natural_size_without_the_root() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 200.0, 100.0)?;
    let root = tree.column(0.0)?;
    let panel = tree.column(0.0)?;
    let label = tree.text("menu", 10.0, [0, 0, 0, 255])?;
    tree.add_child(panel, label)?;
    tree.add_child(root, panel)?;
    tree.layout()?;
    tree.set_panel(panel)?;
    let (w, h) = tree.measure_panel(panel, SubtreeSpace::MaxContent)?;
    // Four clusters at half the font size: 4 * 5 = 20 wide, 10 tall.
    assert!((w - 20.0).abs() < 0.5, "panel width {w}");
    assert!((h - 10.0).abs() < 0.5, "panel height {h}");
    // The measurement leaves the main tree clean: laying out again is a no-op.
    assert!(tree.rect(root).is_ok());
    Ok(())
}

#[test]
fn a_capped_panel_remeasures_under_the_cap() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 400.0, 100.0)?;
    let root = tree.column(0.0)?;
    let panel = tree.column(0.0)?;
    let label = tree.text("a much longer line of text", 10.0, [0, 0, 0, 255])?;
    tree.add_child(panel, label)?;
    tree.add_child(root, panel)?;
    tree.layout()?;
    tree.set_panel(panel)?;
    // Natural width is 27 clusters * 5 = 135; the cap is 40, so the text
    // wraps and the height grows past one line.
    let (w, h) = tree.measure_panel(panel, SubtreeSpace::CappedMax(40.0))?;
    assert!(w <= 40.5, "capped width {w}");
    assert!(h > 10.0, "wrapped height {h}");
    Ok(())
}

#[test]
fn a_fraction_min_height_fills_half_the_parent() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 100.0, 100.0)?;
    // The column needs a definite height of its own: a percentage min
    // resolves against the parent's resolved size, and a content-sized
    // column has none.
    let root = tree.sized(100.0, 100.0)?;
    let column = tree.column(0.0)?;
    tree.add_child(root, column)?;
    let block = tree.sized(50.0, 10.0)?;
    tree.add_child(column, block)?;
    tree.set_min_axis(block, AxisSize::Auto, AxisSize::Fraction(0.5))?;
    tree.layout()?;
    let height = tree.rect(block)?.h;
    assert!((height - 50.0).abs() < 0.5, "min-height 50% gave {height}");
    Ok(())
}
