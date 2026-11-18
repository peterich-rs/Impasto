//! Native regressions for public component props and their scene consumers.
#![allow(
    clippy::expect_used,
    reason = "test fixtures fail immediately with setup and assertion context"
)]

use std::cell::RefCell;
use std::rc::Rc;

use protocol::Command;
use session::{FrameOutput, FrameTime, InputEvent, Key, KeyPress, Metrics, Session};
use uicore::{Phase, PointerEvent, SemanticsFlags, SemanticsOp};

fn fixture(name: &str, log: crate::ConsoleLog) -> Box<dyn Session> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = root.join(format!("target/{name}.js"));
    let status = std::process::Command::new("node")
        .arg(root.join("tools/pack.mjs"))
        .arg(root.join(format!("crates/quickjs/tests/fixtures/{name}.tsx")))
        .arg(&output)
        .status()
        .expect("pack component regression fixture");
    assert!(status.success(), "failed to pack {name}");
    let source = std::fs::read_to_string(output).expect("read component regression fixture");
    let mut app = crate::open(&source, name, uicore::MonoShaper::default(), log)
        .unwrap_or_else(|error| panic!("open {name}: {error}"));
    app.metrics(&Metrics::new(360.0, 640.0, 1.0));
    app
}

fn frame(app: &mut dyn Session, at: f64) -> FrameOutput {
    let output = app.frame(FrameTime::at(at));
    let faults = app.take_faults();
    assert!(faults.is_empty(), "component frame {at}: {faults:?}");
    output
}

#[test]
fn component_canvas_records_sync_draw_and_reports_async_draw() {
    let mut app = fixture("canvas", Box::new(|_, _| {}));
    let output = frame(app.as_mut(), 0.0);
    let painted = commands(&output)
        .filter(|command| matches!(command, Command::Canvas { .. }))
        .count();
    assert_eq!(painted, 1, "only the synchronous Canvas commits");
    app.frame(FrameTime::at(16.0));
    assert!(
        app.take_faults()
            .iter()
            .any(|fault| fault.source == "canvas" && fault.message.contains("AsyncDraw"))
    );
}

fn commands(frame: &FrameOutput) -> impl Iterator<Item = &Command> {
    frame
        .layers
        .iter()
        .flat_map(|tree| tree.pictures.iter())
        .flat_map(|picture| picture.commands.iter())
}

#[test]
fn component_style_fragments_defaults_and_ref_lifetimes() {
    let mut app = fixture("components", Box::new(|_, _| {}));
    let output = frame(app.as_mut(), 0.0);
    for (text, expected) in [
        ("field-array", 30.0),
        ("field-class", 36.0),
        ("field-reset", 16.0),
        ("error-array", 26.0),
        ("error-page-array", 28.0),
    ] {
        let size = commands(&output).find_map(|command| match command {
            Command::Text {
                utf8, font_size, ..
            } if utf8.as_ref() == text => Some(*font_size),
            _ => None,
        });
        assert_eq!(size, Some(expected), "DrawList font size for {text}");
    }
}

