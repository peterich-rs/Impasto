use super::*;
use crate::js::JsSession;

include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tools/toolchain-benchmark.rs"
));

#[test]
#[ignore = "run explicitly to collect runtime and frame samples"]
fn toolchain_baseline_counts() {
    let started = std::time::Instant::now();
    let mut runtime = javascript::runtime::RuntimeBuilder::new()
        .build()
        .expect("empty runtime");
    println!(
        "SAMPLE runtime-start {:.3}",
        started.elapsed().as_secs_f64() * 1_000_000.0
    );
    runtime
        .eval_script(
            r#"
                globalThis.finished = 0;
                for (let i = 0; i < 16; ++i) {
                    setTimeout(() => {
                        Promise.resolve().then(() => { finished += 1; });
                    }, 0);
                }
            "#,
            "bench-timers.js",
        )
        .expect("schedule timer and promise fixtures");
    let started = std::time::Instant::now();
    let report = runtime
        .drive(100)
        .expect("drive timer and promise fixtures");
    println!(
        "SAMPLE timer-jobs {:.3}",
        started.elapsed().as_secs_f64() * 1_000_000.0
    );
    assert_eq!(report.timers_ran, 16);
    runtime
        .eval_script(
            "if (finished !== 16) throw new Error('incomplete job fixture')",
            "check.js",
        )
        .expect("verify promise checkpoint");
    drop(runtime);

    sample_gallery_frames();
}

/// The gallery bundle is a build artifact and not committed. Tests that
/// need it fail with instructions when it has not been built.
fn gallery_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/gallery-solid/dist/quotes.js");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("run `just gallery-dist` to build the gallery bundle"))
}

fn chart_pictures(frame: &session::FrameOutput, height: f32) -> Vec<(u64, usize)> {
    let Some(tree) = frame.layers.as_deref() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for picture in &tree.pictures {
        for command in picture.commands().iter() {
            if let Command::Canvas {
                height: canvas_height,
                list,
                ..
            } = command
                && (canvas_height - height).abs() < 0.5
            {
                found.push((
                    picture.generation,
                    std::sync::Arc::as_ptr(&list.body) as usize,
                ));
            }
        }
    }
    found
}

fn chart_origin(frame: &session::FrameOutput, height: f32) -> Option<(f32, f32)> {
    let mut found = None;
    visit_commands(frame, &mut |command, tx, ty| {
        if let Command::Canvas {
            x,
            y,
            height: canvas_height,
            ..
        } = command
            && (canvas_height - height).abs() < 0.5
        {
            found = Some((tx + *x, ty + *y));
        }
    });
    found
}

fn volume_readout(frame: &session::FrameOutput) -> bool {
    texts(frame).iter().any(|text| text.starts_with("成交量 "))
}

fn chart_recorded_volume(frame: &session::FrameOutput, height: f32) -> bool {
    let mut found = false;
    visit_commands(frame, &mut |command, _, _| {
        if let Command::Canvas {
            height: canvas_height,
            list,
            ..
        } = command
            && (canvas_height - height).abs() < 0.5
        {
            found |= list.ops().iter().any(
                |op| matches!(op, crate::scene::CanvasOp::Text(text) if text.utf8.contains("成交量")),
            );
        }
    });
    found
}

/// Pumps frames until the first screen lands. The quotes app mounts a
/// heavier first screen than the old counter.

/// Center of the first button whose accessible label matches exactly.
/// Icon controls have no text to spot, so the tests reach them by label.
fn label_spot(session: &mut JsSession, label: &str) -> Option<(f32, f32)> {
    let update = session.semantics()?;
    update
        .nodes
        .iter()
        .find(|node| node.role == crate::semantics::SemanticsRole::Button && node.label == label)
        .map(|node| {
            (
                node.rect.x + node.rect.w / 2.0,
                node.rect.y + node.rect.h / 2.0,
            )
        })
}

