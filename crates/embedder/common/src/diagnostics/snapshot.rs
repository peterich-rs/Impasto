//! Host-side telemetry snapshots and capture commands.

use std::sync::Arc;

use impasto::{DiagnosticsSource, PluginContext, abi::PluginValue};

use super::{DiagnosticsHub, PhaseSample};

/// Extra host facts the hub does not own.
#[derive(Clone)]
struct DevtoolsContext {
    enabled: bool,
    /// Bundle source map text, when the host found `<js>.map` next to the script.
    source_map: Option<Arc<String>>,
    /// `AppState.currentState`, the same slot the app-state module writes.
    app_state: PluginContext,
    os: &'static str,
    os_version: String,
    /// `IMPASTO_DUMP` path, when set.
    dump: Option<String>,
}

fn map(pairs: Vec<(&str, PluginValue)>) -> PluginValue {
    PluginValue::Map(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

fn num(value: u64) -> PluginValue {
    PluginValue::F64(value as f64)
}

fn fnum(value: f64) -> PluginValue {
    PluginValue::F64(value)
}

fn text_preview(bytes: &[u8]) -> PluginValue {
    match std::str::from_utf8(bytes) {
        Ok(text) => PluginValue::Str(text.to_owned()),
        Err(_) => PluginValue::Str(format!("<{} bytes>", bytes.len())),
    }
}

fn headers_json(headers: &[(String, String)]) -> PluginValue {
    PluginValue::List(
        headers
            .iter()
            .map(|(name, value)| {
                PluginValue::List(vec![
                    PluginValue::Str(name.clone()),
                    PluginValue::Str(value.clone()),
                ])
            })
            .collect(),
    )
}

fn body_preview(entry_body: &Option<Vec<u8>>) -> PluginValue {
    match entry_body {
        Some(bytes) if !bytes.is_empty() => text_preview(bytes),
        _ => PluginValue::Null,
    }
}

fn since_arg(args: &[PluginValue]) -> u64 {
    let Some(PluginValue::F64(value)) = args.first() else {
        return 0;
    };
    if !value.is_finite() || *value < 0.0 {
        return 0;
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "log cursors are non-negative sequence numbers and fit in u64"
    )]
    {
        *value as u64
    }
}

fn logs_channel(hub: &DiagnosticsHub, args: &[PluginValue]) -> Result<PluginValue, String> {
    let since = since_arg(args);
    let guard = hub.logs.lock().unwrap_or_else(|p| p.into_inner());
    let entries: Vec<PluginValue> = guard
        .tail(since)
        .map(|entry| {
            map(vec![
                ("seq", num(entry.seq)),
                ("at", fnum(entry.at_ms)),
                ("level", PluginValue::Str(entry.level.as_str().to_owned())),
                ("source", PluginValue::Str(entry.source.name().to_owned())),
                ("target", PluginValue::Str(entry.target.to_owned())),
                ("text", PluginValue::Str(entry.message.clone())),
                ("frame", entry.frame.map(num).unwrap_or(PluginValue::Null)),
            ])
        })
        .collect();
    Ok(map(vec![
        ("revision", num(guard.revision())),
        ("entries", PluginValue::List(entries)),
    ]))
}

fn errors_channel(hub: &DiagnosticsHub, args: &[PluginValue]) -> Result<PluginValue, String> {
    let since = since_arg(args);
    let guard = hub.errors.lock().unwrap_or_else(|p| p.into_inner());
    let entries: Vec<PluginValue> = guard
        .tail(since)
        .map(|entry| {
            map(vec![
                ("seq", num(entry.seq)),
                ("at", fnum(entry.at_ms)),
                ("source", PluginValue::Str(entry.source.to_owned())),
                ("message", PluginValue::Str(entry.message.clone())),
                (
                    "stack",
                    entry
                        .stack
                        .as_deref()
                        .map(|s| PluginValue::Str(s.to_owned()))
                        .unwrap_or(PluginValue::Null),
                ),
                ("frame", entry.frame.map(num).unwrap_or(PluginValue::Null)),
            ])
        })
        .collect();
    Ok(map(vec![
        ("revision", num(guard.revision())),
        ("entries", PluginValue::List(entries)),
    ]))
}

fn network_channel(hub: &DiagnosticsHub, args: &[PluginValue]) -> Result<PluginValue, String> {
    let since = since_arg(args);
    let guard = hub.network.lock().unwrap_or_else(|p| p.into_inner());
    let entries: Vec<PluginValue> = guard
        .tail(since)
        .map(|entry| {
            map(vec![
                ("seq", num(entry.seq)),
                ("at", fnum(entry.at_ms)),
                ("kind", PluginValue::Str(entry.kind.name().to_owned())),
                ("token", num(entry.token)),
                ("method", PluginValue::Str(entry.method.clone())),
                ("url", PluginValue::Str(entry.url.clone())),
                ("requestHeaders", headers_json(&entry.request_headers)),
                ("requestBody", body_preview(&entry.request_body)),
                (
                    "status",
                    entry
                        .status
                        .map(|s| num(u64::from(s)))
                        .unwrap_or(PluginValue::Null),
                ),
                ("responseHeaders", headers_json(&entry.response_headers)),
                (
                    "responseBytes",
                    entry
                        .response_bytes
                        .map(|n| num(n as u64))
                        .unwrap_or(PluginValue::Null),
                ),
                ("responsePreview", body_preview(&entry.response_preview)),
                (
                    "error",
                    entry
                        .error
                        .as_deref()
                        .map(|e| PluginValue::Str(e.to_owned()))
                        .unwrap_or(PluginValue::Null),
                ),
                (
                    "doneAt",
                    entry.done_ms.map(fnum).unwrap_or(PluginValue::Null),
                ),
            ])
        })
        .collect();
    Ok(map(vec![
        ("revision", num(guard.revision())),
        ("entries", PluginValue::List(entries)),
    ]))
}

fn phase_map(phase: PhaseSample) -> PluginValue {
    map(vec![
        ("motionMs", fnum(phase.motion_ms)),
        ("dispatchMs", fnum(phase.dispatch_ms)),
        ("timerMs", fnum(phase.timer_ms)),
        ("rafMs", fnum(phase.raf_ms)),
        ("callbackMs", fnum(phase.callback_ms)),
        ("flushMs", fnum(phase.flush_ms)),
        ("layoutCbMs", fnum(phase.layout_cb_ms)),
        ("syncMs", fnum(phase.sync_ms)),
        ("semanticsMs", fnum(phase.semantics_ms)),
        ("layworkMs", fnum(phase.laywork_ms)),
        ("passes", num(u64::from(phase.layout_passes))),
        ("reuse", num(u64::from(phase.text_reused))),
        ("shaped", num(u64::from(phase.text_shaped))),
        ("pointers", num(u64::from(phase.pointers))),
        ("keys", num(u64::from(phase.keys))),
        ("edits", num(u64::from(phase.edits))),
        ("ime", num(u64::from(phase.ime))),
        ("scrolls", num(u64::from(phase.scrolls))),
        ("focus", num(u64::from(phase.focus))),
        ("blur", num(u64::from(phase.blur))),
        ("canvasRecordMs", fnum(phase.canvas_record_ms)),
        ("canvasPrepareMs", fnum(phase.canvas_prepare_ms)),
        ("canvasOps", num(u64::from(phase.canvas_ops))),
        ("canvasSegments", num(u64::from(phase.canvas_segments))),
        ("canvasRedraws", num(u64::from(phase.canvas_redraws))),
        ("canvasReused", num(u64::from(phase.canvas_reused))),
    ])
}

fn frames_channel(hub: &DiagnosticsHub, args: &[PluginValue]) -> Result<PluginValue, String> {
    let since = since_arg(args);
    let guard = hub.frames.lock().unwrap_or_else(|p| p.into_inner());
    let entries: Vec<PluginValue> = guard
        .tail(since)
        .map(|frame| {
            map(vec![
                ("seq", num(frame.seq)),
                ("number", num(frame.number)),
                ("at", fnum(frame.at_ms)),
                ("sessionMs", fnum(frame.session_ms)),
                ("workMs", fnum(frame.work_ms)),
                ("uiMs", fnum(frame.schedule.ui_ms())),
                ("rasterMs", fnum(frame.schedule.raster_ms())),
                ("queueMs", fnum(frame.schedule.queue_ms())),
                (
                    "compositorOnly",
                    PluginValue::Bool(frame.schedule.compositor_only),
                ),
                (
                    "compositorBlock",
                    frame
                        .schedule
                        .compositor_block
                        .map_or(PluginValue::Null, |reason| {
                            PluginValue::Str(reason.name().into())
                        }),
                ),
                (
                    "compositorBlockNode",
                    frame
                        .schedule
                        .compositor_block
                        .and_then(engine::scene::CompositorBlockReason::node)
                        .map_or(PluginValue::Null, |id| PluginValue::Str(id.to_string())),
                ),
                ("e2eMs", fnum(frame.schedule.e2e_ms())),
                ("draws", num(u64::from(frame.raster.render.draws))),
                ("passes", num(u64::from(frame.raster.render.passes))),
                (
                    "gpuStatus",
                    PluginValue::Str(frame.gpu.status.name().into()),
                ),
                ("budgetMs", fnum(frame.budget_ms)),
                ("gestureMs", fnum(frame.gesture_ms)),
                ("scriptMs", fnum(frame.script_ms)),
                ("layoutMs", fnum(frame.layout_ms)),
                ("recordMs", fnum(frame.record_ms)),
                ("submitMs", fnum(frame.submit_ms)),
                ("presentMs", fnum(frame.present_ms)),
                ("layers", num(u64::from(frame.layers))),
                ("atlasEntries", num(u64::from(frame.atlas_entries))),
                ("pictures", num(u64::from(frame.pictures))),
                ("textures", num(u64::from(frame.textures))),
                ("shapesBuilt", num(u64::from(frame.shapes_built))),
                ("atlasHits", num(u64::from(frame.atlas_hits))),
                ("paintHit", num(u64::from(frame.shapes_paint_hit))),
                ("paintMiss", num(u64::from(frame.shapes_paint_miss))),
                ("pictureRebuilt", num(u64::from(frame.picture_rebuilt))),
                ("pictureReused", num(u64::from(frame.picture_reused))),
                (
                    "layoutCause",
                    PluginValue::Str(frame.layout_cause.to_owned()),
                ),
                ("gestures", num(u64::from(frame.gestures))),
                ("overBudget", PluginValue::Bool(frame.over_budget)),
                ("phases", phase_map(frame.phases)),
            ])
        })
        .collect();
    Ok(map(vec![
        ("revision", num(guard.revision())),
        ("entries", PluginValue::List(entries)),
    ]))
}

fn gpu_timing(hub: &DiagnosticsHub, args: &[PluginValue]) -> Result<PluginValue, String> {
    let number = since_arg(args);
    let timing = hub
        .frames
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .tail(0)
        .find(|frame| frame.number == number)
        .and_then(|frame| frame.gpu_timing.clone());
    let Some(timing) = timing else {
        return Ok(PluginValue::Null);
    };
    let passes = timing
        .passes
        .iter()
        .map(|pass| {
            map(vec![
                ("kind", PluginValue::Str(format!("{:?}", pass.kind))),
                (
                    "sliceIndex",
                    pass.slice_index
                        .map_or(PluginValue::Null, |i| num(i as u64)),
                ),
                ("draws", num(u64::from(pass.draws))),
                ("startUs", fnum(pass.start_us)),
                ("durationUs", fnum(pass.duration_us)),
            ])
        })
        .collect();
    Ok(map(vec![
        ("frameSpanUs", fnum(timing.frame_span_us)),
        ("passesSumUs", fnum(timing.passes_sum_us)),
        ("passes", PluginValue::List(passes)),
    ]))
}

fn start_trace(hub: &Arc<DiagnosticsHub>) -> Result<PluginValue, String> {
    let at = super::telemetry::mono_ns();
    let path =
        std::env::temp_dir().join(format!("impasto-trace-{}-{at}.pftrace", std::process::id()));
    hub.start_trace(path, false)
        .map(num)
        .ok_or_else(|| "trace recording already active".into())
}

fn finish_trace(hub: &DiagnosticsHub, args: &[PluginValue]) -> Result<PluginValue, String> {
    hub.finish_trace(since_arg(args))
        .map(|path| PluginValue::Str(path.to_string_lossy().into_owned()))
        .map_err(|error| error.to_string())
}

fn edge_map(left: f32, top: f32, right: f32, bottom: f32) -> PluginValue {
    map(vec![
        ("left", fnum(f64::from(left))),
        ("top", fnum(f64::from(top))),
        ("right", fnum(f64::from(right))),
        ("bottom", fnum(f64::from(bottom))),
    ])
}

fn platform_method(hub: &DiagnosticsHub, context: &DevtoolsContext) -> Result<PluginValue, String> {
    let metrics = hub.metrics();
    let app_state = match context.app_state.lifecycle() {
        impasto::Lifecycle::Resumed => "active",
        impasto::Lifecycle::Inactive => "inactive",
        _ => "background",
    }
    .to_owned();
    Ok(map(vec![
        ("os", PluginValue::Str(context.os.to_owned())),
        ("osVersion", PluginValue::Str(context.os_version.clone())),
        ("scale", fnum(f64::from(metrics.device_pixel_ratio))),
        (
            "viewport",
            map(vec![
                ("w", fnum(f64::from(metrics.width))),
                ("h", fnum(f64::from(metrics.height))),
                ("dpr", fnum(f64::from(metrics.device_pixel_ratio))),
            ]),
        ),
        (
            "insets",
            edge_map(
                metrics.inset_left,
                metrics.inset_top,
                metrics.inset_right,
                metrics.inset_bottom,
            ),
        ),
        (
            "padding",
            edge_map(
                metrics.padding.left,
                metrics.padding.top,
                metrics.padding.right,
                metrics.padding.bottom,
            ),
        ),
        (
            "viewInsets",
            edge_map(
                metrics.view_insets.left,
                metrics.view_insets.top,
                metrics.view_insets.right,
                metrics.view_insets.bottom,
            ),
        ),
        ("appState", PluginValue::Str(app_state)),
        ("log", PluginValue::Str(crate::log_sub::log_filter_text())),
        ("devtools", PluginValue::Bool(context.enabled)),
        (
            "dump",
            context
                .dump
                .clone()
                .map(PluginValue::Str)
                .unwrap_or(PluginValue::Null),
        ),
    ]))
}

fn versions_method() -> Result<PluginValue, String> {
    Ok(map(vec![
        (
            "shell",
            PluginValue::Str(env!("CARGO_PKG_VERSION").to_owned()),
        ),
        ("wgpu", PluginValue::Str(locked_version("wgpu").to_owned())),
        (
            "parley",
            PluginValue::Str(locked_version("parley").to_owned()),
        ),
        ("quickjsPin", PluginValue::Str(quickjs_pin().to_owned())),
    ]))
}

fn locked_version(package: &str) -> &'static str {
    include_str!("../../../../../Cargo.lock")
        .split("[[package]]")
        .find_map(|entry| {
            let mut lines = entry.lines().filter(|line| !line.is_empty());
            let name = lines.next()?.strip_prefix("name = \"")?.strip_suffix('"')?;
            if name != package {
                return None;
            }
            lines
                .next()?
                .strip_prefix("version = \"")?
                .strip_suffix('"')
        })
        .unwrap_or("unknown")
}