#[test]
fn component_events_compose_and_update_with_native_disabled_state() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&events);
    let mut app = fixture(
        "component-events",
        Box::new(move |_, message| sink.borrow_mut().push(message.to_owned())),
    );
    let initial = frame(app.as_mut(), 0.0);
    let patch = initial.semantics.as_ref().expect("boot semantics");
    let root = patch
        .ops
        .iter()
        .find_map(|op| match op {
            SemanticsOp::Upsert(node) if node.id == patch.root => Some(node),
            _ => None,
        })
        .expect("boot root geometry");
    assert_eq!(
        (root.rect.w, root.rect.h),
        (360.0, 640.0),
        "boot component tracks fill the viewport"
    );
    for (name, y) in [("button", 25.0), ("pressable", 75.0)] {
        app.pointer(PointerEvent::new(Phase::Move, 2.0, y, 1.0));
        for at in [16.0, 32.0] {
            app.pointer(PointerEvent::new(Phase::Down, 50.0, y, at));
            app.pointer(PointerEvent::new(Phase::Up, 50.0, y, at + 1.0));
            frame(app.as_mut(), at);
        }
        app.pointer(PointerEvent::wheel(50.0, y, 33.0, 0.0, 4.0));
        app.input(InputEvent::Composing(Some("x".to_owned())));
        app.input(InputEvent::Replace {
            text: "x".to_owned(),
            start: 1,
            end: 1,
            compose_start: -1,
            compose_end: -1,
        });
        app.input(InputEvent::Text(" ".to_owned()));
        frame(app.as_mut(), 48.0);
        app.input(InputEvent::Key(KeyPress::new(Key::Enter)));
        let disabled = frame(app.as_mut(), 64.0);
        let semantics = disabled
            .semantics
            .as_ref()
            .expect("disabled semantics patch");
        assert!(semantics.ops.iter().any(|op| matches!(op,
            SemanticsOp::Upsert(node) if node.label == name && node.flags.contains(SemanticsFlags::DISABLED)
                && !node.flags.contains(SemanticsFlags::FOCUSABLE))), "disabled {name}: {semantics:?}");
        app.pointer(PointerEvent::new(Phase::Down, 50.0, y, 65.0));
        app.pointer(PointerEvent::new(Phase::Up, 50.0, y, 66.0));
        frame(app.as_mut(), 80.0);
        app.pointer(PointerEvent::new(Phase::Move, 300.0, 400.0, 81.0));
        let observed = events.borrow();
        assert_eq!(
            observed
                .iter()
                .filter(|value| value.as_str() == format!("{name}:activate"))
                .count(),
            4,
            "disabled {name} activated again: {observed:?}"
        );
        for event in [
            "old-press",
            "new-press",
            "release",
            "focus",
            "blur",
            "hover",
            "leave",
            "composing",
            "replace",
            "text: ",
            "key:Enter",
        ] {
            assert!(
                observed.contains(&format!("{name}:{event}")),
                "missing {name}:{event}: {observed:?}"
            );
        }
    }
}

#[test]
fn component_environment_inherits_direction_tokens_and_clears_semantics() {
    let mut app = fixture("component-environment", Box::new(|_, _| {}));
    let initial = frame(app.as_mut(), 0.0);
    let labeled = initial
        .semantics
        .as_ref()
        .expect("initial semantics")
        .ops
        .iter()
        .find_map(|op| match op {
            SemanticsOp::Upsert(node) if node.label == "removable" => Some(node.id),
            _ => None,
        })
        .expect("view with author semantics");
    for (locale, dark, high_contrast, expected) in [
        ("ar", true, false, [0xF4, 0xF7, 0xF5, 255]),
        ("ar", true, true, [255, 255, 255, 255]),
        ("en", false, false, [0x14, 0x20, 0x1A, 255]),
    ] {
        app.settings(&session::PlatformSettings {
            locale: locale.to_owned(),
            brightness: if dark {
                session::Brightness::Dark
            } else {
                session::Brightness::Light
            },
            bold_text: dark,
            high_contrast,
            ..session::PlatformSettings::default()
        });
        let output = frame(app.as_mut(), 16.0);
        for text in ["class-token", "object-token"] {
            let color = commands(&output).find_map(|command| match command {
                Command::Text { utf8, color, .. } if utf8.as_ref() == text => Some(*color),
                _ => None,
            });
            assert_eq!(
                color,
                Some(expected),
                "{text} with {locale}, dark={dark}, contrast={high_contrast}"
            );
        }
        if locale == "ar" && !high_contrast {
            assert!(output.semantics.as_ref().expect("updated semantics").ops.iter().any(|op| matches!(op,
                SemanticsOp::Upsert(node) if node.id == labeled && node.label.is_empty() && node.role == uicore::SemanticsRole::Generic
                    && node.description.is_none())), "removed author label, role and hint must clear");
        }
    }
}

#[test]
fn component_overlays_react_and_revoke_refs_after_exit() {
    let complete = Rc::new(RefCell::new(false));
    let sink = Rc::clone(&complete);
    let mut app = fixture(
        "component-overlays",
        Box::new(move |_, text| {
            if text == "overlays:complete" {
                *sink.borrow_mut() = true;
            }
        }),
    );
    for step in 0..=250 {
        let output = frame(app.as_mut(), f64::from(step) * 16.0);
        if step == 2 {
            for (text, expected) in [
                ("dialog-title", 36.0),
                ("sheet-title", 36.0),
                ("toast-label", 30.0),
                ("tooltip-label", 36.0),
            ] {
                let size = commands(&output).find_map(|command| match command {
                    Command::Text {
                        utf8, font_size, ..
                    } if utf8.as_ref() == text => Some(*font_size),
                    _ => None,
                });
                assert_eq!(size, Some(expected), "overlay author typography for {text}");
            }
        }
    }
    assert!(
        *complete.borrow(),
        "overlay lifecycle verifier did not finish"
    );
}
