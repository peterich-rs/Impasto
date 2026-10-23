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
/// `log` receives console lines. Level 3 is an error. Other levels are informational.
pub fn open(
    script: &str,
    shaper: impl Shaper + 'static,
    log: Box<dyn Fn(u8, &str)>,
) -> Result<Box<dyn session::Session>, Error> {
    #[cfg(quickjs_missing)]
    {
        let _ = (script, shaper, log);
        Err(Error::NotLinked)
    }
    #[cfg(quickjs)]
    {
        Ok(Box::new(JsSession::open(script, shaper, log)?))
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
        Metrics {
            width: 320.0,
            height: 180.0,
            device_pixel_ratio: 1.0,
            inset_left: 0.0,
            inset_top: 0.0,
            inset_right: 0.0,
            inset_bottom: 0.0,
        }
    }

    fn texts(list: &protocol::DrawList) -> Vec<&str> {
        list.commands
            .iter()
            .filter_map(|command| match command {
                Command::Text { utf8, .. } => Some(utf8.as_str()),
                _ => None,
            })
            .collect()
    }

    fn text_spot(list: &protocol::DrawList, utf8: &str) -> (f32, f32) {
        for command in &list.commands {
            if let Command::Text {
                utf8: text, x, y, ..
            } = command
                && text == utf8
            {
                return (*x + 1.0, *y + 1.0);
            }
        }
        panic!("missing text {utf8}");
    }

    fn click(session: &mut dyn Session, x: f32, y: f32) {
        session.pointer(PointerEvent {
            phase: Phase::Down,
            x,
            y,
            time_ms: 0.0,
        });
        session.pointer(PointerEvent {
            phase: Phase::Up,
            x,
            y,
            time_ms: 0.0,
        });
    }

    fn open_counter() -> Option<Box<dyn Session>> {
        let source = include_str!("../../../apps/gallery-solid/dist/counter.js");
        match open(source, ui_core::MonoShaper, no_log()) {
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
        let first = session.frame(0.0);
        assert!(texts(&first).contains(&"Count: 0"));
        let (x, y) = text_spot(&first, "-");
        click(session.as_mut(), x, y);
        let second = session.frame(16.0);
        assert!(texts(&second).contains(&"Count: -1"));
        assert!(!texts(&second).contains(&"Count: 0"));
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
            let mut session = match super::JsSession::open(source, ui_core::MonoShaper, no_log()) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(0.0);
            assert!(!session.jobs_pending());
            let (x, y) = text_spot(&first, "-");
            click(&mut session, x, y);
            let second = session.frame(16.0);
            click(&mut session, x, y);
            let third = session.frame(32.0);
            assert!(texts(&second).contains(&"Count: -1"));
            assert!(texts(&third).contains(&"Count: -2"));
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
            let session = match super::JsSession::open(kept, ui_core::MonoShaper, no_log()) {
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
            match super::JsSession::open(dropped, ui_core::MonoShaper, no_log()) {
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
        let first = session.frame(0.0);
        let (x, y) = text_spot(&first, "+");
        click(session.as_mut(), x, y);
        let second = session.frame(16.0);
        assert!(texts(&second).contains(&"Count: 1"));
        assert!(!texts(&second).contains(&"Count: 0"));
    }

    #[test]
    fn press_move_out_up_does_not_fire() {
        let Some(mut session) = open_counter() else {
            return;
        };
        session.metrics(&metrics());
        let first = session.frame(0.0);
        let (x, y) = text_spot(&first, "-");
        session.pointer(PointerEvent {
            phase: Phase::Down,
            x,
            y,
            time_ms: 0.0,
        });
        session.pointer(PointerEvent {
            phase: Phase::Move,
            x: 300.0,
            y: 170.0,
            time_ms: 0.0,
        });
        session.pointer(PointerEvent {
            phase: Phase::Up,
            x: 300.0,
            y: 170.0,
            time_ms: 0.0,
        });
        let second = session.frame(16.0);
        assert!(texts(&second).contains(&"Count: 0"));
        assert!(!texts(&second).contains(&"Count: -1"));
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
            let mut session = match super::JsSession::open(source, ui_core::MonoShaper, no_log()) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(0.0);
            let (x, y) = text_spot(&first, "idle");
            session.pointer(PointerEvent {
                phase: Phase::Down,
                x,
                y,
                time_ms: 0.0,
            });
            let pressed = session.frame(16.0);
            assert_eq!(texts(&pressed), ["P"]);
            session.pointer(PointerEvent {
                phase: Phase::Up,
                x,
                y,
                time_ms: 0.0,
            });
            let lifted = session.frame(32.0);
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
            let mut session = match super::JsSession::open(source, ui_core::MonoShaper, no_log()) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            let mut window = metrics();
            window.inset_left = 8.0;
            window.inset_top = 14.0;
            window.width = 200.0;
            window.height = 120.0;
            session.metrics(&window);
            let frame = session.frame(0.0);
            assert_eq!(text_spot(&frame, "Hi"), (9.0, 15.0));
        }
    }

    #[test]
    fn syntax_error_script_is_reported() {
        match open("{", ui_core::MonoShaper, no_log()) {
            Err(Error::NotLinked | Error::Script(_)) => {}
            Err(error) => panic!("{error}"),
            Ok(_) => panic!("expected a script error"),
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
            let mut session = match super::JsSession::open(source, ui_core::MonoShaper, no_log()) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(0.0);
            let (x, y) = text_spot(&first, "Go");
            click(&mut session, x, y);
            let message = session.take_script_error().unwrap_or_default();
            assert!(message.contains("boom"), "script error was {message:?}");
            let second = session.frame(16.0);
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
            let mut session = match super::JsSession::open(source, ui_core::MonoShaper, no_log()) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(0.0);
            assert_eq!(texts(&first), ["1"]);
            assert!(session.wants_frame());
            let second = session.frame(16.0);
            assert_eq!(texts(&second), ["2"]);
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
                core.append(root, scroll);
                const note = (mark) => { log += mark; core.setText(label, log); };
                core.onDragBegin(scroll, () => note("B"));
                core.onDragBy(scroll, () => note("M"));
                core.onDragEnd(scroll, () => note("E"));
                core.onClick(region, () => note("C"));
            "#;
            let mut session = match super::JsSession::open(source, ui_core::MonoShaper, no_log()) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(0.0);
            let (x, y) = text_spot(&first, "idle");
            session.pointer(PointerEvent {
                phase: Phase::Down,
                x,
                y,
                time_ms: 0.0,
            });
            session.pointer(PointerEvent {
                phase: Phase::Move,
                x,
                y: y + 4.0,
                time_ms: 10.0,
            });
            session.pointer(PointerEvent {
                phase: Phase::Up,
                x,
                y: y + 4.0,
                time_ms: 20.0,
            });
            let clicked = session.frame(16.0);
            assert_eq!(texts(&clicked), ["C"]);
            session.pointer(PointerEvent {
                phase: Phase::Down,
                x,
                y,
                time_ms: 30.0,
            });
            session.pointer(PointerEvent {
                phase: Phase::Move,
                x,
                y: y + 24.0,
                time_ms: 40.0,
            });
            session.pointer(PointerEvent {
                phase: Phase::Up,
                x,
                y: y + 24.0,
                time_ms: 50.0,
            });
            let dragged = session.frame(32.0);
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
            let mut session = match super::JsSession::open(source, ui_core::MonoShaper, no_log()) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(0.0);
            let Some((clip_y, clip_h)) = first.commands.iter().find_map(|command| match command {
                Command::PushClip { y, h, .. } => Some((*y, *h)),
                _ => None,
            }) else {
                panic!("scroll clip");
            };
            let Some((x, before)) = first.commands.iter().find_map(|command| match command {
                Command::Text { utf8, x, y, .. }
                    if utf8.starts_with("Item ") && *y >= clip_y && *y < clip_y + clip_h =>
                {
                    Some((*x + 1.0, *y))
                }
                _ => None,
            }) else {
                panic!("a row inside the viewport");
            };
            let Some(label) = first.commands.iter().find_map(|command| match command {
                Command::Text { utf8, y, .. } if (*y - before).abs() < 0.5 => Some(utf8.clone()),
                _ => None,
            }) else {
                panic!("label");
            };
            session.pointer(PointerEvent {
                phase: Phase::Down,
                x,
                y: before + 1.0,
                time_ms: 0.0,
            });
            session.pointer(PointerEvent {
                phase: Phase::Move,
                x,
                y: before - 39.0,
                time_ms: 16.0,
            });
            let scrolled = session.frame(16.0);
            let Some(after) = scrolled.commands.iter().find_map(|command| match command {
                Command::Text { utf8, y, .. } if utf8 == &label => Some(*y),
                _ => None,
            }) else {
                panic!("scrolled label");
            };
            assert!(
                (before - after - 40.0).abs() < 0.5,
                "{label} moved from {before} to {after}"
            );
            assert!(texts(&scrolled).contains(&"Count: 0"));
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
            let mut session = match super::JsSession::open(source, ui_core::MonoShaper, no_log()) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(0.0);
            assert!(texts(&first).contains(&"field"));
            session.input(InputEvent::Text("你".to_string()));
            let ignored = session.frame(16.0);
            assert!(texts(&ignored).contains(&"field"));
            assert!(!texts(&ignored).contains(&"你"));
            assert!(!session.wants_ime());
            let (x, y) = text_spot(&ignored, "field");
            click(&mut session, x, y);
            assert!(session.wants_ime());
            assert_eq!(session.take_ime_text(), Some(String::new()));
            session.input(InputEvent::Text("你".to_string()));
            session.input(InputEvent::Composing(Some("ni".to_string())));
            let composed = session.frame(32.0);
            assert!(texts(&composed).contains(&"你ni"));
            session.input(InputEvent::Composing(None));
            session.input(InputEvent::Key(Key::Backspace));
            let cleared = session.frame(48.0);
            assert!(!texts(&cleared).contains(&"你"));
            assert!(!texts(&cleared).contains(&"你ni"));
            assert_eq!(session.take_ime_text(), Some(String::new()));
            session.input(InputEvent::Text("drop".to_string()));
            let _ = session.frame(64.0);
            assert_eq!(session.tap_count(), 0);
            assert!(!session.wants_ime());
            session.input(InputEvent::Text("no".to_string()));
            let after = session.frame(80.0);
            assert!(!texts(&after).contains(&"no"));
        }
    }
}
