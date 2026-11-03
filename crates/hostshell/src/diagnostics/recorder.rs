//! Wraps the host HTTP client so the devtools network page sees every call.

use std::sync::Arc;

use session::{HttpClient, HttpError, HttpPiece, HttpRequest, HttpResponse};

use super::hub::{DiagnosticsHub, NetEntry, NetKind, PREVIEW_CAP};

/// Wraps `inner` so each call lands in the hub's network ring.
pub fn wrap_http(inner: Arc<dyn HttpClient>, hub: Arc<DiagnosticsHub>) -> Arc<dyn HttpClient> {
    Arc::new(RecordingHttpClient { inner, hub })
}

/// `inner` unchanged when devtools are off; recorded through the hub when on.
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
            let masked = SENSITIVE.contains(&name.to_ascii_lowercase().as_str())
                || name.to_ascii_lowercase().starts_with("x-api-key");
            (
                name.clone(),
                if masked {
                    "***".to_owned()
                } else {
                    value.clone()
                },
            )
        })
        .collect()
}

impl RecordingHttpClient {
    fn entry(&self, request: &HttpRequest, kind: NetKind) -> NetEntry {
        NetEntry {
            seq: 0,
            at_ms: crate::net::mono_ms(),
            kind,
            token: request.token,
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

fn canceled(request: &HttpRequest, error: Option<&HttpError>) -> bool {
    request.cancel.load(std::sync::atomic::Ordering::Relaxed)
        || error.is_some_and(|error| error.kind == session::HttpFailure::Canceled)
}

impl HttpClient for RecordingHttpClient {
    fn perform(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let result = self.inner.perform(request);
        if canceled(request, result.as_ref().err()) {
            return result;
        }
        let mut entry = self.entry(request, NetKind::Http);
        match &result {
            Ok(response) => {
                entry.status = Some(response.status);
                entry.response_headers = redact(&response.headers);
                entry.response_bytes = Some(response.body.len());
                entry.response_preview = Some(preview(&response.body));
            }
            Err(error) => entry.error = Some(error.message.clone()),
        }
        entry.done_ms = Some(crate::net::mono_ms());
        self.hub.record_net(entry);
        result
    }

    fn stream(
        &self,
        request: &HttpRequest,
        sink: &mut dyn FnMut(HttpPiece<'_>) -> bool,
    ) -> Result<(), HttpError> {
        if request.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return self.inner.stream(request, sink);
        }
        let mut entry = self.entry(request, NetKind::Stream);
        let mut delivered = 0usize;
        let mut first: Vec<u8> = Vec::new();
        let outcome = self.inner.stream(request, &mut |piece| match piece {
            HttpPiece::Head(status, headers) => {
                entry.status = Some(status);
                entry.response_headers = redact(headers);
                true
            }
            HttpPiece::Chunk(bytes, done) => {
                delivered += bytes.len();
                if first.len() < PREVIEW_CAP {
                    let room = PREVIEW_CAP - first.len();
                    first.extend_from_slice(&bytes[..bytes.len().min(room)]);
                }
                if done {
                    entry.response_bytes = Some(delivered);
                    entry.response_preview = Some(first.clone());
                }
                sink(HttpPiece::Chunk(bytes, done))
            }
        });
        if canceled(request, outcome.as_ref().err()) {
            return outcome;
        }
        if let Some(error) = outcome.as_ref().err() {
            entry.error = Some(error.message.clone());
        }
        entry.done_ms = Some(crate::net::mono_ms());
        self.hub.record_net(entry);
        outcome
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use session::{HttpClient, HttpPiece, HttpRequest, HttpResponse};

    use super::*;
    use crate::diagnostics::NetKind;

    fn request(url: &str, body: &[u8]) -> HttpRequest {
        HttpRequest {
            token: 9,
            method: "POST".to_owned(),
            url: url.to_owned(),
            headers: vec![("Authorization".to_owned(), "Bearer secret".to_owned())],
            body: body.to_vec(),
            cancel: Arc::new(AtomicBool::new(false)),
            timeout_ms: None,
        }
    }

    struct Fixed;

    impl HttpClient for Fixed {
        fn perform(&self, _request: &HttpRequest) -> Result<HttpResponse, HttpError> {
            Ok(HttpResponse {
                status: 200,
                headers: vec![("Set-Cookie".to_owned(), "a=b".to_owned())],
                body: b"0123456789".to_vec(),
            })
        }

        fn stream(
            &self,
            _request: &HttpRequest,
            sink: &mut dyn FnMut(HttpPiece<'_>) -> bool,
        ) -> Result<(), HttpError> {
            sink(HttpPiece::Head(201, &[]));
            sink(HttpPiece::Chunk(b"one", false));
            sink(HttpPiece::Chunk(b"two", true));
            Ok(())
        }
    }

    #[test]
    fn perform_records_status_headers_and_preview() {
        let hub = Arc::new(DiagnosticsHub::new());
        let client = wrap_http(Arc::new(Fixed), Arc::clone(&hub));
        let response = client
            .perform(&request("https://example.test", b"payload"))
            .unwrap();
        assert_eq!(response.status, 200);
        let entry = hub.network.lock().unwrap().tail(0).next().cloned().unwrap();
        assert_eq!(entry.kind, NetKind::Http);
        assert_eq!(entry.status, Some(200));
        assert_eq!(
            entry.request_headers,
            [("Authorization".to_owned(), "***".to_owned())]
        );
        assert_eq!(
            entry.response_headers,
            [("Set-Cookie".to_owned(), "***".to_owned())]
        );
        assert_eq!(entry.request_body.as_deref(), Some(&b"payload"[..]));
        assert_eq!(entry.response_bytes, Some(10));
        assert_eq!(entry.response_preview.as_deref(), Some(&b"0123456789"[..]));
        assert!(entry.error.is_none());
        assert!(entry.done_ms.is_some());
    }

    #[test]
    fn stream_records_the_joined_body() {
        let hub = Arc::new(DiagnosticsHub::new());
        let client = wrap_http(Arc::new(Fixed), Arc::clone(&hub));
        let outcome = client.stream(&request("https://example.test", b""), &mut |_| true);
        assert!(outcome.is_ok());
        let entry = hub.network.lock().unwrap().tail(0).next().cloned().unwrap();
        assert_eq!(entry.kind, NetKind::Stream);
        assert_eq!(entry.status, Some(201));
        assert_eq!(entry.response_bytes, Some(6));
        assert_eq!(entry.response_preview.as_deref(), Some(&b"onetwo"[..]));
    }

    struct Abort;

    impl HttpClient for Abort {
        fn perform(&self, _request: &HttpRequest) -> Result<HttpResponse, HttpError> {
            Err(HttpError::new(session::HttpFailure::Canceled, "aborted"))
        }

        fn stream(
            &self,
            _request: &HttpRequest,
            _sink: &mut dyn FnMut(HttpPiece<'_>) -> bool,
        ) -> Result<(), HttpError> {
            Err(HttpError::new(session::HttpFailure::Canceled, "aborted"))
        }
    }

    #[test]
    fn a_canceled_call_is_not_recorded() {
        let hub = Arc::new(DiagnosticsHub::new());
        let client = wrap_http(Arc::new(Abort), Arc::clone(&hub));
        let error = client
            .perform(&request("https://example.test", b""))
            .unwrap_err();
        assert_eq!(error.kind, session::HttpFailure::Canceled);
        assert!(hub.network.lock().unwrap().tail(0).next().is_none());
    }
}
