//! Zero-size clips hide descendants from focus, semantics, and hit testing.

use crate::{
    AxisSize, FocusDirection, Gesture, GridLine, Inset, MonoShaper, Phase, PointerEvent,
    SemanticsAction, SemanticsActions, SemanticsFlags, SemanticsHint, SemanticsRole, Shaper,
    TrackSize, Tree,
};

#[test]
fn reverse_is_only_for_a_row() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let row = tree.row(0.0)?;
    tree.set_reverse(row, true)?;
    let column = tree.column(0.0)?;
    assert!(matches!(
        tree.set_reverse(column, true),
        Err(crate::Error::WrongKind)
    ));
    let text = tree.text("a", 16.0, [0, 0, 0, 255])?;
    assert!(matches!(
        tree.set_reverse(text, true),
        Err(crate::Error::WrongKind)
    ));
    Ok(())
}

fn event(phase: Phase, x: f32, y: f32) -> PointerEvent {
    PointerEvent::new(phase, x, y, 0.0)
}

#[test]
fn a_zero_clip_hides_its_focusable_child() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let clip = tree.clip(0.0)?;
    tree.set_min_size(clip, 0.0, 0.0)?;
    tree.set_max_size(clip, 0.0, 0.0)?;
    let button = tree.pointer_region()?;
    let box_ = tree.sized(40.0, 24.0)?;
    tree.add_child(button, box_)?;
    tree.add_child(clip, button)?;
    tree.set_focusable(button, true)?;
    tree.layout()?;
    assert_eq!(tree.rect(clip)?.w, 0.0);
    assert_eq!(tree.rect(clip)?.h, 0.0);
    assert!(tree.focus_step(FocusDirection::Next).is_empty());
    assert!(tree.focused().is_none());
    let snapshot = tree.semantics().expect("snapshot");
    assert!(
        snapshot
            .nodes
            .iter()
            .all(|node| node.role != SemanticsRole::Button)
    );
    let spot = tree.rect(button)?;
    let hit = tree.pointer(event(
        Phase::Down,
        spot.x + spot.w * 0.5,
        spot.y + spot.h * 0.5,
    ));
    assert!(
        hit.iter()
            .all(|gesture| !matches!(gesture, Gesture::Press(id) if *id == button))
    );
    Ok(())
}

#[test]
fn a_positive_clip_keeps_its_focusable_child() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let clip = tree.clip(0.0)?;
    let button = tree.pointer_region()?;
    let box_ = tree.sized(40.0, 24.0)?;
    tree.add_child(button, box_)?;
    tree.add_child(clip, button)?;
    tree.set_focusable(button, true)?;
    tree.layout()?;
    assert!(tree.rect(clip)?.w > 0.0);
    let stepped = tree.focus_step(FocusDirection::Next);
    assert!(matches!(stepped.as_slice(), [Gesture::Focus(id)] if *id == button));
    let snapshot = tree.semantics().expect("snapshot");
    assert!(
        snapshot
            .nodes
            .iter()
            .any(|node| node.role == SemanticsRole::Button)
    );
    Ok(())
}

#[test]
fn a_disabled_pointer_drops_activate_and_focusable() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let button = tree.pointer_region()?;
    let box_ = tree.sized(40.0, 24.0)?;
    tree.add_child(button, box_)?;
    tree.set_focusable(button, true)?;
    tree.set_semantics(
        button,
        SemanticsHint {
            disabled: Some(true),
            label: Some("Save".to_owned()),
            ..SemanticsHint::default()
        },
    )?;
    tree.layout()?;
    let snapshot = tree.semantics().expect("snapshot");
    let node = snapshot
        .nodes
        .iter()
        .find(|node| node.label == "Save")
        .expect("button");
    assert!(node.flags.contains(SemanticsFlags::DISABLED));
    assert!(!node.flags.contains(SemanticsFlags::FOCUSABLE));
    assert!(!node.actions.contains(SemanticsActions::ACTIVATE));
    Ok(())
}

fn near(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() < 0.6
}

