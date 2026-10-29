use crate::{
    Gesture, OverscrollMode, Phase, PointerEvent, ScrollAccept, ScrollAxis, ScrollPhase,
    ScrollPhysics, SnapAlign, SnapGrid, Tree,
};

fn mono() -> crate::MonoShaper {
    crate::MonoShaper::default()
}

fn event(phase: Phase, x: f32, y: f32, time: f64) -> PointerEvent {
    PointerEvent::new(phase, x, y, time)
}

#[test]
fn default_physics_keeps_the_friction_asymptote() -> Result<(), crate::Error> {
    let mut tree = Tree::new(mono());
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll()?;
    let body = tree.image("asset:a", 80.0, 400.0)?;
    tree.add_child(scroll, body)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;
    tree.set_scroll_offset(scroll, 0.0, 40.0)?;
    tree.arm_fling(scroll, 0.0, -0.2)?;
    let target = tree.fling_target(scroll).expect("coasting");
    let asymptote = 40.0 + 0.2 * crate::SCROLL_TAU_MS;
    let (max_y, _) = (tree.rect(body)?.h - tree.rect(scroll)?.h, 0.0);
    let _ = max_y;
    let limit = (tree.rect(body)?.h - tree.rect(scroll)?.h).max(0.0);
    assert!(
        (target.1 - asymptote.min(limit)).abs() < 0.5,
        "{target:?} limit {limit}"
    );
    Ok(())
}

#[test]
fn clamp_mode_stores_no_overscroll() -> Result<(), crate::Error> {
    let mut tree = Tree::new(mono());
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll()?;
    let body = tree.image("asset:a", 80.0, 200.0)?;
    tree.add_child(scroll, body)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;
    let mut physics = ScrollPhysics::default();
    physics.overscroll = OverscrollMode::Clamp;
    tree.set_scroll_physics(scroll, physics)?;
    tree.set_scroll_offset(scroll, 0.0, -40.0)?;
    assert_eq!(tree.scroll_offset(scroll)?.1, 0.0);
    assert_eq!(tree.overscroll(scroll)?.1, 0.0);
    physics.overscroll = OverscrollMode::RubberBand;
    tree.set_scroll_physics(scroll, physics)?;
    tree.set_scroll_offset(scroll, 0.0, -40.0)?;
    assert_eq!(tree.scroll_offset(scroll)?.1, 0.0);
    assert!(tree.overscroll(scroll)?.1 < -1.0);
    Ok(())
}

#[test]
fn snap_y_settles_on_the_interval() -> Result<(), crate::Error> {
    let mut tree = Tree::new(mono());
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll()?;
    let body = tree.image("asset:a", 80.0, 400.0)?;
    tree.add_child(scroll, body)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;
    let mut physics = ScrollPhysics::default();
    physics.snap_y = Some(SnapGrid {
        interval: 50.0,
        align: SnapAlign::Start,
    });
    tree.set_scroll_physics(scroll, physics)?;
    tree.set_scroll_offset(scroll, 0.0, 40.0)?;
    tree.arm_fling(scroll, 0.0, 0.0)?;
    tree.finish_motions();
    // No coast. Drive a short friction that ends inside the range, then snap.
    tree.arm_fling(scroll, 0.0, -0.05)?;
    for step in 0..80 {
        tree.tick_motions(f64::from(step) * 16.0);
    }
    let y = tree.scroll_offset(scroll)?.1;
    let nearest = (y / 50.0).round() * 50.0;
    assert!((y - nearest).abs() < 1.0, "offset {y} nearest {nearest}");
    Ok(())
}

#[test]
fn a_coast_that_hits_the_inner_edge_continues_on_the_parent() -> Result<(), crate::Error> {
    let mut tree = Tree::new(mono());
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
    tree.set_scroll_offset(outer, 0.0, 50.0)?;
    assert!(
        tree.scroll_offset(outer)?.1 > 10.0,
        "outer cannot scroll {:?}",
        tree.scroll_offset(outer)
    );
    tree.set_scroll_offset(outer, 0.0, 0.0)?;
    tree.set_scroll_offset(inner, 0.0, 20.0)?;
    tree.arm_fling(inner, 0.0, -1.5)?;
    let mut moved_outer = false;
    for step in 0..120 {
        tree.tick_motions(f64::from(step) * 16.0);
        if tree.scroll_offset(outer)?.1 > 1.0 {
            moved_outer = true;
            break;
        }
    }
    assert!(
        moved_outer,
        "outer {:?} inner {:?} target {:?}",
        tree.scroll_offset(outer),
        tree.scroll_offset(inner),
        tree.fling_target(inner)
    );
    assert!(tree.scroll_offset(inner)?.1 > 100.0);

    tree.set_scroll_chain(inner, false)?;
    tree.set_scroll_offset(inner, 0.0, 20.0)?;
    tree.set_scroll_offset(outer, 0.0, 0.0)?;
    tree.stop_scroll(inner)?;
    tree.stop_scroll(outer)?;
    tree.arm_fling(inner, 0.0, -1.5)?;
    for step in 0..120 {
        tree.tick_motions(f64::from(step) * 16.0);
    }
    assert!(
        tree.scroll_offset(outer)?.1 < 0.5,
        "chain off still moved the outer {:?}",
        tree.scroll_offset(outer)
    );
    assert!(tree.scroll_offset(inner)?.1 > 100.0);
    let _ = ScrollPhase::Idle;
    let _ = event;
    Ok(())
}

