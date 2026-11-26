use std::time::Duration;

use crate::scene::{CanvasBrush, CanvasIsolation, CanvasOp, PathVerb};
use kurbo::PathEl;

use super::path::Path2D;
use super::{
    Budget, CanvasError, CanvasErrorCode, CanvasSize, Deadline, FillRule, MAX_DASH, PaintEnv,
    Recorder, TextAlign, TextBaseline, prepare,
};
use crate::text::MonoShaper;

fn ok<T>(result: Result<T, CanvasError>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("canvas error: {error}"),
    }
}

fn new_recorder(budget: &Budget) -> Recorder<'_> {
    Recorder::new(
        CanvasSize {
            width: 120.0,
            height: 80.0,
        },
        PaintEnv::default(),
        budget,
    )
}

fn finish(recorder: Recorder<'_>, shaper: &mut MonoShaper) -> crate::scene::CanvasList {
    ok(prepare(
        recorder.finish(),
        shaper,
        &Deadline::after(Duration::from_secs(2)),
    ))
}

#[test]
fn defaults_are_black_butt_and_source_over() {
    let budget = Budget::session();
    let recorder = new_recorder(&budget);
    assert_eq!(recorder.fill_color(), Some([0, 0, 0, 255]));
    assert!((recorder.line_width() - 1.0).abs() < f64::EPSILON);
    assert!((recorder.global_alpha() - 1.0).abs() < f64::EPSILON);
    assert_eq!(recorder.font().size, 10.0);
    assert_eq!(recorder.font().families, vec!["Sans".to_string()]);
    assert_eq!(recorder.get_transform(), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let _ = (TextAlign::Start, TextBaseline::Alphabetic);
}

#[test]
fn invalid_assignments_keep_the_previous_value() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    recorder.set_fill_style("#fff");
    recorder.set_fill_style("currentColor");
    recorder.set_fill_style("var(--ink)");
    recorder.set_fill_style("not-a-color");
    assert_eq!(recorder.fill_color(), Some([255, 255, 255, 255]));
    recorder.set_line_width(0.0);
    recorder.set_line_width(-2.0);
    recorder.set_line_width(f64::NAN);
    assert!((recorder.line_width() - 1.0).abs() < f64::EPSILON);
    recorder.set_global_alpha(1.5);
    assert!((recorder.global_alpha() - 1.0).abs() < f64::EPSILON);
    recorder.set_font("italic 700 16px Sans");
    recorder.set_font("nope");
    assert_eq!(recorder.font().size, 16.0);
    assert_eq!(recorder.font().slant, crate::scene::FontSlant::Italic);
    assert_eq!(recorder.font().weight, 700);
}

#[test]
fn dash_repeats_odd_lengths_and_rejects_negatives() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    ok(recorder.set_line_dash(&[4.0, 2.0, 1.0]));
    assert_eq!(recorder.line_dash(), &[4.0, 2.0, 1.0, 4.0, 2.0, 1.0]);
    ok(recorder.set_line_dash(&[4.0, -1.0]));
    assert_eq!(recorder.line_dash(), &[4.0, 2.0, 1.0, 4.0, 2.0, 1.0]);
    ok(recorder.set_line_dash(&[0.0, 0.0]));
    assert!(recorder.line_dash().is_empty());
}

#[test]
fn simple_canvas_primitives_keep_their_geometry_and_paint_order() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    recorder.translate(2.0, 4.0);
    ok(recorder.fill_rects(&[6.0, 3.0, -4.0, 2.0, 4.0, 6.0, 2.0, 3.0]));
    ok(recorder.move_to(3.0, 4.0));
    ok(recorder.line_to(9.0, 4.0));
    ok(recorder.stroke());
    recorder.begin_path();
    ok(recorder.arc(9.0, 7.0, 2.0, 0.0, std::f64::consts::TAU, false));
    recorder.reset_transform();
    ok(recorder.fill(FillRule::EvenOdd));
    ok(recorder.rect(0.0, 0.0, 5.0, 5.0));
    ok(recorder.fill(FillRule::EvenOdd));

    let list = finish(recorder, &mut MonoShaper::new());
    let [first, second, line, circle, compound] = list.ops() else {
        panic!("expected one operation per draw");
    };
    for (op, expected_bounds, expected_radius) in [
        (first, [4.0, 7.0, 4.0, 2.0], 0.0),
        (second, [6.0, 10.0, 2.0, 3.0], 0.0),
        (line, [5.0, 7.5, 6.0, 1.0], 0.0),
        (circle, [9.0, 9.0, 4.0, 4.0], 2.0),
    ] {
        let CanvasOp::SolidRect { bounds, radius, .. } = op else {
            panic!("expected a solid rectangle");
        };
        assert_eq!(*bounds, expected_bounds);
        assert_eq!(*radius, expected_radius);
    }
    assert!(matches!(compound, CanvasOp::Fill { .. }));
}