#[test]
fn a_percent_width_is_half_the_parent() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let parent = tree.column(0.0)?;
    tree.set_axis_size(parent, AxisSize::Points(200.0), AxisSize::Points(80.0))?;
    let child = tree.sized(10.0, 10.0)?;
    tree.add_child(parent, child)?;
    tree.set_axis_size(child, AxisSize::Fraction(0.5), AxisSize::Points(20.0))?;
    tree.layout()?;
    let percent = tree.rect(child)?;
    assert!(near(percent.w, 100.0), "width {}", percent.w);
    assert!(near(percent.h, 20.0), "height {}", percent.h);
    assert!(matches!(
        tree.set_axis_size(child, AxisSize::Auto, AxisSize::Fraction(-0.1)),
        Err(crate::Error::BadNumber)
    ));
    Ok(())
}

#[test]
fn aspect_ratio_sets_height_and_clear_removes_it() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let ratio = tree.sized(10.0, 10.0)?;
    tree.set_axis_size(ratio, AxisSize::Points(100.0), AxisSize::Auto)?;
    tree.set_aspect_ratio(ratio, Some(2.0))?;
    tree.layout()?;
    assert!(
        near(tree.rect(ratio)?.h, 50.0),
        "aspect {}",
        tree.rect(ratio)?.h
    );
    tree.set_aspect_ratio(ratio, None)?;
    tree.layout()?;
    assert!(
        tree.rect(ratio)?.h < 1.0,
        "cleared aspect still has height {}",
        tree.rect(ratio)?.h
    );
    assert!(matches!(
        tree.set_aspect_ratio(ratio, Some(0.0)),
        Err(crate::Error::BadNumber)
    ));
    Ok(())
}

#[test]
fn absolute_inset_offsets_from_the_stack_and_later_sibling_paints_after() -> Result<(), crate::Error>
{
    let mut tree = Tree::new(MonoShaper::default());
    let stack = tree.stack()?;
    tree.set_axis_size(stack, AxisSize::Points(80.0), AxisSize::Points(100.0))?;
    let placed = tree.decorated([1, 0, 0, 255], 0.0)?;
    let later = tree.decorated([0, 1, 0, 255], 0.0)?;
    tree.set_axis_size(placed, AxisSize::Points(20.0), AxisSize::Points(20.0))?;
    tree.set_axis_size(later, AxisSize::Points(16.0), AxisSize::Points(16.0))?;
    tree.add_child(stack, placed)?;
    tree.set_position(placed, true)?;
    tree.set_inset(
        placed,
        Inset {
            top: Some(10.0),
            right: None,
            bottom: None,
            left: Some(4.0),
        },
    )?;
    tree.add_child(stack, later)?;
    tree.layout()?;
    let stack_rect = tree.rect(stack)?;
    let placed_rect = tree.rect(placed)?;
    assert!(
        near(placed_rect.y - stack_rect.y, 10.0),
        "inset y {}",
        placed_rect.y - stack_rect.y
    );
    assert!(
        near(placed_rect.x - stack_rect.x, 4.0),
        "inset x {}",
        placed_rect.x - stack_rect.x
    );
    let list = tree.record()?;
    let colors: Vec<[u8; 4]> = list
        .commands
        .iter()
        .filter_map(|command| match command {
            protocol::Command::Rect { color, .. } => Some(*color),
            _ => None,
        })
        .collect();
    let red_at = colors.iter().position(|color| *color == [1, 0, 0, 255]);
    let green_at = colors.iter().position(|color| *color == [0, 1, 0, 255]);
    assert!(
        matches!((red_at, green_at), (Some(red), Some(green)) if red < green),
        "paint order {colors:?}"
    );
    assert!(matches!(
        tree.set_grid_placement(
            placed,
            GridLine::Line(1),
            GridLine::Auto,
            GridLine::Auto,
            GridLine::Auto,
        ),
        Err(crate::Error::WrongKind)
    ));
    Ok(())
}