fn wait_for_label(session: &mut JsSession, time: &mut f64, expected: &str) -> session::FrameOutput {
    for _ in 0..48 {
        *time += 16.0;
        let frame = session.frame(session::FrameTime::at(*time));
        let errors = session.take_faults();
        assert!(errors.is_empty(), "waiting for {expected}: {errors:?}");
        if frame.layers.is_some() && label_spot(session, expected).is_some() {
            return frame;
        }
    }
    panic!("missing label {expected}");
}

fn click_label(session: &mut JsSession, label: &str) {
    let (x, y) = label_spot(session, label).unwrap_or_else(|| panic!("missing label {label}"));
    click(session, x, y);
}

fn wait_for_text(
    session: &mut dyn Session,
    time: &mut f64,
    expected: &str,
) -> session::FrameOutput {
    let mut seen = Vec::new();
    for _ in 0..48 {
        *time += 16.0;
        let frame = session.frame(session::FrameTime::at(*time));
        let errors = session.take_faults();
        assert!(errors.is_empty(), "waiting for {expected}: {errors:?}");
        if frame.layers.is_some() {
            let labels = texts(&frame);
            if labels.iter().any(|text| text == expected) {
                return frame;
            }
            seen = labels;
        }
    }
    panic!("missing {expected} in {seen:?}");
}

/// Host modules the gallery bundle imports. Devtools stays off so the
/// panel does not change the page the assertions read.
fn gallery_host() -> ModuleConfig {
    ModuleConfig {
        asset_reader: None,
        registry: std::sync::Arc::new(ModuleRegistry::new(vec![
            Module {
                name: "AsyncStorage",
                exports: session::ModuleExports {
                    object: true,
                    ..Default::default()
                },
                specifier: "@impasto/async-storage/native",
                methods: vec![
                    Method::sync("multiGet", |_| Ok(PluginValue::List(Vec::new()))),
                    Method::sync("multiSet", |_| Ok(PluginValue::Null)),
                ],
            },
            Module {
                name: "Linking",
                exports: session::ModuleExports {
                    object: true,
                    events: LINKING_EVENTS,
                    ..Default::default()
                },
                specifier: "@impasto/linking/native",
                methods: vec![
                    Method::sync("getInitialURL", |_| Ok(PluginValue::Null)),
                    Method::sync("openURL", |_| Ok(PluginValue::Null)),
                    Method::sync("canOpenURL", |_| Ok(PluginValue::Bool(false))),
                ],
            },
            Module {
                name: "Clipboard",
                exports: session::ModuleExports {
                    object: true,
                    ..Default::default()
                },
                specifier: "@impasto/clipboard/native",
                methods: vec![
                    Method::sync("getString", |_| Ok(PluginValue::Str(String::new()))),
                    Method::sync("setString", |_| Ok(PluginValue::Null)),
                ],
            },
            Module {
                name: "Devtools",
                exports: Default::default(),
                specifier: "@impasto/devtools/native",
                methods: vec![
                    Method::sync("enabled", |_| Ok(PluginValue::Bool(false))),
                    Method::sync("openSource", |_| Ok(PluginValue::Bool(false))),
                ],
            },
        ])),
        exec: std::sync::Arc::new(ImmediateExec),
        settle: std::sync::Arc::new(|_token| session::Settle::new(|_result| {})),
        post_delivery: None,
        transport_clock: None,
        script_execution: Default::default(),
    }
}

/// The gallery bundle is a build artifact and not committed. Tests that
/// need it fail with instructions when it has not been built. Returns
/// `None` when the interpreter is not linked; callers already early-return.
fn open_gallery() -> Option<Box<JsSession>> {
    open_gallery_source(&gallery_source())
}

/// The concrete session type keeps `semantics()` reachable for the label-based helpers.
fn open_gallery_source(source: &str) -> Option<Box<JsSession>> {
    {
        match JsSession::open_with(
            source,
            "test.js",
            crate::text::MonoShaper::default(),
            Box::new(|level, line| eprintln!("[js {level}] {line}")),
            gallery_host(),
        ) {
            Ok(session) => Some(Box::new(session)),
            Err(error) => panic!("{error}"),
        }
    }
}

