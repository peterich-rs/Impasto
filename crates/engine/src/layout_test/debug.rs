//! Overflow scan and inspector probes. See `tree/debug.rs`.

use crate::tree::DebugAxis;
use crate::{Error, input::PointerEvent, text::MonoShaper, tree::Tree};

fn tree() -> Tree {
    Tree::new(MonoShaper::default())
}

fn pointer(x: f32, y: f32, phase: crate::input::Phase) -> PointerEvent {
    PointerEvent::new(phase, x, y, 0.0)
}

#[test]
fn a_row_with_a_too_wide_child_reports_overflow() -> Result<(), Error> {
    let mut tree = tree();
    let root = tree.row(0.0)?;
    tree.set_axis_size(
        root,
        crate::tree::AxisSize::Points(20.0),
        crate::tree::AxisSize::Points(40.0),
    )?;
    let wide = tree.text("wide", 16.0, [0, 0, 0, 255])?;
    tree.add_child(root, wide)?;
    tree.layout()?;
    assert!(tree.inspect_layout_issues().contains("\"excess\":12.00"));
    assert!(tree.take_layout_issues().is_empty());

    tree.set_debug_scan(true);
    tree.set_text(wide, "test")?;
    tree.layout()?;
    let issues = tree.layout_issues();
    // MonoShaper: 4 chars at font 16 are 4 * 8 = 32 wide inside 20.
    assert_eq!(issues.len(), 1, "{issues:?}");
    let issue = &issues[0];
    assert_eq!(issue.parent, root);
    assert_eq!(issue.axis, DebugAxis::Horizontal);
    assert_eq!(issue.worst_child, wide);
    assert!((issue.excess - 12.0).abs() < 0.6, "{issue:?}");
    Ok(())
}

#[test]
fn scan_reports_only_new_signatures_across_frames() -> Result<(), Error> {
    let mut tree = tree();
    let root = tree.column(0.0)?;
    tree.set_axis_size(
        root,
        crate::tree::AxisSize::Points(40.0),
        crate::tree::AxisSize::Points(20.0),
    )?;
    let tall = tree.text("hello", 16.0, [0, 0, 0, 255])?;
    tree.add_child(root, tall)?;
    let filler = tree.text("row", 16.0, [0, 0, 0, 255])?;
    tree.add_child(root, filler)?;
    tree.set_debug_scan(true);
    tree.layout()?;
    let first = tree.take_layout_issues();
    // Two 16pt lines stack to 32 inside a 20pt column.
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].axis, DebugAxis::Vertical);
    // Same tree again: nothing new.
    tree.set_text(tall, "there")?;
    tree.layout()?;
    assert!(tree.take_layout_issues().is_empty());
    assert_eq!(tree.layout_issues().len(), 1);
    Ok(())
}

#[test]
fn scroll_content_never_reports() -> Result<(), Error> {
    let mut tree = tree();
    let root = tree.column(0.0)?;
    tree.set_fill(root)?;
    let scroll = tree.scroll()?;
    tree.add_child(root, scroll)?;
    let inner = tree.column(0.0)?;
    tree.add_child(scroll, inner)?;
    for index in 0..8 {
        let row = tree.text(format!("row{index}"), 16.0, [0, 0, 0, 255])?;
        tree.add_child(inner, row)?;
    }
    tree.set_debug_scan(true);
    tree.layout()?;
    assert!(
        tree.layout_issues().is_empty(),
        "{:?}",
        tree.layout_issues()
    );
    Ok(())
}

#[test]
fn visible_text_overflow_never_reports() -> Result<(), Error> {
    let mut tree = tree();
    let root = tree.column(0.0)?;
    tree.set_axis_size(
        root,
        crate::tree::AxisSize::Points(60.0),
        crate::tree::AxisSize::Points(60.0),
    )?;
    let long = tree.text("a very long line that cannot wrap", 16.0, [0, 0, 0, 255])?;
    tree.add_child(root, long)?;
    tree.set_soft_wrap(long, false)?;
    tree.set_overflow(long, crate::scene::TextOverflow::Visible)?;
    tree.set_debug_scan(true);
    tree.layout()?;
    assert!(
        tree.layout_issues().is_empty(),
        "{:?}",
        tree.layout_issues()
    );
    Ok(())
}

