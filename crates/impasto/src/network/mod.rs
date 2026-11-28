//! Shared network observations submitted through an application diagnostics sink.

#[cfg(feature = "network-observation")]
mod openwire;
#[cfg(feature = "network-observation")]
pub use openwire::{OpenWireHop, OpenWireObserver};

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;

use crate::{Bytes, DiagnosticsSink, ImpastoType, IntoPluginValue};

/// Largest captured request or response. Previews remain available for larger bodies.
pub const BODY_LIMIT: usize = 1024 * 1024;
const PREVIEW_LIMIT: usize = 512;

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Allocates a process-local identity shared across native producers.
pub fn next_token() -> u64 {
    NEXT_TOKEN.fetch_add(1, Ordering::Relaxed)
}

/// Nine phases in milliseconds. Optional values represent unobserved phases.
#[derive(Clone, Debug, Default, Serialize, ImpastoType)]
pub struct NetworkTiming {
    pub queue_ms: Option<f64>,
    pub proxy_ms: Option<f64>,
    pub dns_ms: Option<f64>,
    pub tcp_ms: Option<f64>,
    pub tls_ms: Option<f64>,
    pub request_sent_ms: Option<f64>,
    pub waiting_ttfb_ms: Option<f64>,
    pub download_ms: Option<f64>,
    pub total_ms: f64,
}

/// Connection information reported by the transport.
#[derive(Clone, Debug, Default, Serialize, ImpastoType)]
pub struct ConnectionMeta {
    pub reused: bool,
    pub protocol: Option<String>,
    pub remote_addr: Option<String>,
    pub resolved_ips: Vec<String>,
    pub tls_version: Option<String>,
    pub tls_cipher: Option<String>,
    pub alpn: Option<String>,
}

/// One completed HTTP transaction in a redirect chain.
#[derive(Clone, Debug, Serialize, ImpastoType)]
pub struct RedirectHop {
    pub index: u32,
    pub method: String,
    pub url: String,
    pub status: u16,
    pub location: Option<String>,
    pub timing: NetworkTiming,
    pub connection: ConnectionMeta,
}

/// Per-request transport state shared with delegate callbacks and redirect calls.
#[derive(Clone, Debug, Default)]
pub struct HopLog(Arc<Mutex<Vec<RedirectHop>>>);

impl HopLog {
    /// Appends one transaction in request order.
    pub fn add(&self, mut hop: RedirectHop) {
        let mut hops = self.0.lock().unwrap_or_else(|error| error.into_inner());
        hop.index = u32::try_from(hops.len()).unwrap_or(u32::MAX);
        hops.push(hop);
    }

    fn take(&self) -> Vec<RedirectHop> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|error| error.into_inner()))
    }
}

/// Typed envelope shared by the SDK producers and the host's diagnostics sink.
#[doc(hidden)]
#[derive(Clone, Default, ImpastoType)]
pub struct NetworkEvent {
    pub token: String,
    pub initiator: Option<String>,
    pub method: Option<String>,
    pub url: Option<String>,
    pub final_url: Option<String>,
    pub redirected: Option<bool>,
    pub request_headers: Option<Vec<(String, String)>>,
    pub request_body: Option<Bytes>,
    pub request_body_full: Option<Bytes>,
    pub request_body_truncated: Option<bool>,
    pub status: Option<u16>,
    pub response_headers: Option<Vec<(String, String)>>,
    pub response_bytes: Option<u64>,
    pub response_preview: Option<Bytes>,
    pub response_body_full: Option<Bytes>,
    pub response_body_truncated: Option<bool>,
    pub error: Option<String>,
    pub canceled: Option<bool>,
    pub elapsed_ms: Option<f64>,
    pub head_elapsed_ms: Option<f64>,
    pub timing: Option<NetworkTiming>,
    pub connection: Option<ConnectionMeta>,
    pub redirects: Option<Vec<RedirectHop>>,
}

impl NetworkEvent {
    pub fn emit(self, sink: &dyn DiagnosticsSink, source: &str, event: &str) {
        if let Ok(payload) = self.into_value() {
            sink.record(source, event, payload);
        }
    }
}

/// A borrowed request descriptor. The observation owns its bounded previews.
pub struct RequestInfo<'a> {
    pub initiator: &'static str,
    pub method: &'a str,
    pub url: &'a str,
    pub headers: &'a [(String, String)],
    pub body: &'a [u8],
}

/// Emits each stage once, including cancellation when the owner drops a transfer.
pub struct RequestTrace {
    sink: Arc<dyn DiagnosticsSink>,
    start: Instant,
    token: String,
    hops: HopLog,
    body: Option<Vec<u8>>,
    preview: Vec<u8>,
    response_bytes: usize,
    finished: bool,
}

