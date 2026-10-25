//! QuickJS session. The interpreter calls `core` directly. A missing source tree
//! compiles this crate as [`Error::NotLinked`].

use ui_core::Shaper;

mod error;

#[cfg(quickjs)]
mod linked;

#[cfg(quickjs)]
pub use linked::JsSession;

pub use error::Error;

/// Loads `script` and returns the session the host pumps each frame.
///
/// `name` is the filename the interpreter reports for this script. `log` receives
/// console lines. Level 3 is an error. Other levels are informational.
pub fn open(
    script: &str,
    name: &str,
    shaper: impl Shaper + 'static,
    log: Box<dyn Fn(u8, &str)>,
) -> Result<Box<dyn session::Session>, Error> {
    #[cfg(quickjs_missing)]
    {
        let _ = (script, name, shaper, log);
        Err(Error::NotLinked)
    }
    #[cfg(quickjs)]
    {
        Ok(Box::new(JsSession::open(script, name, shaper, log)?))
    }
}

#[cfg(test)]
mod tests {
    use protocol::Command;
    use session::{InputEvent, Key, Metrics, Session};
    use ui_core::{Phase, PointerEvent};

    use super::{Error, open};

    fn no_log() -> Box<dyn Fn(u8, &str)> {
        Box::new(|_, _| {})
    }

    fn metrics() -> Metrics {
        Metrics::new(320.0, 180.0, 1.0)
    }

    fn texts(list: &protocol::DrawList) -> Vec<&str> {
        list.commands
            .iter()
            .filter_map(|command| match command {
                Command::Text { utf8, .. } => Some(utf8.as_ref()),
                _ => None,
            })
            .collect()
    }

    fn text_spot(list: &protocol::DrawList, utf8: &str) -> (f32, f32) {
        for command in &list.commands {
            if let Command::Text {
                utf8: text, x, y, ..
            } = command
                && text.as_ref() == utf8
            {
                return (*x + 1.0, *y + 1.0);
            }
        }
        panic!("missing text {utf8}");
    }

    fn click(session: &mut dyn Session, x: f32, y: f32) {
        session.pointer(PointerEvent::new(Phase::Down, x, y, 0.0));
        session.pointer(PointerEvent::new(Phase::Up, x, y, 0.0));
    }

    fn open_counter() -> Option<Box<dyn Session>> {
        let source = include_str!("../../../apps/gallery-solid/dist/counter.js");
        match open(source, "test.js", ui_core::MonoShaper, no_log()) {
            Ok(session) => Some(session),
            Err(Error::NotLinked) => None,
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn minus_button_rewrites_the_count() {
        let Some(mut session) = open_counter() else {
            return;
        };
        session.metrics(&metrics());
        let first = session.frame(session::FrameTime::at(0.0));
        assert!(texts(&first).contains(&"还是旧的"));
        let (x, y) = text_spot(&first, "微任务改字");
        click(session.as_mut(), x, y);
        let second = session.frame(session::FrameTime::at(16.0));
        assert!(texts(&second).contains(&"已经改了"));
        assert!(!texts(&second).contains(&"还是旧的"));
    }

    #[test]
    fn repeated_clicks_leave_no_pending_jobs() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = include_str!("../../../apps/gallery-solid/dist/counter.js");
            let mut session =
                match super::JsSession::open(source, "test.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            assert!(!session.jobs_pending());
            let (x, y) = text_spot(&first, "微任务改字");
            click(&mut session, x, y);
            let second = session.frame(session::FrameTime::at(16.0));
            click(&mut session, x, y);
            let third = session.frame(session::FrameTime::at(32.0));
            assert!(texts(&second).contains(&"已经改了"));
            assert!(texts(&third).contains(&"已经改了"));
            assert!(!session.jobs_pending());
        }
    }

    #[test]
    fn remove_discards_the_tap_and_the_id() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let kept = r#"
                const root = core.column(0);
                const region = core.pointerRegion();
                const label = core.text("Go", 16, [255, 255, 255, 255]);
                core.append(region, label);
                core.append(root, region);
                core.onClick(region, () => {});
                core.remove(root, region);
            "#;
            let session =
                match super::JsSession::open(kept, "test.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            assert_eq!(session.tap_count(), 0);
            let dropped = r#"
                const root = core.column(0);
                const label = core.text("Go", 16, [255, 255, 255, 255]);
                core.append(root, label);
                core.remove(root, label);
                core.setText(label, "no");
            "#;
            match super::JsSession::open(dropped, "test.js", ui_core::MonoShaper, no_log()) {
                Err(Error::Script(message)) => {
                    assert!(message.contains("setText"), "script error was {message:?}");
                }
                Err(error) => panic!("{error}"),
                Ok(_) => panic!("discarded id still accepted setText"),
            }
        }
    }

