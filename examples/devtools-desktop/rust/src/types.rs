use devtools_protocol::SessionInfo;
use impasto::ImpastoType;
use serde_json::Value;

use crate::discovery::TargetSummary;

#[derive(Clone, ImpastoType)]
pub struct SessionOverview {
    pub app_name: String,
    pub platform: String,
    pub os: String,
    pub framework_version: String,
    pub pid: u32,
    pub width: f64,
    pub height: f64,
    pub dpr: f64,
    pub refresh_rate: Option<f64>,
    pub topology: Option<String>,
}

impl From<&SessionInfo> for SessionOverview {
    fn from(info: &SessionInfo) -> Self {
        Self {
            app_name: info.app_name.clone(),
            platform: info.platform.clone(),
            os: info.os.clone(),
            framework_version: info.framework_version.clone(),
            pid: info.pid,
            width: f64::from(info.width),
            height: f64::from(info.height),
            dpr: f64::from(info.dpr),
            refresh_rate: info.refresh_rate.map(f64::from),
            topology: info.topology.clone(),
        }
    }
}

#[derive(Clone, Default, ImpastoType)]
pub struct FrameStats {
    pub count: u32,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub jank_count: u32,
    pub compositor_pct: f64,
    pub fps: f64,
}

#[derive(Clone, ImpastoType)]
pub struct TickSummary {
    pub state: String,
    pub session: Option<SessionOverview>,
    pub targets: Vec<TargetSummary>,
    pub target_id: Option<String>,
    pub capabilities: Vec<String>,
    pub message: Option<String>,
    pub generation: String,
    pub reconnect_attempt: u32,
    pub reconnected_new_pid: bool,
    pub paused: bool,
    pub rtt_ms: Option<f64>,
    pub profile: String,
    pub frames_rev: f64,
    pub repaints_rev: f64,
    pub network_rev: f64,
    pub logs_rev: f64,
    pub errors_rev: f64,
    pub runtime_rev: f64,
    pub resources_rev: f64,
    pub audits_rev: f64,
    pub frame_count: u32,
    pub latest_frame: Option<String>,
    pub network_count: u32,
    pub log_count: u32,
    pub error_count: u32,
    pub latest_error_seq: Option<String>,
    pub active_net_count: u32,
    pub js_heap_bytes: f64,
    pub stats: FrameStats,
    pub audit_count: u32,
    pub picked_node_id: Option<String>,
    pub picked_seq: Option<String>,
    pub active_capture: Option<String>,
    pub latest_capture: Option<String>,
    pub bundle_loaded: bool,
    pub symbols_loaded: bool,
}

#[derive(Clone, Default, ImpastoType)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub detail: String,
    pub meta: String,
    pub depth: u32,
    pub expandable: bool,
    pub at_ms: f64,
    pub count: f64,
}

#[derive(ImpastoType)]
pub struct RowSlice {
    pub total: u32,
    pub rows: Vec<Row>,
}

impl RowSlice {
    pub(crate) fn page<T>(
        rows: impl IntoIterator<Item = T>,
        offset: u32,
        limit: u32,
        mut row: impl FnMut(T) -> Row,
    ) -> Self {
        let end = offset.saturating_add(limit.min(256));
        let mut result = Self {
            total: 0,
            rows: Vec::new(),
        };
        for item in rows {
            if (offset..end).contains(&result.total) {
                result.rows.push(row(item));
            }
            result.total = result.total.saturating_add(1);
        }
        result
    }
}

#[derive(ImpastoType)]
pub struct DetailField {
    pub name: String,
    pub value: String,
    pub depth: u32,
}

pub(crate) fn fields(value: &Value) -> Vec<DetailField> {
    fn visit(value: &Value, name: &str, depth: u32, out: &mut Vec<DetailField>) {
        if out.len() >= 256 {
            return;
        }
        if depth < 4 {
            match value {
                Value::Object(values) if !values.is_empty() => {
                    for (key, child) in values.iter().take(256) {
                        visit(child, key, depth + 1, out);
                    }
                    return;
                }
                Value::Array(values) if !values.is_empty() => {
                    for (index, child) in values.iter().take(256).enumerate() {
                        visit(child, &format!("{name} {index}"), depth + 1, out);
                    }
                    return;
                }
                _ => {}
            }
        }
        let value = match value {
            Value::String(text) => clip(text, 1024),
            Value::Null => "—".to_owned(),
            Value::Array(values) => format!("{} entries", values.len()),
            Value::Number(value) if value.is_f64() => value
                .as_f64()
                .map(|number| format!("{number:.3}"))
                .unwrap_or_default(),
            Value::Object(values) => format!("{} fields", values.len()),
            _ => value.to_string(),
        };
        out.push(DetailField {
            name: name.replace('_', " "),
            value,
            depth: depth.saturating_sub(1),
        });
    }
    let mut result = Vec::new();
    visit(value, "", 0, &mut result);
    result
}

pub(crate) fn clip(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    format!("{}…", &text[..text.floor_char_boundary(limit)])
}

pub(crate) fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

pub(crate) fn number(value: &Value, key: &str) -> f64 {
    value[key].as_f64().unwrap_or_default()
}

pub(crate) fn id(value: &Value) -> String {
    match value {
        Value::String(id) => id.clone(),
        Value::Number(id) => id.to_string(),
        _ => String::new(),
    }
}

pub(crate) fn matches(text: &str, query: &str) -> bool {
    query.is_empty() || text.to_lowercase().contains(query)
}