fn quickjs_pin() -> &'static str {
    "quickjs-jit-0.14.2"
}

fn flags_pair(hub: &DiagnosticsHub) -> Result<PluginValue, String> {
    let flags = hub.flags();
    Ok(map(vec![
        ("layoutScan", PluginValue::Bool(flags.layout_scan)),
        ("pointerTrace", PluginValue::Bool(flags.pointer_trace)),
        (
            "suppressDispatch",
            PluginValue::Bool(flags.suppress_dispatch),
        ),
    ]))
}

fn set_flags(hub: &DiagnosticsHub, args: &[PluginValue]) -> Result<PluginValue, String> {
    let value = args.first().cloned().unwrap_or(PluginValue::Null);
    let PluginValue::Map(pairs) = value else {
        return Ok(PluginValue::Null);
    };
    let get = |key: &str| -> Option<bool> {
        pairs.iter().find_map(|(k, v)| match (k == key, v) {
            (true, PluginValue::Bool(b)) => Some(*b),
            _ => None,
        })
    };
    let mut flags = hub.flags();
    if let Some(v) = get("layoutScan") {
        flags.layout_scan = v;
    }
    if let Some(v) = get("pointerTrace") {
        flags.pointer_trace = v;
    }
    if let Some(v) = get("suppressDispatch") {
        flags.suppress_dispatch = v;
    }
    hub.set_flags(flags);
    Ok(PluginValue::Null)
}