#[test]
fn clip_subtree_never_reports() -> Result<(), Error> {
    let mut tree = tree();
    let root = tree.column(0.0)?;
    tree.set_axis_size(
        root,
        crate::tree::AxisSize::Points(80.0),
        crate::tree::AxisSize::Points(80.0),
    )?;
    let clip = tree.clip(0.0)?;
    tree.add_child(root, clip)?;
    let row = tree.row(0.0)?;
    tree.set_axis_size(
        row,
        crate::tree::AxisSize::Points(20.0),
        crate::tree::AxisSize::Points(20.0),
    )?;
    tree.add_child(clip, row)?;
    let wide = tree.text("wide", 16.0, [0, 0, 0, 255])?;
    tree.add_child(row, wide)?;
    tree.set_debug_scan(true);
    tree.layout()?;
    assert!(
        tree.layout_issues().is_empty(),
        "{:?}",
        tree.layout_issues()
    );
    Ok(())
}

#[test]
fn scan_off_leaves_the_report_empty() -> Result<(), Error> {
    let mut tree = tree();
    let root = tree.row(0.0)?;
    tree.set_axis_size(
        root,
        crate::tree::AxisSize::Points(10.0),
        crate::tree::AxisSize::Points(10.0),
    )?;
    let wide = tree.text("wide text", 16.0, [0, 0, 0, 255])?;
    tree.add_child(root, wide)?;
    tree.layout()?;
    assert!(tree.layout_issues().is_empty());
    assert!(tree.take_layout_issues().is_empty());
    Ok(())
}

#[test]
fn snapshot_cuts_debug_owned_subtrees() -> Result<(), Error> {
    let mut tree = tree();
    let root = tree.column(0.0)?;
    let label = tree.text("app", 16.0, [0, 0, 0, 255])?;
    tree.add_child(root, label)?;
    let panel = tree.column(0.0)?;
    tree.add_child(root, panel)?;
    let panel_label = tree.text("devtool", 16.0, [0, 0, 0, 255])?;
    tree.add_child(panel, panel_label)?;
    tree.layout()?;
    let full = tree.debug_snapshot();
    assert!(full.contains("devtool"));
    tree.set_debug_owned(panel, true);
    let cut = tree.debug_snapshot();
    assert!(!cut.contains("devtool"), "{cut}");
    assert!(cut.contains("app"));
    tree.set_debug_owned(panel, false);
    assert!(tree.debug_snapshot().contains("devtool"));
    Ok(())
}

#[test]
fn probe_returns_the_root_to_leaf_chain() -> Result<(), Error> {
    let mut tree = tree();
    let root = tree.column(0.0)?;
    let inner = tree.row(0.0)?;
    tree.add_child(root, inner)?;
    let label = tree.text("hit", 16.0, [0, 0, 0, 255])?;
    tree.add_child(inner, label)?;
    let outside = tree.text("miss", 16.0, [0, 0, 0, 255])?;
    tree.add_child(root, outside)?;
    tree.layout()?;
    let json = tree.debug_probe(2.0, 2.0);
    assert!(json.contains("\"chain\":["));
    assert!(json.contains("\"kind\":\"column\""));
    assert!(json.contains("\"kind\":\"row\""));
    assert!(json.contains("\"kind\":\"text\""));
    // The sibling the point misses is absent.
    assert!(!json.contains("miss"), "{json}");
    Ok(())
}