#[test]
fn grid_line_two_lands_in_the_second_column() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let grid = tree.grid_tracks(
        &[
            TrackSize::Points(40.0),
            TrackSize::Points(50.0),
            TrackSize::Points(60.0),
        ],
        &[TrackSize::Points(20.0)],
        0.0,
    )?;
    let cell = tree.sized(10.0, 10.0)?;
    tree.add_child(grid, cell)?;
    assert!(matches!(
        tree.set_grid_placement(
            cell,
            GridLine::Line(0),
            GridLine::Auto,
            GridLine::Auto,
            GridLine::Auto
        ),
        Err(crate::Error::BadNumber)
    ));
    assert!(matches!(
        tree.set_grid_placement(
            cell,
            GridLine::Span(0),
            GridLine::Auto,
            GridLine::Auto,
            GridLine::Auto
        ),
        Err(crate::Error::BadNumber)
    ));
    tree.set_grid_placement(
        cell,
        GridLine::Line(2),
        GridLine::Line(3),
        GridLine::Auto,
        GridLine::Auto,
    )?;
    tree.layout()?;
    assert!(
        near(tree.rect(cell)?.x, 40.0),
        "column x {}",
        tree.rect(cell)?.x
    );
    Ok(())
}

#[test]
fn set_text_loads_the_editor_and_reports_replaced() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let text = tree.text("old", 16.0, [0, 0, 0, 255])?;
    tree.set_semantics(
        text,
        SemanticsHint {
            role: Some(SemanticsRole::TextField),
            editable: Some(true),
            read_only: Some(false),
            label: Some("Name".to_owned()),
            ..SemanticsHint::default()
        },
    )?;
    tree.layout()?;
    let snapshot = tree.semantics().expect("snapshot");
    let node = snapshot
        .nodes
        .iter()
        .find(|node| node.role == SemanticsRole::TextField)
        .expect("field");
    assert!(node.actions.contains(SemanticsActions::SET_TEXT));
    let gestures = tree.semantics_action(SemanticsAction::SetText {
        id: node.id,
        text: "Hello".to_owned(),
    });
    assert_eq!(gestures, vec![Gesture::TextReplaced(text)]);
    assert_eq!(tree.editor_text(text).as_deref(), Some("Hello"));
    Ok(())
}

#[test]
fn a_read_only_field_has_no_set_text() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let text = tree.text("old", 16.0, [0, 0, 0, 255])?;
    tree.set_semantics(
        text,
        SemanticsHint {
            role: Some(SemanticsRole::TextField),
            editable: Some(true),
            read_only: Some(true),
            ..SemanticsHint::default()
        },
    )?;
    tree.layout()?;
    let snapshot = tree.semantics().expect("snapshot");
    let node = snapshot
        .nodes
        .iter()
        .find(|node| node.role == SemanticsRole::TextField)
        .expect("field");
    assert!(node.flags.contains(SemanticsFlags::READ_ONLY));
    assert!(!node.flags.contains(SemanticsFlags::EDITABLE));
    assert!(!node.actions.contains(SemanticsActions::SET_TEXT));
    Ok(())
}

#[test]
fn negative_scale_x_mirrors_a_local_point() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let shifted = tree.transform()?;
    let region = tree.pointer_region()?;
    let target = tree.sized(16.0, 16.0)?;
    tree.add_child(region, target)?;
    tree.add_child(shifted, region)?;
    tree.layout()?;
    tree.set_scale_xy(shifted, -1.0, 1.0)?;
    let list = tree.record()?;
    assert!(list.commands.iter().any(|command| matches!(
        command,
        protocol::Command::PushTransform {
            scale_x,
            scale_y,
            ..
        } if *scale_x == -1.0 && *scale_y == 1.0
    )));
    let mirrored = tree.pointer(event(Phase::Down, -8.0, 8.0));
    assert!(
        mirrored
            .iter()
            .any(|gesture| matches!(gesture, Gesture::Press(id) if *id == region)),
        "mirrored hit {mirrored:?}"
    );
    let _ = tree.pointer(event(Phase::Cancel, -8.0, 8.0));
    let plain = tree.pointer(event(Phase::Down, 8.0, 8.0));
    assert!(
        plain
            .iter()
            .all(|gesture| !matches!(gesture, Gesture::Press(_))),
        "unmirrored side {plain:?}"
    );
    assert!(matches!(
        tree.set_scale_xy(shifted, 0.0, 1.0),
        Err(crate::Error::BadNumber)
    ));
    let kept = tree.record()?;
    assert!(kept.commands.iter().any(|command| matches!(
        command,
        protocol::Command::PushTransform { scale_x, .. } if *scale_x == -1.0
    )));
    Ok(())
}