#[test]
fn translate_is_baked_once() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    recorder.translate(10.0, 4.0);
    ok(recorder.move_to(0.0, 0.0));
    ok(recorder.line_to(5.0, 0.0));
    ok(recorder.fill(FillRule::NonZero));
    let list = finish(recorder, &mut MonoShaper::new());
    let CanvasOp::Fill { path, .. } = &list.ops()[0] else {
        panic!("expected a fill");
    };
    assert!(
        matches!(path.verbs[0], PathVerb::Move(x, y) if (x - 10.0).abs() < 0.01 && (y - 4.0).abs() < 0.01)
    );
    assert!(
        matches!(path.verbs[1], PathVerb::Line(x, y) if (x - 15.0).abs() < 0.01 && (y - 4.0).abs() < 0.01)
    );
}

#[test]
fn fill_does_not_change_the_path_stroke_sees() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    ok(recorder.move_to(0.0, 0.0));
    ok(recorder.line_to(10.0, 0.0));
    ok(recorder.line_to(10.0, 10.0));
    ok(recorder.fill(FillRule::NonZero));
    ok(recorder.stroke());
    let list = finish(recorder, &mut MonoShaper::new());
    assert_eq!(list.ops().len(), 2);
    let CanvasOp::Fill { path, .. } = &list.ops()[0] else {
        panic!("fill");
    };
    assert!(
        path.verbs
            .iter()
            .any(|verb| matches!(verb, PathVerb::Close))
    );
    let CanvasOp::Fill { path: stroke, .. } = &list.ops()[1] else {
        panic!("stroke outline");
    };
    assert!(stroke.verbs.len() > path.verbs.len());
}

#[test]
fn begin_path_clears_and_save_does_not() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    ok(recorder.move_to(0.0, 0.0));
    ok(recorder.save());
    ok(recorder.line_to(8.0, 0.0));
    recorder.restore();
    ok(recorder.line_to(8.0, 6.0));
    ok(recorder.fill(FillRule::NonZero));
    let list = finish(recorder, &mut MonoShaper::new());
    let path = list.ops().iter().find_map(|op| match op {
        CanvasOp::Fill { path, .. } => Some(path),
        _ => None,
    });
    let Some(path) = path else {
        panic!("fill");
    };
    assert!(path.verbs.len() >= 3);

    let mut recorder = new_recorder(&budget);
    ok(recorder.move_to(0.0, 0.0));
    recorder.begin_path();
    ok(recorder.fill(FillRule::NonZero));
    let list = finish(recorder, &mut MonoShaper::new());
    assert!(list.ops().is_empty());
}

#[test]
fn empty_restore_is_a_noop_and_finish_balances_saves() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    recorder.restore();
    ok(recorder.save());
    ok(recorder.fill_rect(0.0, 0.0, 4.0, 4.0));
    let list = finish(recorder, &mut MonoShaper::new());
    assert!(matches!(list.ops()[0], CanvasOp::Save));
    assert!(matches!(list.ops().last(), Some(CanvasOp::Restore)));
}

#[test]
fn non_finite_draws_emit_nothing_and_negative_radius_errors() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    ok(recorder.move_to(f64::NAN, 0.0));
    ok(recorder.fill_rect(0.0, 0.0, f64::INFINITY, 4.0));
    ok(recorder.clear_rect(0.0, 0.0, 0.0, 4.0));
    let list = finish(recorder, &mut MonoShaper::new());
    assert!(list.ops().is_empty());

    let mut recorder = new_recorder(&budget);
    let error = match recorder.arc(0.0, 0.0, -1.0, 0.0, 1.0, false) {
        Ok(()) => panic!("negative radius must fail"),
        Err(error) => error,
    };
    assert_eq!(error.api, Some("arc"));
    let error = match recorder.round_rect(0.0, 0.0, 10.0, 10.0, &[1.0, -2.0]) {
        Ok(()) => panic!("negative radius must fail"),
        Err(error) => error,
    };
    assert_eq!(error.api, Some("roundRect"));
}

