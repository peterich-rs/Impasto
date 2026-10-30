//! QuickJS session. The interpreter calls `core` directly. A missing source tree
//! compiles this crate as [`Error::NotLinked`].

use uicore::Shaper;

mod error;

mod abi {
    include!(concat!(env!("OUT_DIR"), "/abi.rs"));
}

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
    open_with_io(
        script,
        name,
        shaper,
        log,
        None,
        session::HostConfig::disconnected(),
    )
}

/// Opens a session and installs the host sink for [`session::IoJob`].
pub fn open_with_io(
    script: &str,
    name: &str,
    shaper: impl Shaper + 'static,
    log: Box<dyn Fn(u8, &str)>,
    io: Option<Box<dyn Fn(session::IoJob) + Send>>,
    host: session::HostConfig,
) -> Result<Box<dyn session::Session>, Error> {
    #[cfg(quickjs_missing)]
    {
        let _ = (script, name, shaper, log, io, host);
        Err(Error::NotLinked)
    }
    #[cfg(quickjs)]
    {
        let mut session = JsSession::open_with(script, name, shaper, log, None, host)?;
        if let Some(io) = io {
            session.install_io(io);
        }
        Ok(Box::new(session))
    }
}

#[cfg(test)]
mod tests {
    use protocol::Command;
    #[cfg(quickjs)]
    use session::{
        HostConfig, HostMethod, HostModule, ImmediateExec, InputEvent, IoEvent, Key,
        ModuleRegistry, PluginResult, PluginValue,
    };
    use session::{Metrics, Session};
    use uicore::{Phase, PointerEvent};

    use super::{Error, open};

    #[test]
    fn list_driver_methods_are_functions() {
        let source = r#"
          var d = core.listDriver(2, 20);
          var proto = Object.getPrototypeOf(d);
          var names = Object.getOwnPropertyNames(proto).join(",");
          if (typeof d.configure !== "function") {
            throw new Error("configure " + typeof d.configure + " proto " + names);
          }
          d.configure(20, 2, false);
          d.setItems([{key:"a", type:"row", sticky:false}]);
          d.release();
        "#;
        match open(source, "driver.js", uicore::MonoShaper::default(), no_log()) {
            Ok(_) => {}
            Err(Error::NotLinked) => {}
            Err(error) => panic!("{error}"),
        }
    }

    fn no_log() -> Box<dyn Fn(u8, &str)> {
        Box::new(|_, _| {})
    }