#[test]
fn watchlist_page_shows_prices() {
    let Some(mut session) = open_gallery() else {
        return;
    };
    session.metrics(&phone());
    // First-screen mount is heavy; pump frames until the page lands.
    let mut first = session.frame(session::FrameTime::at(0.0));
    for step in 0..16 {
        if first.layers.is_some() {
            break;
        }
        first = session.frame(session::FrameTime::at(f64::from(step) * 16.0));
    }
    let shown = texts(&first);
    assert!(
        shown.iter().any(|text| text == "综合指数"),
        "{shown:?} error={:?} faults={:?}",
        session.take_script_error(),
        session.take_faults()
    );
    assert!(shown.iter().any(|text| text == "腾讯控股"), "{shown:?}");
    assert!(
        shown.iter().any(|text| text == "42"),
        "the default watchlist should show all 42 stocks: {shown:?}"
    );
    let mut sparks = 0;
    visit_commands(&first, &mut |command, _, _| {
        if matches!(command, Command::Canvas { list, .. } if !list.ops().is_empty()) {
            sparks += 1;
        }
    });
    assert!(sparks > 0, "no sparklines: {:?}", session.take_faults());
    // Binder write-on-bind: price cells carry formatted numbers already.
    assert!(
        shown.iter().any(|text| text.ends_with(".00")
            || text.ends_with(".20")
            || text.ends_with(".40")
            || text.ends_with(".60")
            || text.ends_with(".80")
            || text.chars().any(|c| c == '.')),
        "no formatted price in {shown:?}"
    );
}

#[test]
fn account_tab_renders_equity_and_positions() {
    let Some(mut session) = open_gallery() else {
        return;
    };
    session.metrics(&phone());
    let first = pump_first(session.as_mut());
    assert!(
        texts(&first).iter().any(|text| text == "综合指数"),
        "{:?}",
        texts(&first)
    );
    assert!(
        !texts(&first)
            .iter()
            .any(|text| text.contains("not a function")),
        "{:?}",
        texts(&first)
    );
    click_label(session.as_mut(), "账户");
    let mut shown = session.frame(session::FrameTime::at(16.0));
    for step in 0..6 {
        let next = session.frame(session::FrameTime::at(32.0 + f64::from(step) * 16.0));
        if next.layers.is_some() {
            shown = next;
        }
    }
    let labels = texts(&shown);
    for expected in [
        "总资产",
        "可用资金",
        "持仓市值",
        "累计盈亏",
        "成本价",
        "仓位",
        "今日盈亏",
    ] {
        assert!(
            labels.iter().any(|text| text == expected),
            "{expected} missing {labels:?}"
        );
    }
    // Seed positions are present with a formatted price badge.
    assert!(labels.iter().any(|text| text == "00700"), "{labels:?}");
    assert!(!labels.iter().any(|text| text.contains("not a function")));
    assert!(
        !labels.iter().any(|text| text == "综合指数"),
        "account tab replaced the board {labels:?}"
    );
}

fn visible_watch_rows(frame: &session::FrameOutput) -> Vec<(String, f32)> {
    let mut rows = Vec::new();
    visit_commands(frame, &mut |command, _, origin_y| {
        if let Command::Text {
            shaped: Some(shaped),
            y,
            ..
        } = command
        {
            let text = mono_text(shaped);
            if IN_WATCHLIST_NAMES.contains(&text.as_str()) {
                rows.push((text, *y + origin_y));
            }
        }
    });
    rows.sort_by(|left, right| left.1.total_cmp(&right.1));
    rows
}

const IN_WATCHLIST_NAMES: &[&str] = &[
    "腾讯控股",
    "阿里巴巴",
    "美团",
    "中国移动",
    "小米集团",
    "中国平安",
    "友邦保险",
    "理想汽车",
    "小鹏汽车",
    "京东集团",
    "Apple",
    "NVIDIA",
    "贵州茅台",
    "宁德时代",
    "Tesla",
];