#[test]
fn a_click_inside_a_link_range_activates_it() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let region = tree.pointer_region()?;
    let text = tree.text("hello", 16.0, [0, 0, 0, 255])?;
    tree.add_child(region, text)?;
    tree.set_text_runs(
        text,
        vec![
            protocol::TextSpan {
                utf8: "he".into(),
                color: [0, 0, 255, 255],
                weight: 700,
                underline: true,
                line_through: false,
                family: Some("One".into()),
            },
            protocol::TextSpan {
                utf8: "llo".into(),
                color: [0, 0, 0, 255],
                weight: 400,
                underline: false,
                line_through: false,
                family: Some("Two".into()),
            },
        ],
        vec![protocol::TextLink {
            start: 0,
            end: 2,
            id: 7,
        }],
    )?;
    tree.layout()?;
    let painted = tree.record()?;
    let command = painted.commands.iter().find_map(|command| match command {
        protocol::Command::Text {
            utf8, spans, links, ..
        } => Some((utf8, spans, links)),
        _ => None,
    });
    let Some((utf8, spans, links)) = command else {
        panic!("missing text");
    };
    assert_eq!(utf8.as_ref(), "hello");
    assert_eq!(spans.len(), 2);
    assert_eq!(
        links.as_ref(),
        &[protocol::TextLink {
            start: 0,
            end: 2,
            id: 7
        }][..]
    );
    let width = MonoShaper::default()
        .metrics(
            &crate::paint_key(
                std::sync::Arc::from("hello"),
                std::sync::Arc::new(protocol::TextStyle::default()),
                spans.as_ref(),
                16.0,
                1_000_000.0,
                [0, 0, 0, 255],
            )
            .metrics,
        )
        .size;
    assert!((width.width - 40.0).abs() < 0.01, "{}", width.width);
    tree.pointer(event(Phase::Down, 2.0, 8.0));
    let inside = tree.pointer(event(Phase::Up, 2.0, 8.0));
    assert!(
        inside.iter().any(|gesture| matches!(
            gesture,
            Gesture::ActivateLink { id, link } if *id == text && *link == 7
        )),
        "{inside:?}"
    );
    tree.pointer(event(Phase::Down, 28.0, 8.0));
    let outside = tree.pointer(event(Phase::Up, 28.0, 8.0));
    assert!(
        outside
            .iter()
            .all(|gesture| !matches!(gesture, Gesture::ActivateLink { .. })),
        "{outside:?}"
    );
    let row = tree.row(0.0)?;
    assert!(matches!(
        tree.set_text_runs(row, Vec::new(), Vec::new()),
        Err(crate::Error::NotText)
    ));
    Ok(())
}

