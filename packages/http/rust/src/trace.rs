use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use impasto::abi::PluginValue;
use impasto::{Bytes, DiagnosticsSink, ImpastoType, IntoPluginValue};

use crate::wire::HttpRequest;

const PREVIEW_BYTES: usize = 2048;
static SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(ImpastoType)]
struct Entry {
    token: String,
    method: String,
    url: String,
    request_headers: Vec<(String, String)>,
    request_body: Option<Bytes>,
    status: Option<u16>,
    response_headers: Vec<(String, String)>,
    response_bytes: Option<u64>,
    response_preview: Option<Bytes>,
    error: Option<String>,
    elapsed_ms: f64,
}

pub(crate) struct RequestTrace {
    sink: Arc<dyn DiagnosticsSink>,
    start: Instant,
    entry: Entry,
}

impl RequestTrace {
    pub(crate) fn new(sink: Arc<dyn DiagnosticsSink>, request: &HttpRequest) -> Self {
        Self {
            sink,
            start: Instant::now(),
            entry: Entry {
                token: SEQUENCE.fetch_add(1, Ordering::Relaxed).to_string(),
                method: request.method.clone(),
                url: request.url.clone(),
                request_headers: redact(&request.headers),
                request_body: (!request.body.is_empty())
                    .then(|| Bytes(request.body.iter().copied().take(PREVIEW_BYTES).collect())),
                status: None,
                response_headers: Vec::new(),
                response_bytes: None,
                response_preview: None,
                error: None,
                elapsed_ms: 0.0,
            },
        }
    }

    pub(crate) fn head(&mut self, status: u16, headers: &[(String, String)]) {
        self.entry.status = Some(status);
        self.entry.response_headers = redact(headers);
        self.entry.response_bytes = Some(0);
    }

    pub(crate) fn chunk(&mut self, data: &[u8]) {
        let size = self.entry.response_bytes.get_or_insert(0);
        *size = size.saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX));
        let preview = &mut self
            .entry
            .response_preview
            .get_or_insert_with(|| Bytes(Vec::new()))
            .0;
        preview.extend(
            data.iter()
                .copied()
                .take(PREVIEW_BYTES.saturating_sub(preview.len())),
        );
    }

    pub(crate) fn finish(mut self, error: Option<&str>) {
        self.entry.error = error.map(str::to_owned);
        self.entry.elapsed_ms = self.start.elapsed().as_secs_f64() * 1_000.0;
        if let Ok(payload @ PluginValue::Map(_)) = self.entry.into_value() {
            self.sink.record("http", "network", payload);
        }
    }
}

fn redact(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            let redacted = matches!(
                name.to_ascii_lowercase().as_str(),
                "authorization" | "proxy-authorization" | "cookie" | "set-cookie"
            );
            (
                name.clone(),
                if redacted {
                    "***".into()
                } else {
                    value.clone()
                },
            )
        })
        .collect()
}