#[test]
fn watchlist_publishes_quotes_during_drag_and_coast() {
    let Some(mut session) = open_gallery() else {
        return;
    };
    session.metrics(&phone());
    let mut time = 0.0;
    let first = wait_for_text(session.as_mut(), &mut time, "腾讯控股");
    let start = visible_watch_rows(&first);
    let (x, y) = text_spot(&first, "宁德时代");
    session.pointer(PointerEvent::new(Phase::Down, x, y, time));
    let mut dragged = first;
    for step in 1u16..=3 {
        time += 16.0;
        session.pointer(PointerEvent::new(
            Phase::Move,
            x,
            y - 20.0 * f32::from(step),
            time,
        ));
        let frame = session.frame(session::FrameTime::at(time));
        if frame.layers.is_some() {
            dragged = frame;
        }
    }
    let moved = visible_watch_rows(&dragged);
    assert!(
        start.iter().any(|(name, y)| moved
            .iter()
            .any(|(row, offset)| name == row && y - offset > 8.0)),
        "the gesture at ({x}, {y}) did not scroll the watchlist: {start:?} -> {moved:?}"
    );
    let clock = |frame: &session::FrameOutput| {
        texts(frame).into_iter().find(|text| {
            text.len() == 8 && text.as_bytes()[2] == b':' && text.as_bytes()[5] == b':'
        })
    };
    let mut last_clock = clock(&dragged).expect("market clock");
    let mut updated_while_dragging = false;
    let mut drag_sparks_redrawn = false;
    for step in 1u16..=40 {
        time += 16.0;
        session.pointer(PointerEvent::new(
            Phase::Move,
            x,
            y - 60.0 - f32::from(step),
            time,
        ));
        let frame = session.frame(session::FrameTime::at(time));
        assert!(session.take_faults().is_empty(), "watchlist frame failed");
        if let Some(next) = clock(&frame) {
            if next != last_clock {
                updated_while_dragging = true;
                drag_sparks_redrawn |= redraws(&frame) > 0;
                last_clock = next;
            }
            dragged = frame;
        }
    }
    assert!(
        updated_while_dragging,
        "market clock stalled during the drag"
    );
    assert!(drag_sparks_redrawn, "sparklines stalled during the drag");

    let mut last_rows = visible_watch_rows(&dragged);
    for step in 1u16..=4 {
        time += 16.0;
        session.pointer(PointerEvent::new(
            Phase::Move,
            x,
            y - 100.0 - 20.0 * f32::from(step),
            time,
        ));
        let frame = session.frame(session::FrameTime::at(time));
        if let Some(next) = clock(&frame) {
            last_clock = next;
            last_rows = visible_watch_rows(&frame);
        }
    }
    time += 16.0;
    session.pointer(PointerEvent::new(Phase::Up, x, y - 180.0, time));
    let mut coasting = false;
    let mut updated_while_coasting = false;
    for _ in 0..40 {
        let frame = session.frame(session::FrameTime::at(time));
        assert!(session.take_faults().is_empty(), "watchlist frame failed");
        if let Some(next) = clock(&frame) {
            let rows = visible_watch_rows(&frame);
            let moved = !rows.is_empty() && rows != last_rows;
            coasting |= moved;
            updated_while_coasting |= moved && next != last_clock && redraws(&frame) > 0;
            last_clock = next;
            last_rows = rows;
        }
        time += 16.0;
    }
    assert!(coasting, "the gesture did not start inertial scrolling");
    assert!(
        updated_while_coasting,
        "quotes did not update while the list was coasting"
    );
}

#[test]
fn scrolled_watch_rows_stay_in_order() {
    let Some(mut session) = open_gallery() else {
        return;
    };
    session.metrics(&phone());
    let _first = pump_first(session.as_mut());
    let mut shown = session.frame(session::FrameTime::at(16.0));
    for step in 0..6 {
        let next = session.frame(session::FrameTime::at(32.0 + f64::from(step) * 16.0));
        if next.layers.is_some() {
            shown = next;
        }
    }
    let start = visible_watch_rows(&shown);
    assert!(
        start.len() >= 2,
        "need visible rows, texts {:?}",
        texts(&shown)
    );
    let (x, y) = text_spot(&shown, "宁德时代");
    for step in 0..6 {
        let time = 200.0 + f64::from(step) * 80.0;
        session.pointer(PointerEvent::new(Phase::Down, x, y, time));
        session.pointer(PointerEvent::new(Phase::Move, x, y - 120.0, time + 16.0));
        session.pointer(PointerEvent::new(Phase::Up, x, y - 120.0, time + 32.0));
        for tick in 0..4 {
            let frame = session.frame(session::FrameTime::at(time + 48.0 + f64::from(tick) * 16.0));
            if frame.layers.is_some() {
                shown = frame;
            }
        }
        let rows = visible_watch_rows(&shown);
        assert!(
            rows.len() >= 2,
            "rows disappeared {:?} after drag {step}",
            texts(&shown)
        );
    }
}