fn clear_logs(hub: &DiagnosticsHub) -> Result<PluginValue, String> {
    hub.clear_logs();
    Ok(PluginValue::Null)
}

pub(crate) fn source(
    hub: Arc<DiagnosticsHub>,
    source_map: Option<Arc<String>>,
    context: PluginContext,
    os_version: String,
    dump: Option<String>,
) -> DiagnosticsSource {
    let context = DevtoolsContext {
        enabled: true,
        source_map,
        app_state: context,
        os: std::env::consts::OS,
        os_version,
        dump,
    };
    DiagnosticsSource::new(move |operation, args| match operation {
        "openSource" => open_source(args),
        "sourceMap" => Ok(context
            .source_map
            .as_deref()
            .cloned()
            .map_or(PluginValue::Null, PluginValue::Str)),
        "revision" => Ok(num(hub.revision())),
        "logs" => logs_channel(&hub, args),
        "errors" => errors_channel(&hub, args),
        "network" => network_channel(&hub, args),
        "frames" => frames_channel(&hub, args),
        "gpuTiming" => gpu_timing(&hub, args),
        "startTrace" => start_trace(&hub),
        "finishTrace" => finish_trace(&hub, args),
        "flags" => flags_pair(&hub),
        "setFlags" => set_flags(&hub, args),
        "clear" => clear_logs(&hub),
        "platform" => platform_method(&hub, &context),
        "versions" => versions_method(),
        _ => Err(format!("unknown diagnostics operation {operation}")),
    })
}