#[test]
fn suppressed_pointers_update_the_trace_without_dispatch() -> Result<(), Error> {
    let mut tree = tree();
    let root = tree.column(0.0)?;
    let region = tree.pointer_region()?;
    let label = tree.text("go", 16.0, [0, 0, 0, 255])?;
    tree.add_child(region, label)?;
    tree.add_child(root, region)?;
    tree.layout()?;
    tree.set_pointer_trace(true, true);
    let suppressed = tree.pointer(pointer(4.0, 4.0, crate::input::Phase::Down));
    assert!(suppressed.is_empty(), "{suppressed:?}");
    let (x, y, phase) = tree.debug_last_pointer().expect("traced");
    assert_eq!((x, y), (4.0, 4.0));
    assert_eq!(phase, "down");
    let up = tree.pointer(pointer(4.0, 4.0, crate::input::Phase::Up));
    assert!(up.is_empty(), "{up:?}");
    assert_eq!(tree.debug_last_pointer().map(|p| p.2), Some("up"));
    tree.set_pointer_trace(true, false);
    let live = tree.pointer(pointer(4.0, 4.0, crate::input::Phase::Down));
    assert!(
        live.iter()
            .any(|gesture| matches!(gesture, crate::tree::Gesture::Press(_))),
        "{live:?}"
    );
    Ok(())
}

#[test]
fn overlay_appends_to_the_scene_without_changing_the_view_tree() -> Result<(), Error> {
    use crate::tree::RemoteOverlayConfig;

    let mut tree = tree();
    let root = tree.column(0.0)?;
    let label = tree.text("overlay target", 16.0, [0, 0, 0, 255])?;
    tree.add_child(root, label)?;
    tree.layout()?;
    tree.record_layers()?;
    let before = tree.debug_snapshot();
    let probe = tree.debug_probe(2.0, 2.0);
    let pictures = tree.layer_tree().pictures.len();
    tree.set_remote_overlay(RemoteOverlayConfig {
        highlight: Some(label),
        pick_mode: true,
        ..Default::default()
    });
    tree.record_layers()?;
    assert_eq!(tree.layer_tree().pictures.len(), pictures + 1);
    assert_eq!(tree.debug_snapshot(), before);
    assert_eq!(tree.debug_probe(2.0, 2.0), probe);
    assert!(
        tree.pointer(pointer(2.0, 2.0, crate::input::Phase::Down))
            .is_empty()
    );
    assert_eq!(
        tree.take_picked_view()
            .and_then(|picked| picked.chain.last().copied()),
        Some(label)
    );
    tree.set_remote_overlay(RemoteOverlayConfig::default());
    tree.record_layers()?;
    assert_eq!(tree.layer_tree().pictures.len(), pictures);
    Ok(())
}

#[test]
fn recorded_repaints_are_coalesced_and_generations_do_not_share_stats() -> Result<(), Error> {
    use crate::tree::RepaintReason;

    let mut tree = tree();
    let root = tree.column(0.0)?;
    let label = tree.text("a", 16.0, [0, 0, 0, 255])?;
    tree.add_child(root, label)?;
    tree.begin_repaint_frame(true, 1);
    tree.layout()?;
    tree.record_layers()?;
    let _ = tree.take_repaints();
    tree.begin_repaint_frame(true, 2);
    tree.set_text(label, "bb")?;
    tree.set_text(label, "ccc")?;
    tree.layout()?;
    tree.record_layers()?;
    let batch = tree
        .take_repaints()
        .expect("changed label recorded a picture");
    let entry = batch
        .entries
        .iter()
        .find(|record| record.node == label)
        .expect("label attributed");
    assert!(entry.reason.contains(RepaintReason::TEXT_CONTENT));
    assert!(entry.reason.contains(RepaintReason::LAYOUT_GEOMETRY));
    assert_eq!(
        batch
            .entries
            .iter()
            .filter(|record| record.node == label)
            .count(),
        1
    );
    tree.remove(root, label)?;
    tree.discard_detached();
    assert!(
        !tree
            .repaint_leaderboard(100)
            .iter()
            .any(|entry| entry.node == label)
    );
    Ok(())
}