#[test]
fn singular_matrix_keeps_fill_geometry_and_skips_stroke() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    recorder.set_transform(0.0, 0.0, 0.0, 0.0, 3.0, 4.0);
    ok(recorder.rect(0.0, 0.0, 10.0, 10.0));
    ok(recorder.fill(FillRule::NonZero));
    ok(recorder.stroke());
    let list = finish(recorder, &mut MonoShaper::new());
    assert_eq!(list.ops().len(), 1);
    assert!(matches!(list.ops()[0], CanvasOp::Fill { .. }));
}

#[test]
fn clear_and_destination_blends_isolate() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    ok(recorder.fill_rect(0.0, 0.0, 4.0, 4.0));
    let list = finish(recorder, &mut MonoShaper::new());
    assert_eq!(list.isolation(), CanvasIsolation::Shared);

    let mut recorder = new_recorder(&budget);
    ok(recorder.clear_rect(1.0, 1.0, -4.0, 3.0));
    let list = finish(recorder, &mut MonoShaper::new());
    assert_eq!(list.isolation(), CanvasIsolation::Isolated);
    let CanvasOp::Clear { x, y, w, h, .. } = list.ops()[0] else {
        panic!("clear");
    };
    assert!((x + 3.0).abs() < 0.01 && (y - 1.0).abs() < 0.01);
    assert!((w - 4.0).abs() < 0.01 && (h - 3.0).abs() < 0.01);

    let mut recorder = new_recorder(&budget);
    recorder.set_composite(crate::scene::CanvasComposite::DestinationOut);
    ok(recorder.fill_rect(0.0, 0.0, 2.0, 2.0));
    let list = finish(recorder, &mut MonoShaper::new());
    assert_eq!(list.isolation(), CanvasIsolation::Isolated);
}

#[test]
fn degenerate_linear_gradient_emits_nothing() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    let mut gradient = ok(super::Gradient::linear(1.0, 1.0, 1.0, 1.0));
    ok(gradient.add_color_stop(0.0, "#000"));
    ok(gradient.add_color_stop(1.0, "#fff"));
    recorder.set_fill_gradient(&gradient);
    ok(recorder.fill_rect(0.0, 0.0, 8.0, 8.0));
    let list = finish(recorder, &mut MonoShaper::new());
    assert!(list.ops().is_empty());
}

#[test]
fn gradient_stops_are_sorted_and_transformed() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    recorder.translate(5.0, 0.0);
    let mut gradient = ok(super::Gradient::linear(0.0, 0.0, 10.0, 0.0));
    ok(gradient.add_color_stop(1.0, "rgba(0, 0, 0, 0)"));
    ok(gradient.add_color_stop(0.0, "#f00"));
    recorder.set_fill_gradient(&gradient);
    ok(recorder.fill_rect(0.0, 0.0, 4.0, 4.0));
    let list = finish(recorder, &mut MonoShaper::new());
    let CanvasOp::Fill { paint, .. } = &list.ops()[0] else {
        panic!("fill");
    };
    let CanvasBrush::Linear { start, end, stops } = &paint.brush else {
        panic!("linear");
    };
    assert!((start[0] - 5.0).abs() < 0.01);
    assert!((end[0] - 15.0).abs() < 0.01);
    assert!(stops[0].offset <= stops[1].offset);
    assert_eq!(stops[0].color, [255, 0, 0, 255]);
}

#[test]
fn negative_radius_on_a_reusable_path_names_the_api() {
    let mut path = Path2D::new();
    let error = match path.arc(0.0, 0.0, -3.0, 0.0, 1.0, false) {
        Ok(()) => panic!("negative radius must fail"),
        Err(error) => error,
    };
    assert_eq!(error.code, CanvasErrorCode::InvalidArgument);
    assert_eq!(error.api, Some("arc"));
    let copied = Path2D::from_path(&path);
    ok(path.rect(0.0, 0.0, 4.0, 4.0));
    assert_eq!(copied.segments(), 0);
}

#[test]
fn arc_to_rounds_a_right_angle() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    ok(recorder.move_to(0.0, 0.0));
    ok(recorder.arc_to(100.0, 0.0, 100.0, 100.0, 10.0));
    ok(recorder.line_to(100.0, 100.0));
    let deadline = Deadline::after(Duration::from_secs(1));
    assert!(ok(recorder.is_point_in_stroke(50.0, 0.0, &deadline)));
    assert!(!ok(recorder.is_point_in_stroke(100.0, 0.0, &deadline)));
}

