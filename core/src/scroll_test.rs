use crate::{
    Gesture, OverscrollMode, Phase, PointerEvent, ScrollPhase, ScrollPhysics, SnapAlign, SnapGrid,
    Tree,
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
