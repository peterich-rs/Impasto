//! Wraps the host HTTP client so the devtools network page sees every call.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use engine::session::{CancelToken, MethodExec};

use super::hub::{DiagnosticsHub, NetEntry, NetKind, PREVIEW_CAP};
use crate::http::{
    HttpClient, HttpError, HttpFailure, HttpObserver, HttpPiece, HttpRecordEvent, HttpRequest,
    HttpResponse, ResponseSink,
};

/// Wraps `inner` so each call lands in the hub's network ring.
pub fn wrap_http(inner: Arc<dyn HttpClient>, hub: Arc<DiagnosticsHub>) -> Arc<dyn HttpClient> {
    Arc::new(RecordingHttpClient {
        inner,
        hub,
        next_token: AtomicU64::new(0),
    })
}

pub fn wrap_if_devtools(
    inner: Arc<dyn HttpClient>,
    hub: Option<Arc<DiagnosticsHub>>,
) -> Arc<dyn HttpClient> {
    match hub {
        Some(hub) => wrap_http(inner, hub),
        None => inner,
    }
}

struct RecordingHttpClient {
    inner: Arc<dyn HttpClient>,
    hub: Arc<DiagnosticsHub>,
    next_token: AtomicU64,
}

fn preview(body: &[u8]) -> Vec<u8> {
    body.iter().copied().take(PREVIEW_CAP).collect()
}

const SENSITIVE: [&str; 4] = [
    "authorization",
    "cookie",
    "set-cookie",
    "proxy-authorization",
];

fn redact(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            let redacted = if SENSITIVE.iter().any(|s| name.eq_ignore_ascii_case(s)) {
                "***".to_owned()
            } else {
                value.clone()
            };
            (name.clone(), redacted)
        })
        .collect()
}

impl RecordingHttpClient {
    fn alloc_token(&self) -> u64 {
        self.next_token.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn entry(&self, request: &HttpRequest, kind: NetKind) -> NetEntry {
        NetEntry {
            seq: 0,
            at_ms: crate::timing::mono_ms(),
            kind,
            token: self.alloc_token(),
            method: request.method.clone(),
            url: request.url.clone(),
            request_headers: redact(&request.headers),
            request_body: (!request.body.is_empty()).then(|| preview(&request.body)),
            status: None,
            response_headers: Vec::new(),
            response_bytes: None,
            response_preview: None,
            error: None,
            done_ms: None,
        }
    }
}

fn canceled(cancel: &CancelToken, error: Option<&HttpError>) -> bool {
    cancel.is_cancelled() || error.is_some_and(|error| error.kind == HttpFailure::Canceled)
}

struct StreamRecordState {
    entry: NetEntry,
    total_bytes: usize,
    preview_buf: Vec<u8>,
}

struct RecorderObserver {
    hub: Arc<DiagnosticsHub>,
    cancel: CancelToken,
    state: Mutex<Option<StreamRecordState>>,
}

impl RecorderObserver {
    fn flush(&self, error: Option<&HttpError>) {
        let mut guard = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let Some(mut state) = guard.take() else {
            return;
        };
        if canceled(&self.cancel, error) {
            return;
        }
        state.entry.response_bytes = Some(state.total_bytes);
        if !state.preview_buf.is_empty() {
            state.entry.response_preview = Some(state.preview_buf);
        }
        if let Some(error) = error {
            state.entry.error = Some(error.message.clone());
        }
        state.entry.done_ms = Some(crate::timing::mono_ms());
        self.hub.record_net(state.entry);
    }
}

impl HttpObserver for RecorderObserver {
    fn on_event(&self, event: &HttpRecordEvent) {
        match event {
            HttpRecordEvent::Head {
                status, headers, ..
            } => {
                let mut guard = self.state.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(state) = guard.as_mut() {
                    state.entry.status = Some(*status);
                    state.entry.response_headers = redact(headers);
                }
            }
            HttpRecordEvent::Chunk(bytes) => {
                let mut guard = self.state.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(state) = guard.as_mut() {
                    state.total_bytes = state.total_bytes.saturating_add(bytes.len());
                    if state.preview_buf.len() < PREVIEW_CAP {
                        let room = PREVIEW_CAP - state.preview_buf.len();
                        state
                            .preview_buf
                            .extend_from_slice(&bytes[..bytes.len().min(room)]);
                    }
                }
            }
            HttpRecordEvent::Complete => self.flush(None),
            HttpRecordEvent::Failed(error) => self.flush(Some(error)),
        }
    }
}

impl Drop for RecorderObserver {
    fn drop(&mut self) {
        let interrupted = HttpError::new(HttpFailure::Network, "stream interrupted");
        self.flush(Some(&interrupted));
    }
}

impl HttpClient for RecordingHttpClient {
    fn perform(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let mut entry = self.entry(request, NetKind::Http);
        let result = self.inner.perform(request);
        match &result {
            Ok(response) => {
                entry.status = Some(response.status);
                entry.response_headers = redact(&response.headers);
                entry.response_bytes = Some(response.body.len());
                entry.response_preview =
                    (!response.body.is_empty()).then(|| preview(&response.body));
            }
            Err(error) => {
                if canceled(&request.cancel, Some(error)) {
                    return result;
                }
                entry.error = Some(error.message.clone());
            }
        }
        entry.done_ms = Some(crate::timing::mono_ms());
        self.hub.record_net(entry);
        result
    }