    /// The gallery bundle is a build artifact and not committed. Tests that
    /// need it fail with instructions when it has not been built.
    #[cfg(quickjs)]
    fn gallery_source() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/gallery-solid/dist/counter.js");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("run `just gallery-dist` to build the gallery bundle"))
    }

    #[cfg(quickjs)]
    fn test_host() -> (
        HostConfig,
        std::sync::Arc<std::sync::Mutex<Vec<(u64, PluginResult)>>>,
    ) {
        test_host_with_launch_url(None)
    }

    #[cfg(quickjs)]
    fn test_host_with_launch_url(
        url: Option<String>,
    ) -> (
        HostConfig,
        std::sync::Arc<std::sync::Mutex<Vec<(u64, PluginResult)>>>,
    ) {
        use std::sync::{Arc, Mutex};
        let clipboard = Arc::new(Mutex::new(String::new()));
        let app_state = session::shared("active".to_owned());
        let launch_url = session::shared(url);
        let pending = Arc::new(Mutex::new(Vec::new()));
        let settle = {
            let pending = Arc::clone(&pending);
            Arc::new(move |token: u64| {
                let pending = Arc::clone(&pending);
                session::Settle::new(move |result| {
                    pending
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push((token, result));
                })
            }) as Arc<dyn Fn(u64) -> session::Settle + Send + Sync>
        };
        let config = HostConfig {
            registry: Arc::new(ModuleRegistry::new(vec![
                HostModule {
                    name: "Clipboard",
                    specifier: "@impasto/clipboard/host",
                    methods: vec![
                        HostMethod::sync("getString", {
                            let clipboard = Arc::clone(&clipboard);
                            move |_| {
                                Ok(PluginValue::Str(
                                    clipboard
                                        .lock()
                                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                                        .clone(),
                                ))
                            }
                        }),
                        HostMethod::sync("setString", {
                            let clipboard = Arc::clone(&clipboard);
                            move |args| {
                                let text = match args.first() {
                                    Some(PluginValue::Str(text)) => text.clone(),
                                    _ => String::new(),
                                };
                                *clipboard
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = text;
                                Ok(PluginValue::Null)
                            }
                        }),
                    ],
                },
                HostModule {
                    name: "AppState",
                    specifier: "@impasto/app-state/host",
                    methods: vec![HostMethod::sync("getCurrentState", {
                        let app_state = Arc::clone(&app_state);
                        move |_| Ok(PluginValue::Str(session::shared_read(&app_state)))
                    })],
                },
                HostModule {
                    name: "Echo",
                    specifier: "@impasto/echo/host",
                    methods: vec![
                        HostMethod::sync("fail", |_| Err("boom from host".into())),
                        HostMethod::promise("later", |args, settle, exec| {
                            let value = match args.first() {
                                Some(PluginValue::Str(text)) => text.clone(),
                                _ => String::new(),
                            };
                            exec.now(settle, PluginResult::Done(PluginValue::Str(value)));
                        }),
                    ],
                },
                HostModule {
                    name: "Linking",
                    specifier: "@impasto/linking/host",
                    methods: vec![HostMethod::sync("getInitialURL", {
                        let launch_url = Arc::clone(&launch_url);
                        move |_| {
                            Ok(match session::shared_read(&launch_url) {
                                Some(url) => PluginValue::Str(url),
                                None => PluginValue::Null,
                            })
                        }
                    })],
                },
            ])),
            exec: Arc::new(ImmediateExec),
            settle,
            app_state,
            launch_url,
        };
        (config, pending)
    }

    #[cfg(quickjs)]
    fn drain_plugin(
        session: &mut dyn Session,
        pending: &std::sync::Mutex<Vec<(u64, PluginResult)>>,
    ) {
        let events: Vec<_> = pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .drain(..)
            .collect();
        for (token, result) in events {
            session.io_event(IoEvent::Plugin { token, result });
        }
    }

    fn metrics() -> Metrics {
        Metrics::new(320.0, 180.0, 1.0)
    }

    fn phone() -> Metrics {
        Metrics::new(360.0, 640.0, 2.0)
    }

    fn visit_commands<'a>(
        frame: &'a session::FrameOutput,
        visit: &mut dyn FnMut(&'a Command, f32, f32),
    ) {
        let Some(tree) = frame.layers.as_deref() else {
            return;
        };
        fn walk<'a>(
            tree: &'a protocol::LayerTree,
            id: protocol::LayerId,
            tx: f32,
            ty: f32,
            visit: &mut dyn FnMut(&'a Command, f32, f32),
        ) {
            let Some(layer) = tree.layers.get(id.0 as usize) else {
                return;
            };
            let (mut next_x, mut next_y) = (tx, ty);
            match layer.kind {
                protocol::LayerKind::Offset { x, y } => {
                    next_x += x;
                    next_y += y;
                }
                protocol::LayerKind::Transform {
                    translate_x,
                    translate_y,
                    ..
                } => {
                    next_x += translate_x;
                    next_y += translate_y;
                }
                protocol::LayerKind::Picture { picture } => {
                    if let Some(picture) = tree.picture(picture) {
                        let origin_x = next_x + picture.origin.0;
                        let origin_y = next_y + picture.origin.1;
                        for command in picture.commands.iter() {
                            visit(command, origin_x, origin_y);
                        }
                    }
                }
                _ => {}
            }
            let mut child = layer.first_child;
            while let Some(id) = child {
                walk(tree, id, next_x, next_y, visit);
                child = tree.layers.get(id.0 as usize).and_then(|layer| layer.next);
            }
        }
        let mut id = Some(tree.root);
        while let Some(current) = id {
            walk(tree, current, 0.0, 0.0, visit);
            id = tree
                .layers
                .get(current.0 as usize)
                .and_then(|layer| layer.next);
        }
    }

    fn texts(frame: &session::FrameOutput) -> Vec<&str> {
        let mut found = Vec::new();
        visit_commands(frame, &mut |command, _, _| {
            if let Command::Text { utf8, .. } = command {
                found.push(utf8.as_ref());
            }
        });
        found
    }

    fn text_spot(frame: &session::FrameOutput, utf8: &str) -> (f32, f32) {
        let mut found = None;
        visit_commands(frame, &mut |command, origin_x, origin_y| {
            if let Command::Text {
                utf8: text, x, y, ..
            } = command
                && text.as_ref() == utf8
            {
                found = Some((*x + origin_x + 1.0, *y + origin_y + 1.0));
            }
        });
        found.unwrap_or_else(|| panic!("missing text {utf8}"))
    }

    fn click(session: &mut dyn Session, x: f32, y: f32) {
        session.pointer(PointerEvent::new(Phase::Down, x, y, 0.0));
        session.pointer(PointerEvent::new(Phase::Up, x, y, 0.0));
    }

    fn open_counter() -> Option<Box<dyn Session>> {
        let source = gallery_source();
        match open(&source, "test.js", uicore::MonoShaper::default(), no_log()) {
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
        session.metrics(&phone());
        let first = session.frame(session::FrameTime::at(0.0));
        assert!(texts(&first).contains(&"还是旧的"));
        let (x, y) = text_spot(&first, "微任务改字");
        click(session.as_mut(), x, y);
        let second = session.frame(session::FrameTime::at(16.0));
        assert!(texts(&second).contains(&"已经改了"));
        assert!(!texts(&second).contains(&"还是旧的"));
    }

    #[test]
    fn gallery_list_page_mounts_rows() {
        let Some(mut session) = open_counter() else {
            return;
        };
        session.metrics(&phone());
        session.settings(&session::PlatformSettings {
            locale: "zh-CN".to_owned(),
            text_scale: 1.0,
            brightness: session::Brightness::Dark,
            reduce_motion: false,
            bold_text: false,
            high_contrast: false,
        });
        let first = session.frame(session::FrameTime::at(0.0));
        assert!(texts(&first).contains(&"还是旧的"), "{:?}", texts(&first));
        assert!(
            !texts(&first)
                .iter()
                .any(|text| text.contains("not a function"))
        );
        let (x, y) = text_spot(&first, "列表");
        click(session.as_mut(), x, y);
        let second = session.frame(session::FrameTime::at(16.0));
        let shown = texts(&second);
        assert!(
            shown.contains(&"Item 0"),
            "list page replaced itself with an error: {shown:?}"
        );
        assert!(!shown.iter().any(|text| text.contains("not a function")));
    }

    fn visible_items(frame: &session::FrameOutput) -> Vec<(i32, f32)> {
        let mut rows = Vec::new();
        visit_commands(frame, &mut |command, _, origin_y| {
            if let Command::Text { utf8, y, .. } = command
                && let Some(number) = utf8
                    .strip_prefix("Item ")
                    .and_then(|rest| rest.parse().ok())
            {
                rows.push((number, *y + origin_y));
            }
        });
        rows.sort_by(|left, right| left.1.total_cmp(&right.1));
        rows
    }

    #[test]
    fn scrolled_list_numbers_stay_in_order() {
        let Some(mut session) = open_counter() else {
            return;
        };
        session.metrics(&phone());
        let first = session.frame(session::FrameTime::at(0.0));
        let (tab_x, tab_y) = text_spot(&first, "列表");
        click(session.as_mut(), tab_x, tab_y);
        let mut shown = session.frame(session::FrameTime::at(16.0));
        for step in 0..6 {
            let next = session.frame(session::FrameTime::at(32.0 + f64::from(step) * 16.0));
            if next.layers.is_some() {
                shown = next;
            }
        }
        let start = visible_items(&shown);
        assert!(
            start.len() >= 2,
            "need visible rows, texts {:?}",
            texts(&shown)
        );
        let x = text_spot(&shown, &format!("Item {}", start[0].0)).0;
        let y = start[start.len() / 2].1 + 4.0;
        for step in 0..8 {
            let time = 200.0 + f64::from(step) * 80.0;
            session.pointer(PointerEvent::new(Phase::Down, x, y, time));
            session.pointer(PointerEvent::new(Phase::Move, x, y - 120.0, time + 16.0));
            session.pointer(PointerEvent::new(Phase::Up, x, y - 120.0, time + 32.0));
            for tick in 0..4 {
                let frame =
                    session.frame(session::FrameTime::at(time + 48.0 + f64::from(tick) * 16.0));
                if frame.layers.is_some() {
                    shown = frame;
                }
            }
            let rows = visible_items(&shown);
            let numbers: Vec<i32> = rows.iter().map(|row| row.0).collect();
            for pair in numbers.windows(2) {
                assert!(
                    pair[0] < pair[1],
                    "rows out of order {numbers:?} after drag {step}"
                );
            }
            assert!(
                numbers.len() >= 2,
                "rows disappeared {numbers:?} after drag {step}"
            );
        }
    }

    #[test]
    fn pulling_the_list_commits_refresh() {
        let Some(mut session) = open_counter() else {
            return;
        };
        session.metrics(&phone());
        let first = session.frame(session::FrameTime::at(0.0));
        let (tab_x, tab_y) = text_spot(&first, "列表");
        click(session.as_mut(), tab_x, tab_y);
        let mut shown = session.frame(session::FrameTime::at(16.0));
        for step in 0..4 {
            let next = session.frame(session::FrameTime::at(32.0 + f64::from(step) * 16.0));
            if next.layers.is_some() {
                shown = next;
            }
        }
        assert!(
            texts(&shown).contains(&"下拉刷新"),
            "refresh label missing {:?}",
            texts(&shown)
        );
        let rows = visible_items(&shown);
        assert!(!rows.is_empty(), "no rows {:?}", texts(&shown));
        let x = text_spot(&shown, &format!("Item {}", rows[0].0)).0;
        let y = rows[0].1 + 8.0;
        session.pointer(PointerEvent::new(Phase::Down, x, y, 100.0));
        session.pointer(PointerEvent::new(Phase::Move, x, y + 40.0, 116.0));
        let _ = session.frame(session::FrameTime::at(132.0));
        session.pointer(PointerEvent::new(Phase::Move, x, y + 180.0, 148.0));
        let _ = session.frame(session::FrameTime::at(164.0));
        session.pointer(PointerEvent::new(Phase::Up, x, y + 180.0, 180.0));
        let mut refreshed = false;
        let mut last = shown;
        for step in 0..8 {
            let frame = session.frame(session::FrameTime::at(200.0 + f64::from(step) * 16.0));
            if frame.layers.is_some() {
                last = frame;
            }
            if texts(&last).iter().any(|text| text.starts_with("已刷新")) {
                refreshed = true;
                break;
            }
        }
        assert!(refreshed, "pull did not refresh {:?}", texts(&last));
    }

    #[test]
    fn repeated_clicks_leave_no_pending_jobs() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = gallery_source();
            let mut session = match super::JsSession::open(
                &source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&phone());
            let first = session.frame(session::FrameTime::at(0.0));
            assert!(!session.jobs_pending());
            let (x, y) = text_spot(&first, "微任务改字");
            click(&mut session, x, y);
            let second = session.frame(session::FrameTime::at(16.0));
            click(&mut session, x, y);
            let third = session.frame(session::FrameTime::at(32.0));
            assert!(texts(&second).contains(&"已经改了"));
            if third.layers.is_some() {
                assert!(texts(&third).contains(&"已经改了"), "{:?}", texts(&third));
            }
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
            let session = match super::JsSession::open(
                kept,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            assert_eq!(session.tap_count(), 0);
            let dropped = r#"
                const root = core.column(0);
                const label = core.text("Go", 16, [255, 255, 255, 255]);
                core.append(root, label);
                core.remove(root, label);
                core.setText(label, "still");
                core.append(root, label);
            "#;
            match super::JsSession::open(
                dropped,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(_) => {}
                Err(error) => panic!("removed id could not be inserted again: {error}"),
            }
        }
    }

    #[test]
    fn detach_keeps_the_tap_until_discard() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let kept = r#"
                const root = core.column(0);
                const region = core.pointerRegion();
                core.append(root, region);
                core.onClick(region, () => {});
                core.detach(root, region);
            "#;
            let mut session = match super::JsSession::open(
                kept,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            assert_eq!(session.tap_count(), 1);
            let dropped = r#"
                const root = core.column(0);
                const region = core.pointerRegion();
                core.append(root, region);
                core.onClick(region, () => {});
                core.detach(root, region);
                core.discard(region);
            "#;
            session = match super::JsSession::open(
                dropped,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            assert_eq!(session.tap_count(), 0);
        }
    }

    #[test]
    fn plus_button_rewrites_the_count() {
        let Some(mut session) = open_counter() else {
            return;
        };
        session.metrics(&phone());
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
        session.metrics(&phone());
        let first = session.frame(session::FrameTime::at(0.0));
        let (x, y) = text_spot(&first, "微任务改字");
        session.pointer(PointerEvent::new(Phase::Down, x, y, 0.0));
        session.pointer(PointerEvent::new(Phase::Move, x, -1.0, 0.0));
        session.pointer(PointerEvent::new(Phase::Up, x, -1.0, 0.0));
        let second = session.frame(session::FrameTime::at(16.0));
        if let Some(layers) = second.layers.as_ref() {
            let _ = layers;
            assert!(texts(&second).contains(&"还是旧的"), "{:?}", texts(&second));
            assert!(!texts(&second).contains(&"已经改了"));
        }
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
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
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
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
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
        match open("{", "test.js", uicore::MonoShaper::default(), no_log()) {
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
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            assert!(session.take_script_error().is_none());
            let second = session.frame(session::FrameTime::at(16.0));
            assert!(first.layers.is_some());
            assert!(second.layers.is_none());
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
                core.setListenerErrorPolicy("freeze");
                core.onClick(region, () => { throw new Error("boom"); });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
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
            assert!(second.layers.is_none());
            assert!(texts(&first).contains(&"Go"));
        }
    }

    #[test]
    fn a_reported_listener_error_lets_the_next_listener_run() {
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
                core.onClick(region, () => core.setText(label, "later"));
            "#;
            let mut session = match super::JsSession::open(
                source,
                "report.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "Go");
            click(&mut session, x, y);
            assert!(session.take_script_error().is_none());
            let second = session.frame(session::FrameTime::at(16.0));
            assert!(texts(&second).contains(&"later"), "{:?}", texts(&second));
        }
    }

    #[test]
    fn a_shortcut_consumes_the_modified_key() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const root = core.column(0);
                const label = core.text("none", 16, [0, 0, 0, 255]);
                const field = core.pointerRegion();
                core.append(root, label);
                core.append(root, field);
                core.setFocusable(field, 1);
                core.focus(field);
                core.onKey(field, (key) => core.setText(label, "key:" + key));
                core.onText(field, (text) => core.setText(label, "text:" + text));
                core.onShortcut("mod+k", () => core.setText(label, "cut"));
            "#;
            let mut session = match super::JsSession::open(
                source,
                "shortcut.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            session.input(InputEvent::Key(session::KeyPress {
                key: Key::Char('k'),
                shift: false,
                modifiers: uicore::Modifiers::empty().with(uicore::Modifiers::META),
            }));
            let frame = session.frame(session::FrameTime::at(16.0));
            assert!(texts(&frame).contains(&"cut"), "{:?}", texts(&frame));
        }
    }

    #[test]
    fn host_motion_and_semantics_hints_reach_core() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const root = core.column(0);
                const scroll = core.scroll();
                const label = core.text("row", 16, [0, 0, 0, 255]);
                core.append(root, scroll);
                core.append(scroll, label);
                core.setSemantics(label, 2, "row", undefined, undefined, undefined, undefined, undefined, undefined, undefined, undefined, { live: "polite", order: 3 });
                core.scrollIntoView(label, 0);
                core.animate(label, "opacity", 255, 0, 170, 26);
                core.setStroke(label, 2, [0, 0, 0, 255]);
                core.setRecognizers(label, { tap: true, doubleTapMs: 280, drag: "horizontal" });
                const flung = core.flingTarget(scroll);
                if (flung !== null) {
                    throw new Error("idle scroll has no fling");
                }
            "#;
            let mut session = match super::JsSession::open(
                source,
                "motion.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let frame = session.frame(session::FrameTime::at(0.0));
            assert!(session.take_script_error().is_none());
            assert!(texts(&frame).contains(&"row"), "{:?}", texts(&frame));
            let later = session.frame(session::FrameTime::at(48.0));
            assert!(later.layers.is_some() || frame.layers.is_some());
        }
    }

    #[test]
    fn motion_end_survives_unregistering_its_own_callback() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const root = core.column(0);
                const fading = core.opacity(255);
                core.append(root, fading);
                let seen = 0;
                const finish = () => {
                    seen = 1;
                };
                let first = 0;
                let second = 0;
                first = core.onMotionEnd(fading, "opacity", () => {
                    core.offMotionEnd(first);
                    finish();
                });
                second = core.onMotionEnd(fading, "opacity", () => {
                    core.offMotionEnd(second);
                    const nested = () => {
                        if (seen !== 1) {
                            throw new Error("first skipped");
                        }
                        throw new Error("both");
                    };
                    nested();
                });
                core.animate(fading, "opacity", 255, 0, 170, 26);
            "#;
            let mut session = match super::JsSession::open(
                source,
                "motion-end.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            let _ = session.frame(session::FrameTime::at(48.0));
            let message = session.take_script_error().unwrap_or_default();
            assert!(
                message.contains("both"),
                "both listeners should run, got {message:?}"
            );
        }
    }

    #[test]
    fn motion_end_reports_a_throw_after_unregister() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const root = core.column(0);
                const fading = core.opacity(255);
                const label = core.text("idle", 16, [0, 0, 0, 255]);
                core.append(root, fading);
                core.append(fading, label);
                const finish = () => {
                    const broken = null;
                    broken();
                };
                let token = 0;
                token = core.onMotionEnd(fading, "opacity", () => {
                    core.offMotionEnd(token);
                    finish();
                });
                core.animate(fading, "opacity", 255, 0, 170, 26);
            "#;
            let mut session = match super::JsSession::open(
                source,
                "motion-end-throw.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            let _ = session.frame(session::FrameTime::at(48.0));
            let message = session.take_script_error().unwrap_or_default();
            assert!(
                message.contains("not a function"),
                "script error was {message:?}"
            );
        }
    }

    #[test]
    fn align_rejects_a_column_and_metrics_report_the_viewport() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = r#"
                const frame = core.align("center");
                core.setAlign(frame, "end");
                const column = core.column(0);
                let rejected = false;
                try {
                    core.setAlign(column, "center");
                } catch (error) {
                    rejected = true;
                }
                if (!rejected) {
                    throw new Error("column accepted setAlign");
                }
                core.setCross(column, "center");
                let stackRejected = false;
                try {
                    core.setCross(core.stack(), "center");
                } catch (error) {
                    stackRejected = true;
                }
                if (!stackRejected) {
                    throw new Error("stack accepted setCross");
                }
                const label = core.text("idle", 16, [0, 0, 0, 255]);
                core.append(column, label);
                core.append(frame, column);
                const before = core.metrics();
                if (before.width !== 0) {
                    throw new Error("width before metrics " + before.width);
                }
                core.onFrame(() => {
                    const box = core.metrics();
                    core.setText(label, "w:" + box.width);
                });
            "#;
            let mut session = match super::JsSession::open(
                source,
                "align.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let frame = session.frame(session::FrameTime::at(0.0));
            assert!(session.take_script_error().is_none());
            assert!(
                texts(&frame).contains(&"w:320"),
                "metrics width should be the content box, got {:?}",
                texts(&frame)
            );
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
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
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
            let logged = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
            let logged_for_hook = std::rc::Rc::clone(&logged);
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                Box::new(move |_level, message| {
                    logged_for_hook.borrow_mut().push_str(message);
                }),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            let (x, y) = text_spot(&first, "Go");
            click(&mut session, x, y);
            assert!(session.take_script_error().is_none());
            let message = logged.borrow().clone();
            assert!(
                message.contains("interrupted"),
                "script error was {message:?}"
            );
            let second = session.frame(session::FrameTime::at(16.0));
            assert!(second.layers.is_none());
            assert!(texts(&first).contains(&"Go"));
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
                uicore::MonoShaper::default(),
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
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            assert_eq!(texts(&first), ["1"]);
            assert!(first.again);
            let second = session.frame(session::FrameTime::at(16.0));
            assert_eq!(texts(&second), ["2"]);
            assert!(!second.again);
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
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
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
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
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
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
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
            let source = gallery_source();
            let mut session = match super::JsSession::open(
                &source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
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
            for step in 1..8 {
                let next = session.frame(session::FrameTime::at(8.0 + f64::from(step) * 16.0));
                if next.layers.is_some() {
                    first = next;
                }
            }
            let Some((clip_y, clip_h)) = first.layers.as_deref().and_then(|tree| {
                tree.layers.iter().find_map(|layer| match layer.kind {
                    protocol::LayerKind::Clip { y, h, .. } if h > 100.0 => Some((y, h)),
                    _ => None,
                })
            }) else {
                panic!("scroll clip");
            };
            let mut row = None;
            visit_commands(&first, &mut |command, origin_x, origin_y| {
                if let Command::Text { utf8, x, y, .. } = command
                    && utf8.starts_with("Item ")
                {
                    let scene_y = *y + origin_y;
                    if scene_y >= clip_y && scene_y < clip_y + clip_h {
                        row = Some((utf8.clone(), *x + origin_x + 1.0, scene_y));
                    }
                }
            });
            let Some((label, x, before)) = row else {
                panic!("a row inside the viewport");
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
            let mut after = None;
            visit_commands(&scrolled, &mut |command, _, origin_y| {
                if let Command::Text { utf8, y, .. } = command
                    && utf8.as_ref() == label.as_ref()
                {
                    after = Some(*y + origin_y);
                }
            });
            let Some(after) = after else {
                panic!(
                    "scrolled label {} layers={} texts={:?}",
                    label,
                    scrolled.layers.is_some(),
                    texts(&scrolled)
                );
            };
            assert!(
                (before - after - delta).abs() < 0.5,
                "{label} moved from {before} to {after}"
            );
            assert!(texts(&scrolled).contains(&"列表"));
        }
    }

    #[cfg(quickjs)]
    fn rewritten_commands(
        before: &protocol::LayerTree,
        after: &protocol::LayerTree,
    ) -> (usize, usize) {
        let mut changed = 0;
        let mut unchanged = 0;
        for picture in &after.pictures {
            let same = before
                .picture(picture.id)
                .is_some_and(|old| old.generation == picture.generation);
            if same {
                unchanged += 1;
            } else {
                changed += picture.commands.len();
            }
        }
        (changed, unchanged)
    }

    #[test]
    fn module_loader_accepts_a_sibling_script_and_rejects_escape() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let dir = std::env::temp_dir().join(format!("quickjs-mod-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp app dir");
            std::fs::write(dir.join("util.js"), "export const label = \"from-util\";\n")
                .expect("util");
            let app = dir.join("app.js");
            std::fs::write(
                &app,
                "import { label } from \"./util.js\";\ncore.text(label, 16, [0, 0, 0, 255]);\n",
            )
            .expect("app");
            let name = app.to_string_lossy().into_owned();
            let source = std::fs::read_to_string(&app).expect("read app");
            let mut session = match super::JsSession::open(
                &source,
                &name,
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(Error::NotLinked) => return,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let frame = session.frame(session::FrameTime::at(0.0));
            assert!(
                texts(&frame).contains(&"from-util"),
                "{:?} {}",
                texts(&frame),
                session.take_script_error().unwrap_or_default()
            );
            let rejected = [
                "import { label } from \"../util.js\";\n",
                "import { label } from \"/tmp/util.js\";\n",
                "import { label } from \"./util.txt\";\n",
                "import { label } from \"./a\\\\b.js\";\n",
            ];
            for source in rejected {
                match super::JsSession::open(source, &name, uicore::MonoShaper::default(), no_log())
                {
                    Err(Error::Script(message)) => {
                        assert!(
                            message.contains("module")
                                || message.contains("outside")
                                || message.contains("could not"),
                            "{message}"
                        );
                    }
                    Err(Error::NotLinked) => return,
                    Ok(_) => panic!("expected a rejected import"),
                    Err(error) => panic!("{error}"),
                }
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn gallery_pages_scroll_without_picture_writes() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let source = gallery_source();
            let mut session = match super::JsSession::open(
                &source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(Error::NotLinked) => return,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&phone());
            let mut shown = session.frame(session::FrameTime::at(0.0));
            if !texts(&shown).iter().any(|text| text.starts_with("Item ")) {
                let (x, y) = text_spot(&shown, "列表");
                click(&mut session, x, y);
                shown = session.frame(session::FrameTime::at(16.0));
            }
            let mut quiet = 0;
            for step in 1..24 {
                let next = session.frame(session::FrameTime::at(16.0 + f64::from(step) * 16.0));
                if next.layers.is_some() {
                    shown = next;
                    quiet = 0;
                } else {
                    quiet += 1;
                    if quiet >= 2 {
                        break;
                    }
                }
            }
            assert!(
                texts(&shown).iter().any(|text| text.starts_with("Item ")),
                "{:?}",
                texts(&shown)
            );
            let Some((clip_y, clip_h)) = shown.layers.as_deref().and_then(|tree| {
                tree.layers.iter().find_map(|layer| match layer.kind {
                    protocol::LayerKind::Clip { y, h, .. } if h > 100.0 => Some((y, h)),
                    _ => None,
                })
            }) else {
                panic!("scroll clip");
            };
            let mut row = None;
            visit_commands(&shown, &mut |command, origin_x, origin_y| {
                if let Command::Text { utf8, x, y, .. } = command
                    && utf8.starts_with("Item ")
                {
                    let scene_y = *y + origin_y;
                    if scene_y >= clip_y && scene_y < clip_y + clip_h {
                        row = Some((utf8.clone(), *x + origin_x + 1.0, scene_y));
                    }
                }
            });
            let Some((_label, x, before)) = row else {
                panic!("a row inside the viewport");
            };
            let before_tree = shown.layers.clone().expect("list tree");
            session.pointer(PointerEvent::wheel(x, before + 1.0, 200.0, 0.0, clip_h));
            let scrolled = session.frame(session::FrameTime::at(200.0));
            assert!(
                scrolled.layers.is_some(),
                "scroll produced no layer tree {}",
                session.take_script_error().unwrap_or_default()
            );
            let (screen_changed, screen_kept) = rewritten_commands(
                &before_tree,
                scrolled.layers.as_deref().expect("screen tree"),
            );
            assert_eq!(session.picture_writes(), screen_changed);
            assert!(
                screen_kept > 0,
                "scrolling one screen rewrote every picture ({screen_changed} commands)"
            );
            assert!(
                texts(&scrolled)
                    .iter()
                    .any(|text| text.starts_with("Item ")),
                "list rows left after scrolling {:?}",
                texts(&scrolled)
            );
            let mut baseline = scrolled.layers.clone().expect("scroll tree");
            for step in 0..6 {
                let next = session.frame(session::FrameTime::at(216.0 + f64::from(step) * 16.0));
                if let Some(layers) = next.layers.clone() {
                    baseline = layers;
                }
            }
            let (field_x, field_y) = if texts(&shown).contains(&"姓名") {
                text_spot(&shown, "姓名")
            } else {
                text_spot(&shown, "未填写")
            };
            click(&mut session, field_x, field_y);
            session.input(InputEvent::Text("A".to_string()));
            let edited = session.frame(session::FrameTime::at(232.0));
            let Some(after_tree) = edited.layers.as_deref() else {
                panic!(
                    "edit produced no layer tree {}",
                    session.take_script_error().unwrap_or_default()
                );
            };
            let (changed, unchanged) = rewritten_commands(&baseline, after_tree);
            assert_eq!(
                session.picture_writes(),
                changed,
                "picture writes {changed} unchanged pictures {unchanged}"
            );
            assert!(changed > 0, "the edited node wrote no commands");
            assert!(unchanged > 0, "editing one node rewrote every picture");
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
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let first = session.frame(session::FrameTime::at(0.0));
            assert!(texts(&first).contains(&"field"));
            session.input(InputEvent::Text("你".to_string()));
            let ignored = session.frame(session::FrameTime::at(16.0));
            assert!(ignored.layers.is_none());
            assert!(texts(&first).contains(&"field"));
            assert!(session.take_ime().is_none());
            let (x, y) = text_spot(&first, "field");
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
            match cleared.ime {
                Some(session::ImeUpdate::Update(state)) => assert_eq!(state.text, ""),
                other => panic!("expected update, got {other:?}"),
            }
            session.input(InputEvent::Text("drop".to_string()));
            let dropped = session.frame(session::FrameTime::at(64.0));
            assert_eq!(session.tap_count(), 0);
            assert!(matches!(dropped.ime, Some(session::ImeUpdate::Hide)));
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
            let mut session = match super::JsSession::open(
                source,
                "test.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
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
                import { setString, getString } from "@impasto/clipboard/host";
                import { addEventListener } from "@impasto/app-state/host";
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
                addEventListener("change", (state) => console.log("state:" + state));
                setString("hi");
                console.log("clip:" + getString());
                const socket = new WebSocket("ws://example.test/socket");
                socket.onerror = () => console.log("ws");
            "#;
            let (host, _pending) = test_host();
            let mut session = match super::JsSession::open_with(
                source,
                "host.js",
                uicore::MonoShaper::default(),
                log,
                Some(Arc::new(FixedHttp)),
                host,
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
    fn get_initial_url_is_visible_during_eval() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let log_lines = std::sync::Arc::clone(&lines);
            let log = Box::new(move |_level: u8, text: &str| {
                log_lines
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(text.to_owned());
            });
            let source = r#"
                import { getInitialURL } from "@impasto/linking/host";
                console.log("initial:" + String(getInitialURL()));
            "#;
            let (host, _pending) =
                test_host_with_launch_url(Some("https://app.example/item".to_owned()));
            match super::JsSession::open_with(
                source,
                "launch.js",
                uicore::MonoShaper::default(),
                log,
                None,
                host,
            ) {
                Ok(_) => {}
                Err(error) => panic!("{error}"),
            }
            let text = lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .join("\n");
            assert!(text.contains("initial:https://app.example/item"), "{text}");
        }
    }

    #[test]
    fn sync_module_error_reaches_js() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let log_lines = std::sync::Arc::clone(&lines);
            let log = Box::new(move |_level: u8, text: &str| {
                log_lines
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(text.to_owned());
            });
            let source = r#"
                import { fail } from "@impasto/echo/host";
                try {
                    fail();
                    console.log("ok");
                } catch (error) {
                    console.log("err:" + error.message);
                }
            "#;
            let (host, _pending) = test_host();
            match super::JsSession::open_with(
                source,
                "error.js",
                uicore::MonoShaper::default(),
                log,
                None,
                host,
            ) {
                Ok(_) => {}
                Err(error) => panic!("{error}"),
            }
            let text = lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .join("\n");
            assert!(text.contains("err:boom from host"), "{text}");
        }
    }

    #[test]
    fn promise_module_settles_through_io_event() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let log_lines = std::sync::Arc::clone(&lines);
            let log = Box::new(move |_level: u8, text: &str| {
                log_lines
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(text.to_owned());
            });
            let source = r#"
                import { later } from "@impasto/echo/host";
                later("ok").then((value) => console.log("later:" + value));
            "#;
            let (host, pending) = test_host();
            let mut session = match super::JsSession::open_with(
                source,
                "promise.js",
                uicore::MonoShaper::default(),
                log,
                None,
                host,
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            drain_plugin(&mut session, pending.as_ref());
            session.metrics(&metrics());
            let _ = session.frame(session::FrameTime::at(0.0));
            let text = lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .join("\n");
            assert!(text.contains("later:ok"), "{text}");
        }
    }

    #[test]
    fn unregistered_host_specifier_is_not_linked() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let (host, _pending) = test_host();
            match super::JsSession::open_with(
                r#"import { getString } from "@impasto/missing/host";"#,
                "missing.js",
                uicore::MonoShaper::default(),
                no_log(),
                None,
                host,
            ) {
                Err(Error::Script(message)) => {
                    assert!(message.contains("is not linked"), "{message}");
                }
                Err(Error::NotLinked) => {}
                Ok(_) => panic!("expected unregistered host specifier to fail"),
                Err(error) => panic!("{error}"),
            }
        }
    }

    #[test]
    fn bare_scope_must_be_bundled() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let (host, _pending) = test_host();
            match super::JsSession::open_with(
                r#"import { Clipboard } from "@impasto/clipboard";"#,
                "bare.js",
                uicore::MonoShaper::default(),
                no_log(),
                None,
                host,
            ) {
                Err(Error::Script(message)) => {
                    assert!(
                        message.contains("bare @impasto/* must be bundled"),
                        "{message}"
                    );
                }
                Err(Error::NotLinked) => {}
                Ok(_) => panic!("expected bare @impasto specifier to fail"),
                Err(error) => panic!("{error}"),
            }
        }
    }

    #[test]
    fn nested_host_path_must_be_bundled() {
        #[cfg(not(quickjs))]
        {
            return;
        }
        #[cfg(quickjs)]
        {
            let (host, _pending) = test_host();
            match super::JsSession::open_with(
                r#"import { getString } from "@impasto/clipboard/extra/host";"#,
                "nested.js",
                uicore::MonoShaper::default(),
                no_log(),
                None,
                host,
            ) {
                Err(Error::Script(message)) => {
                    assert!(
                        message.contains("bare @impasto/* must be bundled"),
                        "{message}"
                    );
                }
                Err(Error::NotLinked) => {}
                Ok(_) => panic!("expected nested @impasto host path to fail"),
                Err(error) => panic!("{error}"),
            }
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
            let mut session = match super::JsSession::open(
                source,
                "semantics.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
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
                .find(|node| node.role == uicore::SemanticsRole::Switch)
                .expect("switch");
            assert_eq!(switch.value.as_deref(), Some("On"));
            assert!(switch.flags.contains(uicore::SemanticsFlags::CHECKED));
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
                uicore::MonoShaper::default(),
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
            let mut session = match super::JsSession::open(
                source,
                "tab.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
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
                uicore::MonoShaper::default(),
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
                .find(|node| node.role == uicore::SemanticsRole::TextField)
                .expect("field");
            session.semantics_action(uicore::SemanticsAction::SetText {
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
                core.registerFont("Sans", new Uint8Array([1]));
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
            let mut session = match super::JsSession::open(
                source,
                "fonts.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let queued = session.take_fonts();
            assert_eq!(queued.len(), 9, "{queued:?}");
            assert!(session.take_fonts().is_empty());
            let replaced = queued.iter().find(|(name, _)| name == "F0").expect("F0");
            assert_eq!(replaced.1, vec![9, 9]);
            assert!(queued.iter().any(|(name, _)| name == "Sans"));
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
            let again = listed.fonts;
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
            let mut session = match super::JsSession::open(
                source,
                "bigfont.js",
                uicore::MonoShaper::default(),
                no_log(),
            ) {
                Ok(session) => session,
                Err(error) => panic!("{error}"),
            };
            session.metrics(&metrics());
            let listed = session.frame(session::FrameTime::at(0.0));
            assert!(texts(&listed).contains(&"rejected"), "{:?}", texts(&listed));
            let queued = listed.fonts;
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
        let mut session = match open(source, "reload.js", uicore::MonoShaper::default(), no_log()) {
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
        let mut session = match open(source, "axis.js", uicore::MonoShaper::default(), no_log()) {
            Ok(session) => session,
            Err(Error::NotLinked) => return,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(texts(&frame).contains(&"bad-axis"), "{:?}", texts(&frame));
        let mut child = None;
        visit_commands(&frame, &mut |command, _, _| {
            if let Command::Rect { w, h, color, .. } = command
                && *color == [0, 9, 0, 255]
            {
                child = Some((*w, *h));
            }
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
        let mut session = match open(source, "image.js", uicore::MonoShaper::default(), no_log()) {
            Ok(session) => session,
            Err(Error::NotLinked) => return,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(texts(&frame).contains(&"bad-image"), "{:?}", texts(&frame));
        let mut image = None;
        visit_commands(&frame, &mut |command, _, _| {
            if let Command::Image { mode, required, .. } = command {
                image = Some((*mode, *required));
            }
        });
        assert_eq!(
            image,
            Some((protocol::ContentMode::Crop, true)),
            "{image:?}"
        );
    }

    #[test]
    fn rect_flushes_a_buffered_style_mutation() {
        let source = r#"
            const root = core.column(0);
            const box = core.sized(8, 8);
            core.append(root, box);
            const label = core.text("stale", 16, [0, 0, 0, 255]);
            core.append(root, label);
            core.applyMutation(7, box, 0, 0, 1, 30, 8, [0, 0, 0, 0], "");
            const rect = core.rect(box);
            core.setText(label, rect.w === 30 ? "applied" : "stale");
        "#;
        let mut session = match open(source, "mutate.js", uicore::MonoShaper::default(), no_log()) {
            Ok(session) => session,
            Err(Error::NotLinked) => return,
            Err(error) => panic!("{error}"),
        };
        session.metrics(&metrics());
        let frame = session.frame(session::FrameTime::at(0.0));
        assert!(texts(&frame).contains(&"applied"), "{:?}", texts(&frame));
    }
}