#[test]
fn measure_text_is_uncompressed_and_empty_keeps_the_font_box() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    recorder.set_font("20px Sans");
    let mut shaper = MonoShaper::new();
    let empty = ok(recorder.measure_text("", &mut shaper));
    assert_eq!(empty.width, 0.0);
    assert!((empty.font_bounding_box_ascent - 16.0).abs() < 0.01);
    assert!((empty.font_bounding_box_descent - 4.0).abs() < 0.01);
    let word = ok(recorder.measure_text("ab\n", &mut shaper));
    assert!((word.width - 30.0).abs() < 0.01);
    ok(recorder.fill_text("ab", 0.0, 16.0, Some(5.0)));
    let list = finish(recorder, &mut shaper);
    let CanvasOp::Text(text) = &list.ops()[0] else {
        panic!("text");
    };
    assert_eq!(text.utf8.as_ref(), "ab");
    assert!(text.compress < 1.0);
    assert!(text.y.abs() < 0.01);
}

#[test]
fn point_query_uses_the_current_transform_once() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    recorder.scale(2.0, 2.0);
    ok(recorder.rect(0.0, 0.0, 10.0, 10.0));
    assert!(recorder.is_point_in_path(5.0, 5.0, FillRule::NonZero));
    assert!(!recorder.is_point_in_path(15.0, 5.0, FillRule::NonZero));
}

#[test]
fn dropped_list_refunds_the_session_budget() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    ok(recorder.fill_rect(0.0, 0.0, 4.0, 4.0));
    let list = finish(recorder, &mut MonoShaper::new());
    assert!(budget.used() >= list.bytes());
    drop(list);
    assert_eq!(budget.used(), 0);
}

#[test]
fn image_negative_destination_does_not_flip() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    ok(recorder.draw_image("asset:mark", None, 10.0, 8.0, Some(-4.0), Some(6.0)));
    let list = finish(recorder, &mut MonoShaper::new());
    let CanvasOp::Image(image) = &list.ops()[0] else {
        panic!("image");
    };
    assert!((image.x - 6.0).abs() < 0.01);
    let width = match image.w {
        Some(width) => width,
        None => panic!("destination width"),
    };
    assert!((width - 4.0).abs() < 0.01);
    assert_eq!(image.ctm, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    assert_eq!(list.images().len(), 1);
}

#[test]
fn empty_subpath_establishes_its_own_start() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    ok(recorder.close_path());
    ok(recorder.fill(FillRule::NonZero));
    assert!(recorder.finish().ops.is_empty());

    let mut line = Path2D::new();
    ok(line.close());
    assert!(line.path().elements().is_empty());
    ok(line.line_to(3.0, 4.0));
    assert!(matches!(
        line.path().elements(),
        [PathEl::MoveTo(point)] if point.x == 3.0 && point.y == 4.0
    ));
    ok(line.line_to(6.0, 8.0));
    assert!(matches!(
        line.path().elements(),
        [PathEl::MoveTo(start), PathEl::LineTo(end)] if start.x == 3.0 && end.x == 6.0
    ));

    let mut quad = Path2D::new();
    ok(quad.quad_to(1.0, 2.0, 3.0, 4.0));
    assert!(matches!(
        quad.path().elements(),
        [PathEl::MoveTo(start), PathEl::QuadTo(_, end)]
            if start.x == 1.0 && start.y == 2.0 && end.x == 3.0 && end.y == 4.0
    ));

    let mut cubic = Path2D::new();
    ok(cubic.cubic_to(1.0, 2.0, 3.0, 4.0, 5.0, 6.0));
    assert!(matches!(
        cubic.path().elements(),
        [PathEl::MoveTo(start), PathEl::CurveTo(_, _, end)]
            if start.x == 1.0 && start.y == 2.0 && end.x == 5.0 && end.y == 6.0
    ));
}

#[test]
fn gradient_added_after_assignment_affects_the_next_draw() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    let mut gradient = ok(super::Gradient::linear(0.0, 0.0, 10.0, 0.0));
    recorder.set_fill_gradient(&gradient);
    ok(gradient.add_color_stop(0.0, "#f00"));
    ok(recorder.fill_rect(0.0, 0.0, 4.0, 4.0));
    ok(gradient.add_color_stop(1.0, "#00f"));
    ok(recorder.fill_rect(4.0, 0.0, 4.0, 4.0));
    let list = finish(recorder, &mut MonoShaper::new());
    let stops = |index: usize| {
        let CanvasOp::Fill { paint, .. } = &list.ops()[index] else {
            panic!("fill");
        };
        let CanvasBrush::Linear { stops, .. } = &paint.brush else {
            panic!("linear");
        };
        stops.len()
    };
    assert_eq!(stops(0), 1);
    assert_eq!(stops(1), 2);
}