impl RequestTrace {
    /// Publishes a new pending network row.
    pub fn new(sink: Arc<dyn DiagnosticsSink>, request: RequestInfo<'_>) -> Self {
        let start = Instant::now();
        let token = next_token().to_string();
        let capture = request.initiator != "image";
        let bytes = request.body;
        NetworkEvent {
            token: token.clone(),
            initiator: Some(request.initiator.to_owned()),
            method: Some(request.method.to_owned()),
            url: Some(request.url.to_owned()),
            request_headers: Some(redact_headers(request.headers)),
            request_body: (!bytes.is_empty())
                .then(|| Bytes(bytes[..bytes.len().min(PREVIEW_LIMIT)].to_vec())),
            request_body_full: (capture && !bytes.is_empty())
                .then(|| Bytes(bytes[..bytes.len().min(BODY_LIMIT)].to_vec())),
            request_body_truncated: Some(bytes.len() > BODY_LIMIT),
            ..Default::default()
        }
        .emit(sink.as_ref(), "http", "network.start");
        Self {
            sink,
            start,
            token,
            hops: HopLog::default(),
            body: capture.then(Vec::new),
            preview: Vec::new(),
            response_bytes: 0,
            finished: false,
        }
    }

    /// Provides the transport's request-local hop collector.
    pub fn hops(&self) -> HopLog {
        self.hops.clone()
    }

    /// Updates the row when the final response begins streaming.
    pub fn head(&mut self, status: u16, url: &str, redirected: bool, headers: &[(String, String)]) {
        NetworkEvent {
            token: self.token.clone(),
            status: Some(status),
            final_url: Some(url.to_owned()),
            redirected: Some(redirected),
            response_headers: Some(redact_headers(headers)),
            head_elapsed_ms: Some(self.elapsed_ms()),
            ..Default::default()
        }
        .emit(self.sink.as_ref(), "http", "network.head");
    }

    /// Records bounded samples as data is received.
    pub fn chunk(&mut self, bytes: &[u8]) {
        self.response_bytes = self.response_bytes.saturating_add(bytes.len());
        let count = bytes
            .len()
            .min(PREVIEW_LIMIT.saturating_sub(self.preview.len()));
        self.preview.extend_from_slice(&bytes[..count]);
        if let Some(body) = &mut self.body {
            let count = bytes.len().min(BODY_LIMIT.saturating_sub(body.len()));
            body.extend_from_slice(&bytes[..count]);
        }
    }

    /// Completes the observation with the transfer's terminal result.
    pub fn finish(mut self, error: Option<&str>, canceled: bool) {
        self.emit_finish(error, canceled);
    }

    fn elapsed_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    fn emit_finish(&mut self, error: Option<&str>, canceled: bool) {
        self.finished = true;
        let redirects = self.hops.take();
        let (mut timing, connection) = redirects.last().map_or_else(
            || (NetworkTiming::default(), None),
            |hop| (hop.timing.clone(), Some(hop.connection.clone())),
        );
        let elapsed = self.elapsed_ms();
        timing.total_ms = elapsed;
        NetworkEvent {
            token: self.token.clone(),
            error: error.map(str::to_owned),
            canceled: Some(canceled),
            elapsed_ms: Some(elapsed),
            timing: Some(timing),
            connection,
            redirects: Some(redirects),
            response_bytes: u64::try_from(self.response_bytes).ok(),
            response_preview: Some(Bytes(std::mem::take(&mut self.preview))),
            response_body_full: self.body.take().filter(|body| !body.is_empty()).map(Bytes),
            response_body_truncated: Some(self.response_bytes > BODY_LIMIT),
            ..Default::default()
        }
        .emit(self.sink.as_ref(), "http", "network.finish");
    }
}

impl Drop for RequestTrace {
    fn drop(&mut self) {
        if !self.finished {
            self.emit_finish(Some("transfer canceled"), true);
        }
    }
}

/// Copies headers with credentials masked unless debug capture explicitly opts in.
pub fn redact_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    let raw = cfg!(debug_assertions)
        && std::env::var("IMPASTO_DEVTOOLS_RAW_HEADERS").as_deref() == Ok("1");
    headers
        .iter()
        .map(|(name, value)| {
            let sensitive = [
                "authorization",
                "proxy-authorization",
                "cookie",
                "set-cookie",
            ]
            .iter()
            .any(|key| name.eq_ignore_ascii_case(key));
            (
                name.clone(),
                if !raw && sensitive {
                    "***".to_owned()
                } else {
                    value.clone()
                },
            )
        })
        .collect()
}