    #[test]
    fn plus_button_rewrites_the_count() {
        let Some(mut session) = open_counter() else {
            return;
        };
        session.metrics(&metrics());
        let first = session.frame(session::FrameTime::at(0.0));
        let (x, y) = text_spot(&first, "导航");
        click(session.as_mut(), x, y);
        let second = session.frame(session::FrameTime::at(16.0));
        assert!(texts(&second).contains(&"第 1 页"));
        assert!(!texts(&second).contains(&"还是旧的"));
    }

    #[test]
    fn press_move_out_up_does_not_fire() {
        let Some(mut session) = open_counter() else {
            return;
        };
        session.metrics(&metrics());
        let first = session.frame(session::FrameTime::at(0.0));
        let (x, y) = text_spot(&first, "微任务改字");
        session.pointer(PointerEvent::new(Phase::Down, x, y, 0.0));
        session.pointer(PointerEvent::new(Phase::Move, 300.0, 170.0, 0.0));
        session.pointer(PointerEvent::new(Phase::Up, 300.0, 170.0, 0.0));
        let second = session.frame(session::FrameTime::at(16.0));
        assert!(texts(&second).contains(&"还是旧的"));
        assert!(!texts(&second).contains(&"已经改了"));
    }

    #[test]
    fn press_then_release_then_click_rewrites_the_label() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                let log = "";
                const root = core.column(0);
                const region = core.pointerRegion();
                const label = core.text("idle", 16, [0, 0, 0, 255]);
                core.append(region, label);
                core.append(root, region);
                const note = (mark) => { log += mark; core.setText(label, log); };
                core.onPress(region, () => note("P"));
                core.onRelease(region, () => note("R"));
                core.onClick(region, () => note("C"));
            "#;
            let mut session =
                match super::JsSession::open(source, "test.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "idle");
            session.pointer(PointerEvent::new(Phase::Down, x, y, 0.0));
            let pressed = session.frame(session::FrameTime::at(16.0));
            assert_eq!(texts(&pressed), ["P"]);
            session.pointer(PointerEvent::new(Phase::Up, x, y, 0.0));
            let lifted = session.frame(session::FrameTime::at(32.0));
            assert_eq!(texts(&lifted), ["PRC"]);
        }
    }

    #[test]
    fn insets_shift_the_recorded_text() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const label = core.text("Hi", 16, [0, 0, 0, 255]);
            "#;
            let mut session =
                match super::JsSession::open(source, "test.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            let mut window = metrics();
            window.inset_left = 8.0;
            window.inset_top = 14.0;
            window.width = 200.0;
            window.height = 120.0;
            session.metrics(&window);
            let frame = session.frame(session::FrameTime::at(0.0));
            assert_eq!(text_spot(&frame, "Hi"), (9.0, 15.0));
        }
    }

    #[test]
    fn syntax_error_script_is_reported() {
        match open("{", "test.js", ui_core::MonoShaper, no_log()) {
            Err(Error::NotLinked | Error::Script(_)) => {}
            Err(error) => panic!("{error}"),
            Ok(_) => panic!("expected a script error"),
        }
    }

    #[test]
    fn an_infinite_frame_callback_keeps_the_previous_list() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const root = core.column(0);
                let ticks = 0;
                core.onFrame(() => {
                    ticks += 1;
                    if (ticks > 1) {
                        while (true) {}
                    }
                });
            "#;
            let mut session =
                match super::JsSession::open(source, "test.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            assert!(session.take_script_error().is_none());
            let second = session.frame(session::FrameTime::at(16.0));
            assert_eq!(second, first);
            assert!(session.take_script_error().is_some());
        }
    }

    #[test]
    fn listener_error_is_kept_as_script_error() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const root = core.column(0);
                const region = core.pointerRegion();
                const label = core.text("Go", 16, [255, 255, 255, 255]);
                core.append(region, label);
                core.append(root, region);
                core.onClick(region, () => { throw new Error("boom"); });
            "#;
            let mut session =
                match super::JsSession::open(source, "test.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "Go");
            click(&mut session, x, y);
            let message = session.take_script_error().unwrap_or_default();
            assert!(message.contains("boom"), "script error was {message:?}");
            let second = session.frame(session::FrameTime::at(16.0));
            assert!(texts(&second).contains(&"Go"));
        }
    }

    #[test]
    fn a_click_longer_than_one_frame_still_updates_the_label() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const root = core.column(0);
                const region = core.pointerRegion();
                const label = core.text("Go", 16, [255, 255, 255, 255]);
                core.append(region, label);
                core.append(root, region);
                core.onClick(region, () => {
                    const start = Date.now();
                    while (Date.now() - start < 40) {}
                    core.setText(label, "Done");
                });
            "#;
            let mut session =
                match super::JsSession::open(source, "test.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "Go");
            click(&mut session, x, y);
            assert!(
                session.take_script_error().is_none(),
                "a 40 ms turn was interrupted"
            );
            let second = session.frame(session::FrameTime::at(16.0));
            assert!(texts(&second).contains(&"Done"));
        }
    }

    #[test]
    fn a_runaway_click_is_interrupted() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const root = core.column(0);
                const region = core.pointerRegion();
                const label = core.text("Go", 16, [255, 255, 255, 255]);
                core.append(region, label);
                core.append(root, region);
                core.onClick(region, () => {
                    const start = Date.now();
                    while (Date.now() - start < 400) {}
                    core.setText(label, "Done");
                });
            "#;
            let mut session =
                match super::JsSession::open(source, "test.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "Go");
            click(&mut session, x, y);
            let message = session.take_script_error().unwrap_or_default();
            assert!(
                message.contains("interrupted"),
                "script error was {message:?}"
            );
            let second = session.frame(session::FrameTime::at(16.0));
            assert!(texts(&second).contains(&"Go"));
        }
    }

    #[test]
    fn console_log_reaches_the_host() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
            let record = std::rc::Rc::clone(&seen);
            let opened = super::JsSession::open(
                "console.log('boot')",
                "test.js",
                ui_core::MonoShaper,
                Box::new(move |level, text| {
                    *record.borrow_mut() = Some((level, text.to_owned()));
                }),
            );
            let Ok(_session) = opened else {
                panic!("console script");
            };
            let borrowed = seen.borrow();
            let recorded = borrowed
                .as_ref()
                .map(|(level, text)| (*level, text.as_str()));
            assert_eq!(recorded, Some((0, "boot")));
        }
    }

    #[test]
    fn request_frame_lasts_for_one_extra_frame() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const label = core.text("0", 16, [0, 0, 0, 255]);
                let n = 0;
                core.onFrame(() => {
                    n += 1;
                    core.setText(label, String(n));
                    if (n < 2) core.requestFrame();
                });
            "#;
            let mut session =
                match super::JsSession::open(source, "test.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            assert_eq!(texts(&first), ["1"]);
            assert!(session.wants_frame());
            let second = session.frame(session::FrameTime::at(16.0));
            assert_eq!(texts(&second), ["2"]);
            assert!(!session.wants_frame());
        }
    }

    #[test]
    fn microtask_set_text_paints_without_request_frame() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const label = core.text("old", 16, [0, 0, 0, 255]);
                const region = core.pointerRegion();
                core.append(region, label);
                core.onClick(region, () => {
                    queueMicrotask(() => core.setText(label, "new"));
                });
            "#;
            let mut session =
                match super::JsSession::open(source, "test.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "old");
            click(&mut session, x, y);
            let second = session.frame(session::FrameTime::at(16.0));
            assert!(texts(&second).contains(&"new"), "{:?}", texts(&second));
            assert!(!session.wants_frame());
        }
    }

    #[test]
    fn a_throwing_frame_callback_does_not_ask_for_another_frame() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const label = core.text("stay", 16, [0, 0, 0, 255]);
                core.onFrame(() => {
                    core.requestFrame();
                    core.setText(label, "nope");
                    throw new Error("boom");
                });
            "#;
            let mut session =
                match super::JsSession::open(source, "test.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            assert!(session.take_script_error().is_some());
            assert!(texts(&first).iter().all(|text| *text != "nope"));
            assert!(!session.wants_frame());
        }
    }

    #[test]
    fn a_drag_past_the_slop_does_not_click() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                let log = "";
                const root = core.column(0);
                core.setFill(root);
                const scroll = core.scroll();
                const region = core.pointerRegion();
                const label = core.text("idle", 16, [0, 0, 0, 255]);
                core.append(region, label);
                core.append(scroll, region);
                const tall = core.sized(20, 400);
                core.append(scroll, tall);
                core.append(root, scroll);
                const note = (mark) => { log += mark; core.setText(label, log); };
                core.onDragBegin(scroll, () => note("B"));
                core.onDragBy(scroll, () => note("M"));
                core.onDragEnd(scroll, () => note("E"));
                core.onClick(region, () => note("C"));
            "#;
            let mut session =
                match super::JsSession::open(source, "test.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "idle");
            session.pointer(PointerEvent::new(Phase::Down, x, y, 0.0));
            session.pointer(PointerEvent::new(Phase::Move, x, y + 4.0, 10.0));
            session.pointer(PointerEvent::new(Phase::Up, x, y + 4.0, 20.0));
            let clicked = session.frame(session::FrameTime::at(16.0));
            assert_eq!(texts(&clicked), ["C"]);
            session.pointer(PointerEvent::new(Phase::Down, x, y, 30.0));
            session.pointer(PointerEvent::new(Phase::Move, x, y + 24.0, 40.0));
            session.pointer(PointerEvent::new(Phase::Up, x, y + 24.0, 50.0));
            let dragged = session.frame(session::FrameTime::at(32.0));
            assert_eq!(texts(&dragged), ["CBME"]);
        }
    }

    #[test]
    fn dragging_the_list_translates_a_visible_row() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = include_str!("../../../apps/gallery-solid/dist/counter.js");
            let mut session =
                match super::JsSession::open(source, "test.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            let mut window = metrics();
            window.height = 800.0;
            session.metrics(&window);
            let mut first = session.frame(session::FrameTime::at(0.0));
            if !texts(&first).iter().any(|text| text.starts_with("Item ")) {
                let (x, y) = text_spot(&first, "列表");
                click(&mut session, x, y);
                first = session.frame(session::FrameTime::at(8.0));
            }
            let Some((clip_y, clip_h)) =
                first
                    .commands
                    .iter()
                    .rev()
                    .find_map(|command| match command {
                        Command::PushClip { y, h, .. } if *h > 100.0 => Some((*y, *h)),
                        _ => None,
                    })
            else {
                panic!("scroll clip");
            };
            let Some((x, before)) = first
                .commands
                .iter()
                .rev()
                .find_map(|command| match command {
                    Command::Text { utf8, x, y, .. }
                        if utf8.starts_with("Item ") && *y >= clip_y && *y < clip_y + clip_h =>
                    {
                        Some((*x + 1.0, *y))
                    }
                    _ => None,
                })
            else {
                panic!("a row inside the viewport");
            };
            let Some(label) = first.commands.iter().find_map(|command| match command {
                Command::Text { utf8, y, .. } if (*y - before).abs() < 0.5 => Some(utf8.clone()),
                _ => None,
            }) else {
                panic!("label");
            };
            let delta = 12.0;
            session.pointer(PointerEvent::new(Phase::Down, x, before + 1.0, 0.0));
            session.pointer(PointerEvent::new(
                Phase::Move,
                x,
                before + 1.0 - delta,
                16.0,
            ));
            let scrolled = session.frame(session::FrameTime::at(16.0));
            let Some(after) = scrolled.commands.iter().find_map(|command| match command {
                Command::Text { utf8, y, .. } if utf8.as_ref() == label.as_ref() => Some(*y),
                _ => None,
            }) else {
                panic!("scrolled label");
            };
            assert!(
                (before - after - delta).abs() < 0.5,
                "{label} moved from {before} to {after}"
            );
            assert!(texts(&scrolled).contains(&"列表"));
        }
    }

    #[test]
    fn text_input_follows_focus_and_clears_when_the_node_is_removed() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                let value = "";
                let composing = "";
                const root = core.column(0);
                const field = core.pointerRegion();
                const elsewhere = core.pointerRegion();
                const label = core.text("field", 16, [255, 255, 255, 255]);
                const other = core.sized(40, 20);
                core.setFocusable(field, 1);
                core.append(field, label);
                core.append(elsewhere, other);
                core.append(root, field);
                core.append(root, elsewhere);
                function paint() {
                    core.setText(label, value + composing);
                }
                core.onFocus(field, () => {
                    core.setImeEnabled(true);
                    core.setImeText(value);
                });
                core.onBlur(field, () => {
                    core.setImeEnabled(false);
                });
                core.onText(field, (text) => {
                    if (text === "drop") {
                        core.remove(root, field);
                        return;
                    }
                    value += text;
                    paint();
                });
                core.onComposing(field, (text) => {
                    composing = text == null ? "" : text;
                    paint();
                });
                core.onKey(field, (key) => {
                    if (key === "Backspace") {
                        value = Array.from(value).slice(0, -1).join("");
                        core.setImeText(value);
                        paint();
                    }
                });
            "#;
            let mut session =
                match super::JsSession::open(source, "test.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            assert!(texts(&first).contains(&"field"));
            session.input(InputEvent::Text("你".to_string()));
            let ignored = session.frame(session::FrameTime::at(16.0));
            assert!(texts(&ignored).contains(&"field"));
            assert!(!texts(&ignored).contains(&"你"));
            assert!(session.take_ime().is_none());
            let (x, y) = text_spot(&ignored, "field");
            click(&mut session, x, y);
            match session.take_ime() {
                Some(session::ImeUpdate::Show(state)) => assert_eq!(state.text, ""),
                other => panic!("expected show, got {other:?}"),
            }
            session.input(InputEvent::Text("你".to_string()));
            session.input(InputEvent::Composing(Some("ni".to_string())));
            let composed = session.frame(session::FrameTime::at(32.0));
            assert!(texts(&composed).contains(&"你ni"));
            session.input(InputEvent::Composing(None));
            session.input(InputEvent::Key(session::KeyPress::new(Key::Backspace)));
            let cleared = session.frame(session::FrameTime::at(48.0));
            assert!(!texts(&cleared).contains(&"你"));
            assert!(!texts(&cleared).contains(&"你ni"));
            match session.take_ime() {
                Some(session::ImeUpdate::Update(state)) => assert_eq!(state.text, ""),
                other => panic!("expected update, got {other:?}"),
            }
            session.input(InputEvent::Text("drop".to_string()));
            let _ = session.frame(session::FrameTime::at(64.0));
            assert_eq!(session.tap_count(), 0);
            assert!(matches!(session.take_ime(), Some(session::ImeUpdate::Hide)));
            session.input(InputEvent::Text("no".to_string()));
            let after = session.frame(session::FrameTime::at(80.0));
            assert!(!texts(&after).contains(&"no"));
        }
    }

    #[test]
    fn dismissing_the_keyboard_lets_blur_clear_ime_state() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const root = core.column(0);
                const field = core.pointerRegion();
                const label = core.text("field", 16, [255, 255, 255, 255]);
                core.setFocusable(field, 1);
                core.append(field, label);
                core.append(root, field);
                core.onFocus(field, () => {
                    core.setImeState("hi", 2, 2, -1, -1, 0, 0, 1, 16);
                });
                core.onBlur(field, () => {
                    core.setImeState(null);
                });
            "#;
            let mut session =
                match super::JsSession::open(source, "test.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "field");
            click(&mut session, x, y);
            assert!(matches!(
                session.take_ime(),
                Some(session::ImeUpdate::Show(_))
            ));
            session.ime_dismissed();
            assert!(matches!(session.take_ime(), Some(session::ImeUpdate::Hide)));
            assert!(session.take_script_error().is_none());
        }
    }

    #[test]
    fn timers_fetch_and_modules_run_inside_the_frame() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            use std::sync::{Arc, Mutex};

            struct FixedHttp;

            impl session::HttpClient for FixedHttp {
                fn perform(
                    &self,
                    request: &session::HttpRequest,
                ) -> Result<session::HttpResponse, session::HttpError> {
                    assert_eq!(request.method, "POST");
                    assert_eq!(request.body, b"hi");
                    Ok(session::HttpResponse {
                        status: 200,
                        headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                        body: b"pong".to_vec(),
                    })
                }
            }

            let lines = Arc::new(Mutex::new(Vec::new()));
            let log_lines = Arc::clone(&lines);
            let log = Box::new(move |_level: u8, text: &str| {
                log_lines
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(text.to_owned());
            });
            let source = r#"
                console.log(new URL("https://example.test/a?q=1").searchParams.get("q"));
                console.log(String(new TextEncoder().encode("A").length));
                setTimeout(() => console.log("soon"), 0);
                setTimeout(() => console.log("later"), 20);
                const cleared = setTimeout(() => console.log("cleared"), 0);
                clearTimeout(cleared);
                requestAnimationFrame(() => console.log("raf"));
                fetch("https://example.test/a", { method: "POST", body: "hi" })
                    .then((response) => response.text())
                    .then((text) => console.log("body:" + text));
                const app = TurboModuleRegistry.getEnforcing("AppState");
                app.addEventListener("change", (state) => console.log("state:" + state));
                const clip = TurboModuleRegistry.getEnforcing("Clipboard");
                clip.setString("hi").then(() => clip.getString()).then((text) => console.log("clip:" + text));
                const socket = new WebSocket("ws://example.test/socket");
                socket.onerror = () => console.log("ws");
            "#;
            let mut session = match super::JsSession::open_with(
                source,
                "host.js",
                ui_core::MonoShaper,
                log,
                Some(Arc::new(FixedHttp)),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            let _ = session.frame(session::FrameTime::at(20.0));
            session.lifecycle(session::Lifecycle::Paused);
            let text = lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .join("\n");
            assert!(text.contains('1'), "{text}");
            assert!(text.contains("soon"), "{text}");
            assert!(text.contains("raf"), "{text}");
            assert!(text.contains("body:pong"), "{text}");
            assert!(text.contains("clip:hi"), "{text}");
            assert!(text.contains("ws"), "{text}");
            assert!(text.contains("state:background"), "{text}");
            assert!(!text.contains("cleared"), "{text}");
            assert!(text.contains("later"), "{text}");
        }
    }

    #[test]
    fn an_object_handle_is_rejected_and_a_switch_hint_sticks() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const region = core.pointerRegion();
                const label = core.text("idle", 16, [0, 0, 0, 255]);
                core.append(region, label);
                try {
                    core.setSemantics({ id: region }, 5, "", "On", 1);
                    core.setText(label, "accepted");
                } catch (error) {
                    const message = String(error);
                    core.setText(label, message.indexOf("bad handle") >= 0 ? "bad handle" : message);
                }
                core.setSemantics(region, 5, "Play", "On", 1);
            "#;
            let mut session =
                match super::JsSession::open(source, "semantics.js", ui_core::MonoShaper, no_log())
                {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            session.metrics(&metrics());
            let frame = session.frame(session::FrameTime::at(0.0));
            assert!(texts(&frame).contains(&"bad handle"), "{:?}", texts(&frame));
            let update = session.semantics().expect("semantics");
            let switch = update
                .nodes
                .iter()
                .find(|node| node.role == ui_core::SemanticsRole::Switch)
                .expect("switch");
            assert_eq!(switch.value.as_deref(), Some("On"));
            assert!(switch.flags.contains(ui_core::SemanticsFlags::CHECKED));
        }
    }

    #[test]
    fn settings_reach_the_script() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const label = core.text("boot", 16, [0, 0, 0, 255]);
                core.onSettings((next) => {
                    core.setText(
                        label,
                        next.locale + ":" + next.brightness + ":" + (next.highContrast ? "1" : "0"),
                    );
                });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "settings.js",
                ui_core::MonoShaper,
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.settings(&session::PlatformSettings {
                locale: "zh-CN".to_owned(),
                text_scale: 1.0,
                brightness: session::Brightness::Dark,
                reduce_motion: false,
                bold_text: true,
                high_contrast: true,
            });
            session.metrics(&metrics());
            let frame = session.frame(session::FrameTime::at(0.0));
            assert!(
                texts(&frame).contains(&"zh-CN:dark:1"),
                "{:?}",
                texts(&frame)
            );
        }
    }

    #[test]
    fn tab_moves_focus_and_escape_does_not_exit() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const column = core.column(0);
                const label = core.text("none", 16, [0, 0, 0, 255]);
                const first = core.pointerRegion();
                const second = core.pointerRegion();
                core.append(column, label);
                core.append(column, first);
                core.append(column, second);
                core.setFocusable(first, 1);
                core.setFocusable(second, 1);
                core.append(first, core.sized(20, 20));
                core.append(second, core.sized(20, 20));
                core.onFocus(first, () => core.setText(label, "first"));
                core.onFocus(second, () => core.setText(label, "second"));
                core.onKey(first, (key) => core.setText(label, "key:" + key));
                core.onBack(() => {
                    core.setText(label, "back");
                    return true;
                });
            "#;
            let mut session =
                match super::JsSession::open(source, "tab.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            session.input(InputEvent::Key(session::KeyPress::new(Key::Tab)));
            let next = session.frame(session::FrameTime::at(16.0));
            assert!(texts(&next).contains(&"first"), "{:?}", texts(&next));
            session.input(InputEvent::Key(session::KeyPress::new(Key::Tab)));
            let after = session.frame(session::FrameTime::at(32.0));
            assert!(texts(&after).contains(&"second"), "{:?}", texts(&after));
            session.input(InputEvent::Key(session::KeyPress::new(Key::Escape)));
            let closed = session.frame(session::FrameTime::at(48.0));
            assert!(texts(&closed).contains(&"back"), "{:?}", texts(&closed));
            assert!(!session.take_exit());
        }
    }

    #[test]
    fn text_replaced_delivers_the_loaded_string() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const root = core.column(0);
                const label = core.text("idle", 16, [0, 0, 0, 255]);
                const field = core.text("old", 16, [0, 0, 0, 255]);
                core.append(root, label);
                core.append(root, field);
                core.setSemantics(field, 6, "Name", "old", -1, -1, -1, 0, 1, 0, "");
                core.onTextReplaced(field, (value) => {
                    core.setText(label, "replaced:" + value);
                });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "replaced.js",
                ui_core::MonoShaper,
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            let update = session.semantics().expect("semantics");
            let field = update
                .nodes
                .iter()
                .find(|node| node.role == ui_core::SemanticsRole::TextField)
                .expect("field");
            session.semantics_action(ui_core::SemanticsAction::SetText {
                id: field.id,
                text: "Hello".to_owned(),
            });
            let frame = session.frame(session::FrameTime::at(16.0));
            assert!(
                texts(&frame).contains(&"replaced:Hello"),
                "{:?}",
                texts(&frame)
            );
        }
    }

    #[test]
    fn register_font_queues_replaces_and_caps_names() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const root = core.column(0);
                const label = core.text("boot", 16, [0, 0, 0, 255]);
                const empty = core.text("empty-pending", 16, [0, 0, 0, 255]);
                core.append(root, label);
                core.append(root, empty);
                core.registerFont("UIFWSans", new Uint8Array([1]));
                for (let i = 0; i < 8; i++) {
                    core.registerFont("F" + i, new Uint8Array([i + 1]));
                }
                core.registerFont("F0", new Uint8Array([9, 9]));
                try {
                    core.registerFont("Extra", new Uint8Array([3]));
                    core.setText(label, "accepted");
                } catch (error) {
                    const message = String(error);
                    core.setText(label, message.indexOf("registerFont failed") >= 0 ? "rejected" : message);
                }
                try {
                    core.registerFont("", new Uint8Array([4]));
                    core.setText(empty, "empty-accepted");
                } catch (error) {
                    const message = String(error);
                    core.setText(empty, message.indexOf("registerFont failed") >= 0 ? "empty-rejected" : message);
                }
                let phase = 0;
                core.onFrame(() => {
                    if (phase !== 0) {
                        return;
                    }
                    phase = 1;
                    try {
                        core.registerFont("Later", new Uint8Array([5]));
                        core.setText(label, "handed-accepted");
                    } catch (error) {
                        const message = String(error);
                        core.setText(
                            label,
                            message.indexOf("registerFont failed") >= 0 ? "handed-rejected" : message,
                        );
                    }
                    core.registerFont("F0", new Uint8Array([8]));
                });
            "#;
            let mut session =
                match super::JsSession::open(source, "fonts.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            session.metrics(&metrics());
            let queued = session.take_fonts();
            assert_eq!(queued.len(), 9, "{queued:?}");
            assert!(session.take_fonts().is_empty());
            let replaced = queued.iter().find(|(name, _)| name == "F0").expect("F0");
            assert_eq!(replaced.1, vec![9, 9]);
            assert!(queued.iter().any(|(name, _)| name == "UIFWSans"));
            assert!(queued.iter().all(|(name, _)| name != "Extra"));
            let listed = session.frame(session::FrameTime::at(0.0));
            assert!(
                texts(&listed).contains(&"handed-rejected"),
                "{:?}",
                texts(&listed)
            );
            assert!(
                texts(&listed).contains(&"empty-rejected"),
                "{:?}",
                texts(&listed)
            );
            let again = session.take_fonts();
            assert_eq!(again.len(), 1, "{again:?}");
            assert_eq!(again[0].0, "F0");
            assert_eq!(again[0].1, vec![8]);
        }
    }

    #[test]
    fn register_font_rejects_an_oversize_face_without_touching_the_queue() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const label = core.text("boot", 16, [0, 0, 0, 255]);
                core.registerFont("Keep", new Uint8Array([7]));
                const big = new Uint8Array(4 * 1024 * 1024 + 1);
                try {
                    core.registerFont("Big", big);
                    core.setText(label, "accepted");
                } catch (error) {
                    const message = String(error);
                    core.setText(label, message.indexOf("registerFont failed") >= 0 ? "rejected" : message);
                }
            "#;
            let mut session =
                match super::JsSession::open(source, "bigfont.js", ui_core::MonoShaper, no_log()) {
                    Ok(session) => session,
                    Err(error) => panic!("{error}"),
                };
            session.metrics(&metrics());
            let listed = session.frame(session::FrameTime::at(0.0));
            assert!(texts(&listed).contains(&"rejected"), "{:?}", texts(&listed));
            let queued = session.take_fonts();
            assert_eq!(queued.len(), 1);
            assert_eq!(queued[0].0, "Keep");
            assert_eq!(queued[0].1, vec![7]);
        }
    }

    #[test]
    fn reload_queues_a_source_and_rejects_an_empty_one() {
        let source = r#"
            const root = core.column(0);
            const label = core.text("boot", 16, [0, 0, 0, 255]);
            core.append(root, label);
            core.reload("asset:logo");
            try {
                core.reload("");
                core.setText(label, "accepted");
            } catch (error) {
                const message = String(error);
                core.setText(label, message.indexOf("reload(source)") >= 0 ? "rejected" : message);
            }
        "#;
        let mut session = match open(source, "reload.js", ui_core::MonoShaper, no_log()) {
            Ok(session) => session,
            Err(Error::NotLinked) => return,
            Err(error) => panic!("{error}"),
        };
        let queued = session.take_reloads();
        assert_eq!(queued, vec!["asset:logo".to_owned()]);
        assert!(session.take_reloads().is_empty());
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(texts(&frame).contains(&"rejected"), "{:?}", texts(&frame));
    }

    #[test]
    fn axis_percent_sizes_the_child_and_fill_is_rejected() {
        let source = r#"
            const parent = core.column(0);
            const child = core.decorated([0, 9, 0, 255]);
            const label = core.text("boot", 16, [0, 0, 0, 255]);
            core.append(parent, child);
            core.append(parent, label);
            core.setAxisSize(parent, 200, 80);
            core.setAxisSize(child, "50%", 20);
            try {
                core.setAxisSize(child, "fill", 20);
                core.setText(label, "accepted");
            } catch (error) {
                const message = String(error);
                core.setText(label, message.indexOf("bad axis") >= 0 ? "bad-axis" : message);
            }
        "#;
        let mut session = match open(source, "axis.js", ui_core::MonoShaper, no_log()) {
            Ok(session) => session,
            Err(Error::NotLinked) => return,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(texts(&frame).contains(&"bad-axis"), "{:?}", texts(&frame));
        let child = frame.commands.iter().find_map(|command| match command {
            Command::Rect { w, h, color, .. } if *color == [0, 9, 0, 255] => Some((*w, *h)),
            _ => None,
        });
        assert_eq!(child, Some((100.0, 20.0)), "{child:?}");
    }

    #[test]
    fn image_mode_is_recorded_and_a_bad_mode_is_rejected() {
        let source = r#"
            const root = core.column(0);
            const picture = core.image("asset:logo", 40, 20, "fit", false);
            const label = core.text("boot", 16, [0, 0, 0, 255]);
            core.append(root, picture);
            core.append(root, label);
            core.setImageMode(picture, "crop", true);
            try {
                core.setImageMode(picture, "stretch", false);
                core.setText(label, "accepted");
            } catch (error) {
                const message = String(error);
                core.setText(label, message.indexOf("bad image") >= 0 ? "bad-image" : message);
            }
        "#;
        let mut session = match open(source, "image.js", ui_core::MonoShaper, no_log()) {
            Ok(session) => session,
            Err(Error::NotLinked) => return,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(texts(&frame).contains(&"bad-image"), "{:?}", texts(&frame));
        let image = frame.commands.iter().find_map(|command| match command {
            Command::Image { mode, required, .. } => Some((*mode, *required)),
            _ => None,
        });
        assert_eq!(
            image,
            Some((protocol::ContentMode::Crop, true)),
            "{image:?}"
        );
    }
}