    fn stream(
        &self,
        request: &HttpRequest,
        sink: &mut dyn FnMut(HttpPiece<'_>) -> bool,
    ) -> Result<(), HttpError> {
        let mut entry = self.entry(request, NetKind::Stream);
        let mut total = 0usize;
        let mut first = Vec::new();
        let result = self.inner.stream(request, &mut |piece| {
            match piece {
                HttpPiece::Head {
                    status, headers, ..
                } => {
                    entry.status = Some(status);
                    entry.response_headers = redact(headers);
                }
                HttpPiece::Chunk(bytes, _) => {
                    total = total.saturating_add(bytes.len());
                    if first.len() < PREVIEW_CAP {
                        let room = PREVIEW_CAP - first.len();
                        first.extend_from_slice(&bytes[..bytes.len().min(room)]);
                    }
                }
            }
            sink(piece)
        });
        if canceled(&request.cancel, result.as_ref().err()) {
            return result;
        }
        entry.response_bytes = Some(total);
        if !first.is_empty() {
            entry.response_preview = Some(first);
        }
        if let Err(error) = &result {
            entry.error = Some(error.message.clone());
        }
        entry.done_ms = Some(crate::timing::mono_ms());
        self.hub.record_net(entry);
        result
    }

    fn stream_to_sink(
        self: Arc<Self>,
        request: HttpRequest,
        sink: ResponseSink,
        exec: &dyn MethodExec,
    ) {
        let observer = Arc::new(RecorderObserver {
            hub: Arc::clone(&self.hub),
            cancel: request.cancel.clone(),
            state: Mutex::new(Some(StreamRecordState {
                entry: self.entry(&request, NetKind::Stream),
                total_bytes: 0,
                preview_buf: Vec::new(),
            })),
        });
        let sink = sink.with_observer(observer);
        Arc::clone(&self.inner).stream_to_sink(request, sink, exec);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpFailure, RedirectMode};

    fn request(url: &str, body: &[u8]) -> HttpRequest {
        HttpRequest {
            method: "POST".to_owned(),
            url: url.to_owned(),
            headers: vec![
                ("Authorization".to_owned(), "Bearer secret".to_owned()),
                ("X-Test".to_owned(), "1".to_owned()),
            ],
            body: body.to_vec(),
            cancel: CancelToken::new(),
            timeout_ms: None,
            redirect: RedirectMode::Follow,
            max_body_bytes: None,
        }
    }

    struct StubHttp;
    impl HttpClient for StubHttp {
        fn perform(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError> {
            Ok(HttpResponse {
                status: 200,
                url: request.url.clone(),
                redirected: false,
                headers: vec![
                    ("Set-Cookie".to_owned(), "session=abc".to_owned()),
                    ("Content-Type".to_owned(), "text/plain".to_owned()),
                ],
                body: b"hello world".to_vec(),
            })
        }

        fn stream(
            &self,
            request: &HttpRequest,
            sink: &mut dyn FnMut(HttpPiece<'_>) -> bool,
        ) -> Result<(), HttpError> {
            sink(HttpPiece::Head {
                status: 201,
                url: &request.url,
                redirected: false,
                headers: &[],
            });
            sink(HttpPiece::Chunk(b"one", false));
            sink(HttpPiece::Chunk(b"two", true));
            Ok(())
        }
    }

    #[test]
    fn perform_records_status_redacted_headers_and_preview() {
        let hub = Arc::new(DiagnosticsHub::new());
        let client = wrap_http(Arc::new(StubHttp), Arc::clone(&hub));
        let response = client
            .perform(&request("https://example.test/api", b"payload"))
            .expect("stub returns ok");
        assert_eq!(response.status, 200);
        let entry = hub.network.lock().unwrap().tail(0).next().cloned().unwrap();
        assert_eq!(entry.kind, NetKind::Http);
        assert_eq!(entry.method, "POST");
        assert_eq!(entry.status, Some(200));
        assert_eq!(entry.response_bytes, Some(11));
        assert_eq!(entry.response_preview.as_deref(), Some(&b"hello world"[..]));
        assert_eq!(
            entry.request_headers,
            vec![
                ("Authorization".to_owned(), "***".to_owned()),
                ("X-Test".to_owned(), "1".to_owned()),
            ]
        );
        assert_eq!(
            entry.response_headers,
            vec![
                ("Set-Cookie".to_owned(), "***".to_owned()),
                ("Content-Type".to_owned(), "text/plain".to_owned()),
            ]
        );
        assert!(entry.done_ms.is_some());
    }

    #[test]
    fn stream_records_the_joined_body() {
        let hub = Arc::new(DiagnosticsHub::new());
        let client = wrap_http(Arc::new(StubHttp), Arc::clone(&hub));
        client
            .stream(&request("https://example.test/stream", &[]), &mut |_| true)
            .expect("stub stream succeeds");
        let entry = hub.network.lock().unwrap().tail(0).next().cloned().unwrap();
        assert_eq!(entry.kind, NetKind::Stream);
        assert_eq!(entry.status, Some(201));
        assert_eq!(entry.response_bytes, Some(6));
        assert_eq!(entry.response_preview.as_deref(), Some(&b"onetwo"[..]));
    }

    #[test]
    fn a_canceled_call_is_not_recorded() {
        struct CanceledHttp;
        impl HttpClient for CanceledHttp {
            fn perform(&self, _request: &HttpRequest) -> Result<HttpResponse, HttpError> {
                Err(HttpError::new(HttpFailure::Canceled, "aborted"))
            }

            fn stream(
                &self,
                _request: &HttpRequest,
                _sink: &mut dyn FnMut(HttpPiece<'_>) -> bool,
            ) -> Result<(), HttpError> {
                Err(HttpError::new(HttpFailure::Canceled, "aborted"))
            }
        }
        let hub = Arc::new(DiagnosticsHub::new());
        let client = wrap_http(Arc::new(CanceledHttp), Arc::clone(&hub));
        let req = request("https://example.test/abort", &[]);
        req.cancel.cancel();
        let error = client.perform(&req).expect_err("canceled");
        assert_eq!(error.kind, HttpFailure::Canceled);
        assert!(hub.network.lock().unwrap().tail(0).next().is_none());
    }
}