#[test]
fn overscroll_spring_returns_to_idle_at_zero() -> Result<(), crate::Error> {
    let mut tree = Tree::new(mono());
    tree.set_viewport(0.0, 0.0, 100.0, 80.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll()?;
    let body = tree.image("asset:a", 80.0, 400.0)?;
    tree.add_child(scroll, body)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;
    tree.pointer(event(Phase::Down, 20.0, 20.0, 0.0));
    tree.pointer(event(Phase::Move, 20.0, 40.0, 20.0));
    tree.set_scroll_offset(scroll, 0.0, -80.0)?;
    assert!(tree.overscroll(scroll)?.1 < -1.0);
    tree.pointer(event(Phase::Up, 20.0, 40.0, 5_000.0));
    let _ = tree.take_scroll_notices();
    let mut idle = false;
    for step in 0..40 {
        tree.tick_motions(f64::from(step + 1) * 16.0);
        for notice in tree.take_scroll_notices() {
            if let Gesture::ScrollPhaseChanged { id, phase, oy, .. } = notice
                && id == scroll
                && phase == ScrollPhase::Idle
            {
                assert!(oy.abs() < 0.5, "idle overscroll {oy}");
                idle = true;
            }
        }
    }
    assert!(idle, "spring never returned to idle");
    assert!(tree.overscroll(scroll)?.1.abs() < 0.5);
    Ok(())
}

fn viewport_scroll(
    axis: ScrollAxis,
    content_w: f32,
    content_h: f32,
) -> Result<(Tree, crate::NodeId), crate::Error> {
    let mut tree = Tree::new(mono());
    tree.set_viewport(0.0, 0.0, 100.0, 80.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll_with(axis)?;
    tree.set_fill(scroll)?;
    tree.set_max_size(scroll, 100.0, 80.0)?;
    let body = tree.sized(content_w, content_h)?;
    tree.add_child(scroll, body)?;
    tree.add_child(root, scroll)?;
    tree.layout()?;
    Ok((tree, scroll))
}

fn began(gestures: &[Gesture], id: crate::NodeId) -> bool {
    gestures
        .iter()
        .any(|gesture| matches!(gesture, Gesture::DragBegin(node) if *node == id))
}

#[test]
fn both_still_scrolls_on_either_axis() -> Result<(), crate::Error> {
    let (mut tree, scroll) = viewport_scroll(ScrollAxis::Both, 400.0, 400.0)?;
    tree.set_scroll_offset(scroll, 30.0, 20.0)?;
    let (x, y) = tree.scroll_offset(scroll)?;
    assert!(x > 1.0 && y > 1.0, "both axes move: {x} {y}");
    let _ = tree.pointer(event(Phase::Down, 20.0, 20.0, 0.0));
    let dragged = tree.pointer(event(Phase::Move, 50.0, 20.0, 16.0));
    assert!(
        began(&dragged, scroll),
        "horizontal drag locks a both scroll: {dragged:?}"
    );
    Ok(())
}

#[test]
fn vertical_drops_horizontal_offset_overscroll_and_fling() -> Result<(), crate::Error> {
    let (mut tree, scroll) = viewport_scroll(ScrollAxis::Vertical, 400.0, 400.0)?;
    tree.set_scroll_offset(scroll, 80.0, 24.0)?;
    let (x, y) = tree.scroll_offset(scroll)?;
    assert_eq!(x, 0.0);
    assert!(y > 1.0, "vertical offset {y}");
    tree.set_scroll_offset(scroll, -40.0, -30.0)?;
    assert_eq!(tree.scroll_offset(scroll)?.0, 0.0);
    assert_eq!(tree.overscroll(scroll)?.0, 0.0);
    assert!(tree.overscroll(scroll)?.1 < -1.0);
    tree.set_scroll_offset(scroll, 0.0, 24.0)?;
    tree.arm_fling(scroll, 0.4, -0.2)?;
    let target = tree.fling_target(scroll).expect("y coast");
    assert!(target.0.abs() < 0.5, "x fling {target:?}");
    assert!(target.1 > 24.0, "y fling {target:?}");
    Ok(())
}

#[test]
fn vertical_drag_ignores_sideways_motion_but_keeps_raw_drag_by() -> Result<(), crate::Error> {
    let (mut tree, scroll) = viewport_scroll(ScrollAxis::Vertical, 400.0, 400.0)?;
    tree.set_scroll_offset(scroll, 0.0, 30.0)?;
    tree.layout()?;
    let _ = tree.pointer(event(Phase::Down, 20.0, 20.0, 0.0));
    let sideways = tree.pointer(event(Phase::Move, 50.0, 20.0, 16.0));
    assert!(
        !began(&sideways, scroll),
        "pure cross-axis drag is not claimed: {sideways:?}"
    );

    let _ = tree.pointer(event(Phase::Down, 20.0, 20.0, 32.0));
    let diagonal = tree.pointer(event(Phase::Move, 42.0, 36.0, 48.0));
    assert!(
        began(&diagonal, scroll),
        "a diagonal drag with dy still locks the vertical scroll: {diagonal:?}"
    );
    assert!(
        diagonal.iter().any(|gesture| {
            matches!(gesture, Gesture::DragBy(_, dx, dy) if dx.abs() > 1.0 && dy.abs() > 1.0)
        }),
        "DragBy keeps both components: {diagonal:?}"
    );
    tree.set_scroll_offset(scroll, 90.0, 12.0)?;
    let (x, y) = tree.scroll_offset(scroll)?;
    assert_eq!(x, 0.0);
    assert!((y - 12.0).abs() < 0.5, "only y was written: {y}");
    Ok(())
}

#[test]
fn declared_axis_edge_follows_accept_and_both_still_locks() -> Result<(), crate::Error> {
    let (mut tree, scroll) = viewport_scroll(ScrollAxis::Vertical, 80.0, 400.0)?;
    let _ = tree.pointer(event(Phase::Down, 20.0, 20.0, 0.0));
    let content = tree.pointer(event(Phase::Move, 20.0, 44.0, 16.0));
    assert!(
        !began(&content, scroll),
        "content at the start edge does not claim: {content:?}"
    );

    let (mut tree, scroll) = viewport_scroll(ScrollAxis::Vertical, 80.0, 400.0)?;
    let mut physics = ScrollPhysics::default();
    physics.accept = ScrollAccept::Always;
    tree.set_scroll_physics(scroll, physics)?;
    let _ = tree.pointer(event(Phase::Down, 20.0, 20.0, 0.0));
    let always = tree.pointer(event(Phase::Move, 20.0, 44.0, 16.0));
    assert!(
        began(&always, scroll),
        "always claims at the edge: {always:?}"
    );
    tree.set_scroll_offset(scroll, -20.0, -40.0)?;
    assert_eq!(tree.overscroll(scroll)?.0, 0.0);
    assert!(tree.overscroll(scroll)?.1 < -1.0);

    let (mut tree, scroll) = viewport_scroll(ScrollAxis::Both, 80.0, 400.0)?;
    let _ = tree.pointer(event(Phase::Down, 20.0, 20.0, 0.0));
    let locked = tree.pointer(event(Phase::Move, 20.0, 44.0, 16.0));
    assert!(
        began(&locked, scroll),
        "both still locks the overflowing edge: {locked:?}"
    );
    Ok(())
}

#[test]
fn never_yields_and_a_cross_axis_parent_can_win_before_lock() -> Result<(), crate::Error> {
    let mut tree = Tree::new(mono());
    tree.set_viewport(0.0, 0.0, 120.0, 80.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let outer = tree.scroll_with(ScrollAxis::Horizontal)?;
    tree.set_fill(outer)?;
    tree.set_max_size(outer, 120.0, 80.0)?;
    let row = tree.row(0.0)?;
    let inner = tree.scroll_with(ScrollAxis::Vertical)?;
    tree.set_max_size(inner, 80.0, 60.0)?;
    let inner_body = tree.sized(200.0, 300.0)?;
    let wide = tree.sized(400.0, 40.0)?;
    tree.add_child(inner, inner_body)?;
    tree.add_child(row, inner)?;
    tree.add_child(row, wide)?;
    tree.add_child(outer, row)?;
    tree.add_child(root, outer)?;
    tree.layout()?;
    tree.set_scroll_offset(inner, 0.0, 40.0)?;
    tree.layout()?;

    let _ = tree.pointer(event(Phase::Down, 16.0, 16.0, 0.0));
    let diagonal = tree.pointer(event(Phase::Move, 36.0, 32.0, 16.0));
    assert!(
        began(&diagonal, inner),
        "inner vertical wins a diagonal drag: {diagonal:?}"
    );

    let _ = tree.pointer(event(Phase::Up, 36.0, 32.0, 32.0));
    let _ = tree.pointer(event(Phase::Down, 16.0, 16.0, 48.0));
    let sideways = tree.pointer(event(Phase::Move, 2.0, 16.0, 64.0));
    assert!(
        began(&sideways, outer),
        "a pure horizontal drag falls through to the pager: {sideways:?}"
    );

    let mut physics = ScrollPhysics::default();
    physics.accept = ScrollAccept::Never;
    tree.set_scroll_physics(inner, physics)?;
    tree.set_scroll_offset(outer, 20.0, 0.0)?;
    let _ = tree.pointer(event(Phase::Up, 2.0, 16.0, 80.0));
    let _ = tree.pointer(event(Phase::Down, 16.0, 16.0, 96.0));
    let yielded = tree.pointer(event(Phase::Move, 16.0, 40.0, 112.0));
    assert!(
        !began(&yielded, inner),
        "never skipped the inner scroll: {yielded:?}"
    );
    assert!(
        !began(&yielded, outer),
        "a vertical drag does not lock the horizontal pager: {yielded:?}"
    );
    Ok(())
}

#[test]
fn vertical_wheel_drops_horizontal_and_fling_does_not_push_a_pager() -> Result<(), crate::Error> {
    let (mut tree, scroll) = viewport_scroll(ScrollAxis::Vertical, 400.0, 400.0)?;
    tree.set_scroll_offset(scroll, 0.0, 30.0)?;
    tree.layout()?;
    let horizontal = tree.pointer(PointerEvent::wheel(20.0, 20.0, 0.0, 12.0, 0.0));
    assert!(horizontal.is_empty(), "horizontal wheel: {horizontal:?}");
    let mixed = tree.pointer(PointerEvent::wheel(20.0, 20.0, 1.0, 12.0, 8.0));
    assert!(
        mixed.iter().any(|gesture| matches!(gesture, Gesture::Wheel(id, dx, dy) if *id == scroll && *dx == 0.0 && *dy == 8.0)),
        "wheel keeps only dy: {mixed:?}"
    );

    let mut tree = Tree::new(mono());
    tree.set_viewport(0.0, 0.0, 120.0, 80.0)?;
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let outer = tree.scroll_with(ScrollAxis::Horizontal)?;
    tree.set_fill(outer)?;
    tree.set_max_size(outer, 120.0, 80.0)?;
    let column = tree.column(0.0)?;
    let inner = tree.scroll_with(ScrollAxis::Vertical)?;
    tree.set_max_size(inner, 100.0, 40.0)?;
    let inner_body = tree.sized(80.0, 300.0)?;
    let wide = tree.sized(400.0, 40.0)?;
    tree.add_child(inner, inner_body)?;
    tree.add_child(column, inner)?;
    tree.add_child(column, wide)?;
    tree.add_child(outer, column)?;
    tree.add_child(root, outer)?;
    tree.layout()?;
    tree.set_scroll_offset(inner, 0.0, 20.0)?;
    tree.arm_fling(inner, 1.2, -1.5)?;
    for step in 0..80 {
        tree.tick_motions(f64::from(step) * 16.0);
    }
    assert!(
        tree.scroll_offset(outer)?.0 < 0.5,
        "vertical fling moved the pager {:?}",
        tree.scroll_offset(outer)
    );
    assert!(tree.scroll_offset(inner)?.1 > 20.0);
    Ok(())
}

#[test]
fn scroll_into_view_on_a_vertical_scroll_leaves_x() -> Result<(), crate::Error> {
    let mut tree = Tree::new(mono());
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    let scroll = tree.scroll_with(ScrollAxis::Vertical)?;
    tree.set_axis_size(
        scroll,
        crate::AxisSize::Points(100.0),
        crate::AxisSize::Points(40.0),
    )?;
    let child = tree.sized(240.0, 200.0)?;
    tree.add_child(scroll, child)?;
    tree.layout()?;
    assert_eq!(tree.scroll_into_view(child, crate::ScrollEdge::End)?, true);
    tree.tick_motions(800.0);
    let (x, y) = tree.scroll_offset(scroll)?;
    assert_eq!(x, 0.0);
    assert!(y > 1.0, "revealed y {y}");
    Ok(())
}