#[test]
fn empty_clip_stays_in_the_list() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    ok(recorder.save());
    ok(recorder.clip(FillRule::NonZero));
    ok(recorder.fill_rect(0.0, 0.0, 8.0, 8.0));
    recorder.restore();
    let list = finish(recorder, &mut MonoShaper::new());
    assert!(matches!(list.ops()[0], CanvasOp::Save));
    let CanvasOp::Clip { path } = &list.ops()[1] else {
        panic!("clip");
    };
    assert!(path.verbs.is_empty());
    assert!(matches!(list.ops()[2], CanvasOp::SolidRect { .. }));
    assert!(matches!(list.ops()[3], CanvasOp::Restore));
}

#[test]
fn dash_length_is_capped_before_the_copy() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    ok(recorder.set_line_dash(&[4.0, 2.0]));
    let too_long = vec![1.0; MAX_DASH + 1];
    let error = match recorder.set_line_dash(&too_long) {
        Ok(()) => panic!("oversized dash must fail"),
        Err(error) => error,
    };
    assert_eq!(error.code, CanvasErrorCode::LimitExceeded);
    assert_eq!(recorder.line_dash(), &[4.0, 2.0]);
    ok(recorder.set_line_dash(&[4.0, f64::NAN]));
    assert_eq!(recorder.line_dash(), &[4.0, 2.0]);

    let mut recorder = new_recorder(&budget);
    ok(recorder.set_line_dash(&[4.0, 2.0]));
    ok(recorder.move_to(0.0, 0.0));
    ok(recorder.line_to(20.0, 0.0));
    ok(recorder.stroke());
    let list = finish(recorder, &mut MonoShaper::new());
    assert!(matches!(list.ops()[0], CanvasOp::Fill { .. }));
}

#[test]
fn tiny_dash_is_rejected_before_stroke_expansion() {
    let budget = Budget::session();
    let mut recorder = new_recorder(&budget);
    ok(recorder.set_line_dash(&[1.0e-6, 1.0e-6]));
    ok(recorder.move_to(0.0, 0.0));
    ok(recorder.line_to(100.0, 0.0));
    let deadline = Deadline::after(Duration::from_secs(1));
    let error = match recorder.is_point_in_stroke(0.0, 0.0, &deadline) {
        Ok(_) => panic!("tiny dash must fail"),
        Err(error) => error,
    };
    assert_eq!(error.code, CanvasErrorCode::LimitExceeded);

    let mut recorder = new_recorder(&budget);
    ok(recorder.set_line_dash(&[1.0e-6, 1.0e-6]));
    ok(recorder.move_to(0.0, 0.0));
    ok(recorder.line_to(100.0, 0.0));
    ok(recorder.stroke());
    let error = match prepare(
        recorder.finish(),
        &mut MonoShaper::new(),
        &Deadline::after(Duration::from_secs(1)),
    ) {
        Ok(_) => panic!("tiny dash must fail in prepare"),
        Err(error) => error,
    };
    assert_eq!(error.code, CanvasErrorCode::LimitExceeded);
}

#[test]
fn svg_assets_keep_canonical_fill_and_stroke_under_canvas_transforms() {
    let budget = Budget::session();
    let path = Path2D::from_svg("M 1 2 L 18 2 L 18 15 Z");
    let mut recorder = new_recorder(&budget);
    ok(recorder.fill_path(&path, FillRule::EvenOdd));
    ok(recorder.stroke_path(&path));
    recorder.translate(22.0, 13.0);
    ok(recorder.fill_path(&path, FillRule::EvenOdd));
    ok(recorder.stroke_path(&path));
    let list = finish(recorder, &mut MonoShaper::new());
    for pair in [[0, 2], [1, 3]] {
        let [first, second] = pair.map(|index| match &list.ops()[index] {
            CanvasOp::Vector {
                path,
                asset,
                transform,
                ..
            } => (path, asset, transform),
            other => panic!("expected vector asset, got {other:?}"),
        });
        assert_eq!(first.1, second.1);
        assert_ne!(first.0, second.0);
        assert_ne!(first.2, second.2);
    }
}
