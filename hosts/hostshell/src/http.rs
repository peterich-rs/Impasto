//! HTTP through [openwire](https://crates.io/crates/openwire).
//!
//! macOS and Android share this client. `uifw-net` blocks on `Call::execute`.
//! WebSocket stays outside openwire.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::BodyExt;
use openwire::{Client, RequestBody, WireErrorKind};
use session::{HTTP_BODY_LIMIT, HttpClient, HttpError, HttpRequest, HttpResponse};

/// openwire `Client` plus the tokio runtime that polls it.
///
/// `client` and `runtime` stay reachable so a later call can use openwire's
/// own `Call` options without growing a second HTTP stack.
pub struct OpenWireClient {
    runtime: tokio::runtime::Runtime,
    client: Client,
}

impl OpenWireClient {
    pub fn new() -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("uifw-http")
            .build()
            .map_err(|error| error.to_string())?;
        let client = Client::builder()
            .call_timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(20))
            .max_redirects(5)
            .max_decompressed_body_bytes(HTTP_BODY_LIMIT)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { runtime, client })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn runtime(&self) -> &tokio::runtime::Runtime {
        &self.runtime
    }
}

impl HttpClient for OpenWireClient {
    fn perform(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError> {
        if request.cancel.load(Ordering::Acquire) {
            return Err(HttpError::new("AbortError"));
        }
        if !(request.url.starts_with("http://") || request.url.starts_with("https://")) {
            return Err(HttpError::new("unsupported URL"));
        }
        let method = http::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| HttpError::new("unsupported URL"))?;
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
            .map_err(|error| HttpError::new(error.to_string()))?;
        let call = self.client.new_call(built);
        let handle = call.handle();
        let cancel = Arc::clone(&request.cancel);
        let watcher = self.runtime.spawn(async move {
            loop {
                if cancel.load(Ordering::Acquire) {
                    handle.cancel();
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        let executed = self.runtime.block_on(call.execute());
        watcher.abort();
        if request.cancel.load(Ordering::Acquire) {
            return Err(HttpError::new("AbortError"));
        }
        let response = executed.map_err(http_error)?;
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
        let payload = self.runtime.block_on(read_limited(response.into_body()))?;
        Ok(HttpResponse {
            status,
            headers,
            body: payload,
        })
    }
}

fn hop_by_hop(name: &str) -> bool {
    name.eq_ignore_ascii_case("host")
        || name.eq_ignore_ascii_case("content-length")
        || name.eq_ignore_ascii_case("connection")
        || name.eq_ignore_ascii_case("transfer-encoding")
}

fn http_error(error: openwire::WireError) -> HttpError {
    if error.kind() == WireErrorKind::Canceled {
        HttpError::new("AbortError")
    } else {
        HttpError::new(error.to_string())
    }
}

async fn read_limited(mut body: openwire::ResponseBody) -> Result<Vec<u8>, HttpError> {
    let mut out = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(http_error)?;
        let Some(data) = frame.data_ref() else {
            continue;
        };
        if out.len().saturating_add(data.len()) > HTTP_BODY_LIMIT {
            return Err(HttpError::new("TypeError"));
        }
        out.extend_from_slice(data);
    }
    Ok(out)
}

/// Shared client installed by the macOS and Android hosts.
pub fn http_client() -> Result<Arc<dyn HttpClient>, String> {
    Ok(Arc::new(OpenWireClient::new()?))
}
