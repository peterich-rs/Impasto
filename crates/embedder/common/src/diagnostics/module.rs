//! The `@impasto/devtools` host module. Sync reads over the hub.

use std::sync::Arc;

use engine::session::{Method, Module, ModuleExports, PluginValue};

use super::hub::{DiagnosticsHub, LogSource, PhaseSample};

const TYPESCRIPT: &str = r#"export type LogLevel = "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE";
export type LogSource = "script" | "layout" | "host" | "net";
export type LogLine = {
  seq: number;
  at: number;
  level: LogLevel;
  source: LogSource;
  target: string;
  text: string;
  frame: number | null;
};
export type DiagError = {
  seq: number;
  at: number;
  source: string;
  message: string;
  stack: string | null;
  frame: number | null;
};
export type NetEntry = {
  seq: number;
  at: number;
  kind: "http" | "stream" | "socket";
  token: number;
  method: string;
  url: string;
  requestHeaders: [string, string][];
  requestBody: string | null;
  status: number | null;
  responseHeaders: [string, string][];
  responseBytes: number | null;
  responsePreview: string | null;
  error: string | null;
  doneAt: number | null;
};
export type PhaseSample = {
  motionMs: number;
  dispatchMs: number;
  timerMs: number;
  rafMs: number;
  callbackMs: number;
  flushMs: number;
  layoutCbMs: number;
  syncMs: number;
  semanticsMs: number;
  layworkMs: number;
  passes: number;
  reuse: number;
  shaped: number;
  pointers: number;
  keys: number;
  edits: number;
  ime: number;
  scrolls: number;
  focus: number;
  blur: number;
  canvasRecordMs: number;
  canvasPrepareMs: number;
  canvasOps: number;
  canvasSegments: number;
  canvasRedraws: number;
  canvasReused: number;
};
export type FrameSample = {
  seq: number;
  number: number;
  at: number;
  workMs: number;
  budgetMs: number;
  sessionMs: number;
  gestureMs: number;
  scriptMs: number;
  layoutMs: number;
  recordMs: number;
  submitMs: number;
  presentMs: number;
  layers: number;
  atlasEntries: number;
  pictures: number;
  textures: number;
  shapesBuilt?: number;
  atlasHits?: number;
  paintHit?: number;
  paintMiss?: number;
  pictureRebuilt?: number;
  pictureReused?: number;
  layoutCause?: string;
  gestures?: number;
  overBudget: boolean;
  phases?: PhaseSample;
};

export type Page<T> = { revision: number; entries: T[] };

export type Flags = {
  layoutScan: boolean;
  pointerTrace: boolean;
  suppressDispatch: boolean;
};

type Insets = { left: number; top: number; right: number; bottom: number };

export type Platform = {
  os: string;
  osVersion: string;
  scale: number;
  viewport: { w: number; h: number; dpr: number };
  insets: Insets;
  padding: Insets;
  viewInsets: Insets;
  appState: string;
  log: string;
  devtools: boolean;
  dump: string | null;
};

export type Versions = { shell: string; wgpu: string; parley: string; quickjsPin: string };"#;