fn open_source(args: &[PluginValue]) -> Result<PluginValue, String> {
    let [
        PluginValue::Str(path),
        PluginValue::F64(line),
        PluginValue::F64(column),
    ] = args
    else {
        return Err("expected a source path, line and column".to_owned());
    };
    if !cfg!(target_os = "macos") {
        return Ok(PluginValue::Bool(false));
    }
    let (key, path) = if let Some(path) = path.strip_prefix("@framework/") {
        ("IMPASTO_DEVTOOLS_FRAMEWORK", path)
    } else {
        ("IMPASTO_DEVTOOLS_PROJECT", path.as_str())
    };
    let Some(root) = std::env::var_os(key).map(std::path::PathBuf::from) else {
        return Ok(PluginValue::Bool(false));
    };
    if ![line, column]
        .into_iter()
        .all(|number| number.fract() == 0. && (1. ..=f64::from(u32::MAX)).contains(number))
    {
        return Err("invalid source position".to_owned());
    }
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let file = root
        .join(path)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !file.starts_with(&root) || !file.is_file() {
        return Err("source must be inside the project".to_owned());
    }
    let mut command;
    if let Some(editor) = std::env::var_os("VISUAL") {
        command = std::process::Command::new(editor);
        command.arg(format!("{}:{line}:{column}", file.display()));
    } else {
        let scheme = match std::env::var("IMPASTO_EDITOR").as_deref() {
            Ok("cursor") => "cursor",
            Ok("zed") => "zed",
            _ => "vscode",
        };
        let absolute =
            url::Url::from_file_path(file).map_err(|()| "invalid source URL".to_owned())?;
        command = std::process::Command::new("open");
        command.arg(format!(
            "{scheme}://file{}:{line}:{column}",
            absolute.path()
        ));
    }
    command.spawn().map_err(|error| error.to_string())?;
    Ok(PluginValue::Bool(true))
}