#[test]
fn short_gradients_blur_and_src_over_stay_out_of_the_list() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let clip = tree.clip(0.0)?;
    let fill = tree.decorated([9, 9, 9, 255], 0.0)?;
    let sized = tree.sized(40.0, 20.0)?;
    tree.add_child(fill, sized)?;
    tree.add_child(clip, fill)?;
    tree.set_gradient(
        fill,
        Some(protocol::Gradient::Linear {
            start: [0.0, 0.0],
            end: [1.0, 0.0],
            stops: vec![protocol::ColorStop {
                offset: 0.0,
                color: [1, 2, 3, 255],
                space: protocol::ColorSpace::Srgb,
            }],
        }),
    )?;
    tree.set_blur(fill, 0.0)?;
    tree.set_backdrop_blur(fill, f32::NAN)?;
    tree.set_blend(clip, protocol::BlendMode::SrcOver)?;
    tree.layout()?;
    let plain = tree.record()?;
    assert!(plain.commands.iter().all(|command| {
        !matches!(
            command,
            protocol::Command::PushBlur { .. }
                | protocol::Command::PushBackdropBlur { .. }
                | protocol::Command::PushBlend { .. }
        )
    }));
    assert!(plain.commands.iter().any(|command| matches!(
        command,
        protocol::Command::Rect { gradient: None, color, .. } if *color == [9, 9, 9, 255]
    )));
    assert_eq!(count(&plain, true), count(&plain, false));

    tree.set_gradient(
        fill,
        Some(protocol::Gradient::Linear {
            start: [0.0, 0.0],
            end: [0.0, 0.0],
            stops: vec![
                protocol::ColorStop {
                    offset: 0.0,
                    color: [4, 0, 0, 255],
                    space: protocol::ColorSpace::Srgb,
                },
                protocol::ColorStop {
                    offset: 1.0,
                    color: [8, 0, 0, 255],
                    space: protocol::ColorSpace::Srgb,
                },
            ],
        }),
    )?;
    let solid = tree.record()?;
    assert!(solid.commands.iter().any(|command| matches!(
        command,
        protocol::Command::Rect { gradient: None, color, .. } if *color == [8, 0, 0, 255]
    )));

    tree.set_gradient(
        fill,
        Some(protocol::Gradient::Linear {
            start: [0.0, 0.0],
            end: [1.0, 1.0],
            stops: vec![
                protocol::ColorStop {
                    offset: 1.0,
                    color: [0, 0, 1, 255],
                    space: protocol::ColorSpace::Srgb,
                },
                protocol::ColorStop {
                    offset: 0.0,
                    color: [0, 0, 2, 255],
                    space: protocol::ColorSpace::Srgb,
                },
            ],
        }),
    )?;
    tree.set_blur(fill, 2.0)?;
    tree.set_blend(clip, protocol::BlendMode::Multiply)?;
    let painted = tree.record()?;
    assert!(painted.commands.iter().any(|command| matches!(
        command,
        protocol::Command::Rect { gradient: Some(protocol::Gradient::Linear { stops, .. }), .. }
            if stops.first().is_some_and(|stop| stop.offset == 0.0)
                && stops.last().is_some_and(|stop| stop.offset == 1.0)
    )));
    let blur_at = painted.commands.iter().position(
        |command| matches!(command, protocol::Command::PushBlur { sigma, .. } if *sigma == 2.0),
    );
    let clip_at = painted
        .commands
        .iter()
        .position(|command| matches!(command, protocol::Command::PushClip { .. }));
    assert!(clip_at.is_some_and(|clip| blur_at.is_some_and(|blur| clip < blur)));
    assert_eq!(count(&painted, true), count(&painted, false));
    let image = tree.image("asset:logo", 12.0, 8.0)?;
    tree.add_child(clip, image)?;
    tree.layout()?;
    let listed = tree.record()?;
    assert!(listed.commands.iter().any(|command| matches!(
        command,
        protocol::Command::Image {
            required: false,
            mode: protocol::ContentMode::Fill,
            ..
        }
    )));
    tree.set_image_mode(image, protocol::ContentMode::Fit, true)?;
    let fitted = tree.record()?;
    assert!(fitted.commands.iter().any(|command| matches!(
        command,
        protocol::Command::Image {
            required: true,
            mode: protocol::ContentMode::Fit,
            ..
        }
    )));
    Ok(())
}

fn count(list: &protocol::DrawList, pushes: bool) -> usize {
    list.commands
        .iter()
        .filter(|command| {
            let push = matches!(
                command,
                protocol::Command::PushClip { .. }
                    | protocol::Command::PushOpacity { .. }
                    | protocol::Command::PushTransform { .. }
                    | protocol::Command::PushBlur { .. }
                    | protocol::Command::PushBackdropBlur { .. }
                    | protocol::Command::PushBlend { .. }
            );
            if pushes {
                push
            } else {
                matches!(command, protocol::Command::Pop)
            }
        })
        .count()
}