/// Extra host facts the hub does not own.
#[derive(Clone)]
pub struct DevtoolsContext {
    pub enabled: bool,
    /// Bundle source map text, when the host found `<js>.map` next to the script.
    pub source_map: Option<Arc<String>>,
    /// `AppState.currentState`, the same slot the app-state module writes.
    pub app_state: engine::session::Shared<String>,
    pub os: &'static str,
    pub os_version: String,
    /// `IMPASTO_DUMP` path, when set.
    pub dump: Option<String>,
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

fn logs_channel(hub: &Arc<DiagnosticsHub>) -> Method {
    let hub = Arc::clone(hub);
    Method::sync("logs", move |args| {
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
    })
    .with_types("since?: number", "Page<LogLine>")
}

fn errors_channel(hub: &Arc<DiagnosticsHub>) -> Method {
    let hub = Arc::clone(hub);
    Method::sync("errors", move |args| {
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
    })
    .with_types("since?: number", "Page<DiagError>")
}

fn network_channel(hub: &Arc<DiagnosticsHub>) -> Method {
    let hub = Arc::clone(hub);
    Method::sync("network", move |args| {
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
    })
    .with_types("since?: number", "Page<NetEntry>")
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

fn frames_channel(hub: &Arc<DiagnosticsHub>) -> Method {
    let hub = Arc::clone(hub);
    Method::sync("frames", move |args| {
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
    })
    .with_types("since?: number", "Page<FrameSample>")
}

fn edge_map(left: f32, top: f32, right: f32, bottom: f32) -> PluginValue {
    map(vec![
        ("left", fnum(f64::from(left))),
        ("top", fnum(f64::from(top))),
        ("right", fnum(f64::from(right))),
        ("bottom", fnum(f64::from(bottom))),
    ])
}

fn platform_method(hub: &Arc<DiagnosticsHub>, context: &DevtoolsContext) -> Method {
    let hub = Arc::clone(hub);
    let context = context.clone();
    Method::sync("platform", move |_| {
        let metrics = hub.metrics();
        let app_state = engine::session::shared_read(&context.app_state);
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
    })
    .with_types("", "Platform")
}

fn versions_method() -> Method {
    Method::sync("versions", |_| {
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
    })
    .with_types("", "Versions")
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

fn flags_pair(hub: &Arc<DiagnosticsHub>) -> Method {
    let hub = Arc::clone(hub);
    Method::sync("flags", move |_| {
        let flags = hub.flags();
        Ok(map(vec![
            ("layoutScan", PluginValue::Bool(flags.layout_scan)),
            ("pointerTrace", PluginValue::Bool(flags.pointer_trace)),
            (
                "suppressDispatch",
                PluginValue::Bool(flags.suppress_dispatch),
            ),
        ]))
    })
    .with_types("", "Flags")
}

fn set_flags(hub: &Arc<DiagnosticsHub>) -> Method {
    let hub = Arc::clone(hub);
    Method::sync("setFlags", move |args| {
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
    })
    .with_types("flags?: Partial<Flags>", "void")
}

fn clear_logs(hub: &Arc<DiagnosticsHub>) -> Method {
    let hub = Arc::clone(hub);
    Method::sync("clear", move |_| {
        hub.logs.lock().unwrap_or_else(|p| p.into_inner()).clear();
        hub.errors.lock().unwrap_or_else(|p| p.into_inner()).clear();
        Ok(PluginValue::Null)
    })
    .with_types("", "void")
}

/// Builds `@impasto/devtools`. Every method is a sync clone over the
/// hub's ring buffers, safe to call on the render thread.
pub fn module(hub: Arc<DiagnosticsHub>, context: DevtoolsContext) -> Module {
    Module {
        name: "Devtools",
        specifier: "@impasto/devtools",
        exports: ModuleExports {
            typescript: TYPESCRIPT,
            ..Default::default()
        },
        methods: vec![
            Method::sync("enabled", {
                let enabled = context.enabled;
                move |_| Ok(PluginValue::Bool(enabled))
            })
            .with_types("", "boolean"),
            Method::sync("sourceMap", {
                let context = context.clone();
                move |_| {
                    Ok(match context.source_map.as_deref() {
                        Some(text) => PluginValue::Str(text.clone()),
                        None => PluginValue::Null,
                    })
                }
            })
            .with_types("", "string | null"),
            Method::sync("revision", {
                let hub = Arc::clone(&hub);
                move |_| Ok(num(hub.revision()))
            })
            .with_types("", "number"),
            logs_channel(&hub),
            errors_channel(&hub),
            network_channel(&hub),
            frames_channel(&hub),
            flags_pair(&hub),
            set_flags(&hub),
            clear_logs(&hub),
            platform_method(&hub, &context),
            versions_method(),
            Method::sync("logSource", move |_| {
                // Source names are fixed; the panel uses them for filters.
                let names: Vec<PluginValue> = [
                    LogSource::Script,
                    LogSource::Layout,
                    LogSource::Host,
                    LogSource::Net,
                ]
                .iter()
                .map(|s| PluginValue::Str(s.name().to_owned()))
                .collect();
                Ok(PluginValue::List(names))
            })
            .with_types("", "LogSource[]"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::hub::{DiagError, LogEntry, NetEntry, NetKind, Ring};

    #[test]
    fn methods_are_sync_and_named_for_the_stub() {
        let module = module(Arc::new(DiagnosticsHub::new()), sample_context());
        assert_eq!(module.specifier, "@impasto/devtools");
        let names: Vec<&str> = module.methods.iter().map(|m| m.name).collect();
        assert_eq!(
            names,
            [
                "enabled",
                "sourceMap",
                "revision",
                "logs",
                "errors",
                "network",
                "frames",
                "flags",
                "setFlags",
                "clear",
                "platform",
                "versions",
                "logSource",
            ]
        );
    }

    fn sample_context() -> DevtoolsContext {
        DevtoolsContext {
            enabled: true,
            source_map: None,
            app_state: engine::session::shared("active".to_owned()),
            os: "test",
            os_version: "0".to_owned(),
            dump: None,
        }
    }

    #[test]
    fn log_entries_come_back_as_maps() {
        let hub = Arc::new(DiagnosticsHub::new());
        hub.push_log(LogEntry {
            seq: 0,
            at_ms: 0.0,
            level: tracing::Level::WARN,
            source: LogSource::Script,
            target: "impasto::script",
            message: "warm".to_owned(),
            fields: Vec::new(),
            frame: None,
        });
        let module = module(Arc::clone(&hub), sample_context());
        let method = module.methods.iter().find(|m| m.name == "logs").unwrap();
        let result = match &method.invoke {
            engine::session::Invoke::Sync(f) => f(&[PluginValue::F64(0.0)]).unwrap(),
            _ => panic!("logs must be sync"),
        };
        let text = format!("{result:?}");
        assert!(text.contains("warm"));
        assert!(text.contains("script"));
    }

    #[test]
    fn rings_are_sized_for_the_memory_budget() {
        assert_eq!(crate::diagnostics::hub::LOG_CAP, 400);
        assert_eq!(crate::diagnostics::hub::ERROR_CAP, 100);
        assert_eq!(crate::diagnostics::hub::NET_CAP, 150);
        assert_eq!(crate::diagnostics::hub::FRAME_CAP, 240);
        let _ = Ring::<LogEntry>::new(1);
        let _ = Ring::<DiagError>::new(1);
        let _ = Ring::<NetEntry>::new(1);
        let _ = (NetKind::Http, NetKind::Stream, NetKind::Socket);
    }
}
