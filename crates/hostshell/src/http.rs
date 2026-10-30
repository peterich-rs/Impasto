//! HTTP through [openwire](https://crates.io/crates/openwire).
//!
//! macOS and Android share this client. `net` blocks on `Call::execute`.
//! WebSocket stays outside openwire.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::BodyExt;
use openwire::{Client, RequestBody, WireErrorKind};
use session::{
    HTTP_BODY_LIMIT, HttpClient, HttpError, HttpFailure, HttpPiece, HttpRequest, HttpResponse,
};

/// openwire `Client` plus a handle to the process tokio runtime.
///
/// `client` and `handle` stay reachable so a later call can use openwire's
/// own `Call` options without growing a second HTTP stack.
pub struct OpenWireClient {
    handle: tokio::runtime::Handle,
    client: Client,
}

impl OpenWireClient {
    pub fn new() -> Result<Self, String> {
        let handle = crate::rt::handle()?;
        let client = Client::builder()
            .call_timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(20))
            .max_redirects(5)
            .max_decompressed_body_bytes(HTTP_BODY_LIMIT)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { handle, client })
    }

    /// Build the openwire request, start the cancel watcher, and execute the
    /// call. Returns the raw response plus the watcher handle so the body can
    /// keep streaming under the same cancellation.
    fn execute(
        &self,
        request: &HttpRequest,
    ) -> Result<
        (
            http::Response<openwire::ResponseBody>,
            tokio::task::JoinHandle<()>,
        ),
        HttpError,
    > {
        if request.cancel.load(Ordering::Acquire) {
            return Err(HttpError::new(HttpFailure::Canceled, "AbortError"));
        }
        if !(request.url.starts_with("http://") || request.url.starts_with("https://")) {
            return Err(HttpError::new(HttpFailure::BadUrl, "unsupported URL"));
        }
        let method = http::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| HttpError::new(HttpFailure::BadUrl, "unsupported URL"))?;
        let mut builder = http::Request::builder()
            .method(method.clone())
            .uri(&request.url);
        for (name, value) in &request.headers {
            if hop_by_hop(name) {
                continue;
            }
            builder = builder.header(name, value);
        }
        let body = if request.body.is_empty() {
            if method == http::Method::GET || method == http::Method::HEAD {
                RequestBody::absent()
            } else {
                RequestBody::explicit_empty()
            }
        } else {
            RequestBody::from_bytes(Bytes::copy_from_slice(&request.body))
        };
        let built = builder
            .body(body)
            .map_err(|error| HttpError::new(HttpFailure::BadUrl, error.to_string()))?;
        let call_timeout = request
            .timeout_ms
            .map(Duration::from_millis)
            .unwrap_or_else(|| Duration::from_secs(20));
        let call = self.client.new_call(built).call_timeout(call_timeout);
        let handle = call.handle();
        let cancel = Arc::clone(&request.cancel);
        let watcher = self.handle.spawn(async move {
            loop {
                if cancel.load(Ordering::Acquire) {
                    handle.cancel();
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        let executed = self.handle.block_on(call.execute()).map_err(|error| {
            watcher.abort();
            http_error(error)
        })?;
        if request.cancel.load(Ordering::Acquire) {
            watcher.abort();
            return Err(HttpError::new(HttpFailure::Canceled, "AbortError"));
        }
        Ok((executed, watcher))
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn runtime(&self) -> &tokio::runtime::Handle {
        &self.handle
    }
}

impl HttpClient for OpenWireClient {
    fn perform(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let (response, watcher) = self.execute(request)?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let payload = self.handle.block_on(read_limited(response.into_body()));
        watcher.abort();
        Ok(HttpResponse {
            status,
            headers,
            body: payload?,
        })
    }

    fn stream(
        &self,
        request: &HttpRequest,
        sink: &mut dyn FnMut(HttpPiece<'_>) -> bool,
    ) -> Result<(), HttpError> {
        let (response, watcher) = self.execute(request)?;
        let status = response.status().as_u16();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        if !sink(HttpPiece::Head(status, &headers)) {
            watcher.abort();
            return Err(HttpError::new(HttpFailure::Canceled, "AbortError"));
        }
        let mut received: usize = 0;
        let mut body = response.into_body();
        let mut failure: Option<HttpError> = None;
        loop {
            let frame = self.handle.block_on(body.frame());
            let Some(frame) = frame else {
                break;
            };
            let frame = match frame {
                Ok(frame) => frame,
                Err(error) => {
                    failure = Some(http_error(error));
                    break;
                }
            };
            let Some(data) = frame.data_ref() else {
                continue;
            };
            received = received.saturating_add(data.len());
            if received > HTTP_BODY_LIMIT {
                failure = Some(HttpError::new(HttpFailure::TooLarge, "TypeError"));
                break;
            }
            if !sink(HttpPiece::Chunk(data, false)) {
                watcher.abort();
                return Err(HttpError::new(HttpFailure::Canceled, "AbortError"));
            }
        }
        watcher.abort();
        if let Some(error) = failure {
            return Err(error);
        }
        if request.cancel.load(Ordering::Acquire) {
            return Err(HttpError::new(HttpFailure::Canceled, "AbortError"));
        }
        sink(HttpPiece::Chunk(&[], true));
        Ok(())
    }
}

fn hop_by_hop(name: &str) -> bool {
    name.eq_ignore_ascii_case("host")
        || name.eq_ignore_ascii_case("content-length")
        || name.eq_ignore_ascii_case("connection")
        || name.eq_ignore_ascii_case("transfer-encoding")
}

fn http_error(error: openwire::WireError) -> HttpError {
    let kind = match error.kind() {
        WireErrorKind::Canceled => HttpFailure::Canceled,
        WireErrorKind::Timeout => HttpFailure::Timeout,
        WireErrorKind::InvalidRequest => HttpFailure::BadUrl,
        _ => HttpFailure::Network,
    };
    HttpError::new(kind, error.to_string())
}

async fn read_limited(mut body: openwire::ResponseBody) -> Result<Vec<u8>, HttpError> {
    let mut out = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(http_error)?;
        let Some(data) = frame.data_ref() else {
            continue;
        };
        if out.len().saturating_add(data.len()) > HTTP_BODY_LIMIT {
            return Err(HttpError::new(HttpFailure::TooLarge, "TypeError"));
        }
        out.extend_from_slice(data);
    }
    Ok(out)
}

/// Shared client installed by the macOS and Android hosts.
pub fn http_client() -> Result<Arc<dyn HttpClient>, String> {
    Ok(Arc::new(OpenWireClient::new()?))
}