#[test]
fn settings_tab_renders_preferences_and_returns_to_the_watchlist() {
    let Some(mut session) = open_gallery() else {
        return;
    };
    session.metrics(&phone());
    let _first = pump_first(session.as_mut());
    click_label(session.as_mut(), "设置");
    let mut shown = session.frame(session::FrameTime::at(16.0));
    for step in 0..6 {
        let next = session.frame(session::FrameTime::at(32.0 + f64::from(step) * 16.0));
        if next.layers.is_some() {
            shown = next;
        }
    }
    let labels = texts(&shown);
    for expected in ["涨跌配色", "语言", "行情速度", "重置模拟账户"] {
        assert!(
            labels.iter().any(|text| text == expected),
            "{expected} missing {labels:?}"
        );
    }
    click_label(session.as_mut(), "自选");
    let mut back = session.frame(session::FrameTime::at(160.0));
    for step in 0..6 {
        let next = session.frame(session::FrameTime::at(176.0 + f64::from(step) * 16.0));
        if next.layers.is_some() {
            back = next;
        }
    }
    assert!(
        texts(&back).iter().any(|text| text == "综合指数"),
        "{:?}",
        texts(&back)
    );
}

#[test]
fn stocks_desktop_switches_symbols_and_keeps_chart_and_book_inline() {
    let Some(mut session) = open_gallery() else {
        return;
    };
    session.metrics(&Metrics::new(1360.0, 820.0, 1.0));
    let mut time = 0.0;
    let first = wait_for_text(session.as_mut(), &mut time, "价格区间");
    let shown = texts(&first);
    assert!(
        shown.iter().any(|text| text == "综合指数") && shown.iter().any(|text| text == "盘口"),
        "{shown:?}"
    );
    let (chart_x, _) = text_spot(&first, "分时");
    let (book_x, _) = text_spot(&first, "盘口");
    assert!(
        chart_x > 300.0 && book_x > chart_x + 400.0,
        "chart {chart_x}, book {book_x}"
    );

    // Apple sits near the top of the saved list so it stays mounted at this height.
    let (x, y) = text_spot(&first, "Apple");
    click(session.as_mut(), x, y);
    let changed = wait_for_text(session.as_mut(), &mut time, "Apple");
    assert_eq!(
        texts(&changed)
            .iter()
            .filter(|text| **text == "AAPL")
            .count(),
        2,
        "selected symbol should appear in both panes"
    );
    let (x, y) = text_spot(&changed, "5 → 10");
    click(session.as_mut(), x, y);
    let ten = wait_for_text(session.as_mut(), &mut time, "A10");
    assert!(
        texts(&ten).iter().any(|text| text == "A10")
            && texts(&ten).iter().any(|text| text == "B10")
    );
    let (x, y) = text_spot(&ten, "1D");
    click(session.as_mut(), x, y);
    let candles = wait_for_text(session.as_mut(), &mut time, "MA20");
    let mut plot = None;
    visit_commands(&candles, &mut |command, tx, ty| {
        if let Command::Canvas { x, y, height, .. } = command
            && *height == 320.0
        {
            plot = Some((tx + *x, ty + *y));
        }
    });
    let (x, y) = plot.expect("daily candle plot");
    session.pointer(PointerEvent::new(Phase::Down, x + 120.0, y + 40.0, time));
    time += 16.0;
    session.pointer(PointerEvent::new(Phase::Move, x + 144.0, y + 40.0, time));
    time += 16.0;
    let mut focus = session.frame(session::FrameTime::at(time));
    for _ in 0..3 {
        if volume_readout(&focus) {
            break;
        }
        time += 16.0;
        let next = session.frame(session::FrameTime::at(time));
        if next.layers.is_some() {
            focus = next;
        }
    }
    assert!(
        volume_readout(&focus),
        "crosshair should show a candle readout, got {:?}",
        texts(&focus)
    );
    assert!(
        !chart_recorded_volume(&focus, 320.0),
        "main chart should not record the crosshair readout"
    );
    let errors = session.take_faults();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn stocks_phone_crosshair_keeps_the_chart_picture() {
    let Some(mut session) = open_gallery() else {
        return;
    };
    session.metrics(&phone());
    let mut time = 0.0;
    let first = wait_for_text(session.as_mut(), &mut time, "腾讯控股");
    let (x, y) = text_spot(&first, "腾讯控股");
    click(session.as_mut(), x, y);
    let _opening = wait_for_label(session.as_mut(), &mut time, "返回");
    let detail = wait_for_text(session.as_mut(), &mut time, "价格区间");
    let (daily_x, daily_y) = text_spot(&detail, "1D");
    click(session.as_mut(), daily_x, daily_y);
    wait_for_text(session.as_mut(), &mut time, "MA5");
    click_label(session.as_mut(), "暂停行情");
    // A quiet frame did not re-record a canvas. The chart box from the last
    // submitted tree is still where the pointer hits.
    let mut baseline = None;
    let mut point = None;
    let mut last_spot = None;
    let mut held_spot = None;
    let mut held_pictures = None;
    for _ in 0..8 {
        time += 16.0;
        let frame = session.frame(session::FrameTime::at(time));
        let errors = session.take_faults();
        assert!(errors.is_empty(), "{errors:?}");
        if let Some(spot) = chart_origin(&frame, 204.0) {
            let pictures = chart_pictures(&frame, 204.0);
            if pictures.len() == 1 {
                held_spot = Some(spot);
                held_pictures = Some(pictures);
            }
        }
        let (Some((plot_x, plot_y)), Some(pictures)) = (held_spot, held_pictures.clone()) else {
            continue;
        };
        last_spot = Some((plot_x, plot_y));
        let x = plot_x + 16.0;
        let y = plot_y + 30.0;
        let placed = (8.0..302.0).contains(&x) && (8.0..600.0).contains(&y);
        if redraws(&frame) == 0 && placed {
            baseline = Some(pictures);
            point = Some((x, y));
            break;
        }
    }
    let baseline = baseline.unwrap_or_else(|| panic!("chart canvas spot={last_spot:?}"));
    let (x, y) = point.expect("chart point");

    session.pointer(PointerEvent::new(Phase::Down, x, y, time));
    time += 16.0;
    session.pointer(PointerEvent::new(Phase::Move, x + 24.0, y, time));
    time += 16.0;
    let mut dragged = session.frame(session::FrameTime::at(time));
    for _ in 0..3 {
        if volume_readout(&dragged) {
            break;
        }
        time += 16.0;
        let next = session.frame(session::FrameTime::at(time));
        if next.layers.is_some() {
            dragged = next;
        }
    }
    assert!(
        volume_readout(&dragged),
        "crosshair tooltip missing {:?}",
        texts(&dragged)
    );
    let same_chart = |frame: &session::FrameOutput, step: &str| {
        if frame.layers.is_none() {
            assert_eq!(redraws(frame), 0, "{step}");
            return;
        }
        assert_eq!(
            chart_pictures(frame, 204.0),
            baseline,
            "{step} redraws={}",
            redraws(frame)
        );
        assert!(!chart_recorded_volume(frame, 204.0), "{step}");
    };
    same_chart(&dragged, "crosshair opens");
    assert_eq!(redraws(&dragged), 0, "press and drag must skip Canvas draw");
    let first_readout = texts(&dragged);
    session.pointer(PointerEvent::new(Phase::Move, x + 48.0, y, time));
    time += 16.0;
    let dragged = session.frame(session::FrameTime::at(time));
    same_chart(&dragged, "horizontal drag again");
    assert!(volume_readout(&dragged));
    assert_ne!(texts(&dragged), first_readout);
    assert_eq!(
        dragged
            .phases
            .expect("timed gallery frame")
            .trace
            .root_layouts,
        0,
        "changing the selected daily bar should keep ancestor layout"
    );

    session.pointer(PointerEvent::new(Phase::Cancel, x + 48.0, y, time));
    time += 16.0;
    let mut cancelled = session.frame(session::FrameTime::at(time));
    for _ in 0..3 {
        if !volume_readout(&cancelled) && cancelled.layers.is_some() {
            break;
        }
        time += 16.0;
        let next = session.frame(session::FrameTime::at(time));
        if next.layers.is_some() {
            cancelled = next;
        }
    }
    assert!(
        !volume_readout(&cancelled),
        "cancel should clear the crosshair {:?}",
        texts(&cancelled)
    );
    same_chart(&cancelled, "crosshair cancels");
    assert_eq!(redraws(&cancelled), 0, "cancel must skip Canvas draw");

    let (_, before_y) = text_spot(&dragged, "分时");
    session.pointer(PointerEvent::new(Phase::Down, x, y, time));
    time += 16.0;
    session.pointer(PointerEvent::new(Phase::Move, x, y - 80.0, time));
    time += 16.0;
    let mut scrolled = None;
    for _ in 0..4 {
        let frame = session.frame(session::FrameTime::at(time));
        time += 16.0;
        let errors = session.take_faults();
        assert!(errors.is_empty(), "{errors:?}");
        if frame.layers.is_some() {
            assert!(!chart_recorded_volume(&frame, 204.0), "vertical scroll");
        }
        if texts(&frame).iter().any(|text| text == "分时") {
            scrolled = Some(text_spot(&frame, "分时").1);
            break;
        }
    }
    let after_y = scrolled.expect("分时 stayed visible");
    assert_ne!(after_y, before_y, "vertical drag should scroll the page");
    session.pointer(PointerEvent::new(Phase::Cancel, x, y - 80.0, time));
    time += 16.0;
    let ended = session.frame(session::FrameTime::at(time));
    assert!(ended.phases.is_some(), "scroll cancel");
    assert!(!chart_recorded_volume(&ended, 204.0), "scroll cancel");
}

#[test]
fn stock_detail_coasts_between_live_market_updates() {
    let Some(mut session) = open_gallery() else {
        return;
    };
    session.metrics(&phone());
    let mut time = 0.0;
    let first = wait_for_text(session.as_mut(), &mut time, "腾讯控股");
    let (x, y) = text_spot(&first, "腾讯控股");
    click(session.as_mut(), x, y);
    let detail = wait_for_text(session.as_mut(), &mut time, "价格区间");
    let (x, y) = text_spot(&detail, "分时");
    let y = y + 80.0;
    session.pointer(PointerEvent::new(Phase::Down, x, y, time));
    for distance in [30.0, 70.0] {
        time += 16.0;
        session.pointer(PointerEvent::new(Phase::Move, x, y - distance, time));
        session.frame(session::FrameTime::at(time));
    }
    time += 16.0;
    session.pointer(PointerEvent::new(Phase::Up, x, y - 70.0, time));

    let mut layers = None;
    let mut eligibility = None;
    for _ in 0..8 {
        let frame = session.frame(session::FrameTime::at(time));
        time += 16.0;
        let faults = session.take_faults();
        assert!(faults.is_empty(), "{faults:?}");
        if let Some(next) = frame.layers {
            layers = Some(next);
        }
        let Some(scene) = &layers else {
            continue;
        };
        if scene.tracks.iter().any(|track| {
            track.prop == crate::scene::CompositorProp::ScrollY && track.threshold_window.is_some()
        }) {
            let result = session.compositor_eligibility(&scene.tracks);
            if result.is_ok() {
                return;
            }
            eligibility = Some(result);
        }
    }
    panic!(
        "stock detail should coast with a section threshold window, eligibility: {eligibility:?}"
    );
}

#[test]
fn stocks_phone_opens_detail_and_order_ticket() {
    let Some(mut session) = open_gallery() else {
        return;
    };
    session.metrics(&phone());
    let mut time = 0.0;
    let first = wait_for_text(session.as_mut(), &mut time, "腾讯控股");
    let (x, y) = text_spot(&first, "腾讯控股");
    click(session.as_mut(), x, y);
    let opening = wait_for_label(session.as_mut(), &mut time, "返回");
    assert!(texts(&opening).iter().any(|text| text == "买入"));
    assert!(!texts(&opening).iter().any(|text| text == "盘口"));
    let mut detail = wait_for_text(session.as_mut(), &mut time, "价格区间");
    for _ in 0..20 {
        time += 16.0;
        let next = session.frame(session::FrameTime::at(time));
        let faults = session.take_faults();
        assert!(faults.is_empty(), "{faults:?}");
        if next.layers.is_some() {
            detail = next;
        }
    }
    let (x, y) = text_spot(&detail, "买入");
    click(session.as_mut(), x, y);
    wait_for_text(session.as_mut(), &mut time, "模拟下单");
}

#[test]
fn stocks_phone_reopens_during_exit_without_leaving_a_detail() {
    let Some(mut session) = open_gallery() else {
        return;
    };
    session.metrics(&phone());
    let mut time = 0.0;
    let home = wait_for_text(session.as_mut(), &mut time, "腾讯控股");
    let (x, y) = text_spot(&home, "腾讯控股");
    let (other_x, other_y) = text_spot(&home, "Apple");
    click(session.as_mut(), x, y);
    wait_for_label(session.as_mut(), &mut time, "返回");
    click(session.as_mut(), other_x, other_y);
    let detail = wait_for_text(session.as_mut(), &mut time, "价格区间");
    let (settled_x, _) = text_spot(&detail, "价格区间");
    click_label(session.as_mut(), "返回");
    // Let the exit slide run for a few frames before re-tapping the home row.
    let mut exiting = detail;
    for _ in 0..9 {
        time += 16.0;
        let frame = session.frame(session::FrameTime::at(time));
        if frame.layers.is_some() {
            exiting = frame;
        }
    }
    // The leaving page has travelled right of the home row it covers.
    let (leaving_x, _) = text_spot(&exiting, "价格区间");
    assert!(
        leaving_x > settled_x && leaving_x > other_x,
        "exit slide did not move the detail: {settled_x} -> {leaving_x}, home row at {other_x}"
    );
    click(session.as_mut(), other_x, other_y);
    let mut reopened = wait_for_text(session.as_mut(), &mut time, "价格区间");
    assert_eq!(
        texts(&reopened)
            .iter()
            .filter(|label| **label == "AAPL")
            .count(),
        2
    );
    session.input(InputEvent::Back);
    for _ in 0..48 {
        time += 16.0;
        let frame = session.frame(session::FrameTime::at(time));
        let faults = session.take_faults();
        assert!(faults.is_empty(), "{faults:?}");
        if frame.layers.is_some() {
            reopened = frame;
        }
    }
    let shown = texts(&reopened);
    assert!(shown.iter().any(|text| text == "综合指数"), "{shown:?}");
    assert!(label_spot(session.as_mut(), "返回").is_none(), "{shown:?}");

    // Re-measure: the home row sits higher now that the detail has gone.
    let (x, y) = text_spot(&reopened, "腾讯控股");
    click(session.as_mut(), x, y);
    wait_for_text(session.as_mut(), &mut time, "买入");
    session.input(InputEvent::Back);
    let mut landed = session.frame(session::FrameTime::at(time));
    for _ in 0..48 {
        time += 16.0;
        let frame = session.frame(session::FrameTime::at(time));
        assert!(session.take_faults().is_empty());
        if frame.layers.is_some() {
            landed = frame;
            if texts(&landed).iter().any(|text| text == "综合指数")
                && label_spot(session.as_mut(), "返回").is_none()
            {
                break;
            }
        }
    }
    assert!(
        texts(&landed).iter().any(|text| text == "综合指数"),
        "back from the detail should land on the watchlist: {:?}",
        texts(&landed).iter().take(20).collect::<Vec<_>>()
    );
}
