//! HTTP transport requests, responses and typed stream delivery.

#[cfg(target_os = "ios")]
use std::time::Duration;

use crate::trace::RequestTrace;
use crate::{HttpError as PublicError, NativeHttpEvent};
use impasto::abi::StreamSendError;
use impasto::{Bytes as JsBytes, CancelToken, ImpastoType, TypedSendError, TypedSink};

#[cfg(not(target_os = "ios"))]
mod openwire_transport;
#[cfg(not(target_os = "ios"))]
mod pinning;
#[cfg(not(target_os = "ios"))]
pub(crate) use openwire_transport::OpenWireClient;

pub const HTTP_BODY_LIMIT: usize = 8 * 1024 * 1024;
#[cfg(target_os = "ios")]
const BLOCKING_SEND_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ImpastoType)]
pub enum RedirectMode {
    #[default]
    Follow,
    Error,
    Manual,
}

#[derive(Clone)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub cancel: CancelToken,
    pub timeout_ms: Option<u64>,
    pub redirect: RedirectMode,
    pub max_body_bytes: Option<usize>,
    pub extra: RequestExtras,
}

#[derive(Clone, Default)]
pub struct RequestExtras {
    pub upload: Option<std::path::PathBuf>,
    pub progress: bool,
    #[cfg(not(target_os = "ios"))]
    pub upload_reporter: Option<std::sync::Arc<dyn Fn(u64, u64) + Send + Sync>>,
    pub cookies: Option<std::sync::Arc<crate::options::CookieJar>>,
    #[cfg(target_os = "ios")]
    pub pins: Option<std::sync::Arc<crate::options::Pins>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub url: String,
    pub redirected: bool,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpFailure {
    Network,
    Timeout,
    Canceled,
    TooLarge,
    BadUrl,
    TlsPinMismatch,
    Io,
}

impl HttpFailure {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Timeout => "timeout",
            Self::Canceled => "canceled",
            Self::TooLarge => "too-large",
            Self::BadUrl => "bad-url",
            Self::TlsPinMismatch => "tls-pin-mismatch",
            Self::Io => "io",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpError {
    pub kind: HttpFailure,
    pub message: String,
}

impl HttpError {
    pub fn new(kind: HttpFailure, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum HttpPiece<'a> {
    Head {
        status: u16,
        url: &'a str,
        redirected: bool,
        headers: &'a [(String, String)],
    },
    Chunk(&'a [u8], bool),
    #[cfg(target_os = "ios")]
    Upload {
        loaded: u64,
        total: Option<u64>,
    },
}

pub(crate) struct ResponseSink {
    stream: TypedSink<NativeHttpEvent>,
    trace: Option<RequestTrace>,
}

impl ResponseSink {
    pub(crate) fn new(stream: TypedSink<NativeHttpEvent>, trace: Option<RequestTrace>) -> Self {
        Self { stream, trace }
    }

    pub(crate) fn head(
        status: u16,
        url: &str,
        redirected: bool,
        headers: &[(String, String)],
    ) -> NativeHttpEvent {
        NativeHttpEvent::Head {
            status,
            url: url.to_owned(),
            redirected,
            headers: headers.to_vec(),
        }
    }

    fn max_chunk_bytes(&self) -> usize {
        self.stream
            .max_piece_bytes()
            .saturating_sub(128)
            .clamp(1, 64 * 1024)
    }

    fn map_send_error(err: TypedSendError) -> HttpError {
        match err {
            TypedSendError::Transport(StreamSendError::Closed) => {
                HttpError::new(HttpFailure::Canceled, "AbortError")
            }
            TypedSendError::Transport(StreamSendError::TooLarge) => {
                HttpError::new(HttpFailure::TooLarge, "response piece exceeds frame limit")
            }
            TypedSendError::Transport(StreamSendError::Backpressure) => HttpError::new(
                HttpFailure::Timeout,
                "response stream backpressure timed out",
            ),
            TypedSendError::Encode(message) => HttpError::new(HttpFailure::Network, message),
        }
    }

    #[cfg(not(target_os = "ios"))]
    async fn send_head_async(
        &mut self,
        status: u16,
        url: &str,
        redirected: bool,
        headers: &[(String, String)],
    ) -> Result<(), HttpError> {
        self.stream
            .send(Self::head(status, url, redirected, headers))
            .await
            .map_err(Self::map_send_error)?;
        if let Some(trace) = &mut self.trace {
            trace.head(status, headers);
        }
        Ok(())
    }

    #[cfg(not(target_os = "ios"))]
    async fn send_chunk_async(&mut self, bytes: &[u8]) -> Result<(), HttpError> {
        for bytes in bytes.chunks(self.max_chunk_bytes()) {
            self.stream
                .send(NativeHttpEvent::Chunk {
                    data: JsBytes(bytes.to_vec()),
                })
                .await
                .map_err(Self::map_send_error)?;
            if let Some(trace) = &mut self.trace {
                trace.chunk(bytes);
            }
        }
        Ok(())
    }

    #[cfg(target_os = "ios")]
    fn send_head_blocking(
        &mut self,
        status: u16,
        url: &str,
        redirected: bool,
        headers: &[(String, String)],
    ) -> Result<(), HttpError> {
        self.stream
            .send_blocking(
                Self::head(status, url, redirected, headers),
                BLOCKING_SEND_TIMEOUT,
            )
            .map_err(Self::map_send_error)?;
        if let Some(trace) = &mut self.trace {
            trace.head(status, headers);
        }
        Ok(())
    }

    #[cfg(target_os = "ios")]
    fn send_chunk_blocking(&mut self, bytes: &[u8]) -> Result<(), HttpError> {
        for bytes in bytes.chunks(self.max_chunk_bytes()) {
            self.stream
                .send_blocking(
                    NativeHttpEvent::Chunk {
                        data: JsBytes(bytes.to_vec()),
                    },
                    BLOCKING_SEND_TIMEOUT,
                )
                .map_err(Self::map_send_error)?;
            if let Some(trace) = &mut self.trace {
                trace.chunk(bytes);
            }
        }
        Ok(())
    }

    fn complete(mut self, result: Result<(), HttpError>) -> Result<(), PublicError> {
        if let Some(trace) = self.trace.take() {
            trace.finish(result.as_ref().err().map(|error| error.message.as_str()));
        }
        result.map_err(PublicError::from)
    }
}

pub(crate) trait HttpClient: Send + Sync + 'static {
    fn perform(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError>;

    fn stream(
        &self,
        request: &HttpRequest,
        sink: &mut dyn FnMut(HttpPiece<'_>) -> bool,
    ) -> Result<(), HttpError> {
        let response = self.perform(request)?;
        if !sink(HttpPiece::Head {
            status: response.status,
            url: &response.url,
            redirected: response.redirected,
            headers: &response.headers,
        }) {
            return Err(HttpError::new(HttpFailure::Canceled, "AbortError"));
        }
        sink(HttpPiece::Chunk(&response.body, true));
        Ok(())
    }
}

#[cfg(target_os = "ios")]
pub(crate) fn stream_blocking_to_sink<C: HttpClient + ?Sized>(
    client: &C,
    request: &HttpRequest,
    mut sink: ResponseSink,
) -> Result<(), PublicError> {
    let limit = request.max_body_bytes.unwrap_or(HTTP_BODY_LIMIT);
    let mut head_seen = false;
    let mut received: usize = 0;
    let mut failure: Option<HttpError> = None;

    let outcome = client.stream(request, &mut |piece| match piece {
        HttpPiece::Head {
            status,
            url,
            redirected,
            headers,
        } if !head_seen => {
            head_seen = true;
            if let Err(err) = sink.send_head_blocking(status, url, redirected, headers) {
                failure = Some(err);
                return false;
            }
            true
        }
        HttpPiece::Chunk(bytes, _) if head_seen => {
            received = received.saturating_add(bytes.len());
            if received > limit {
                failure = Some(HttpError::new(
                    HttpFailure::TooLarge,
                    "response body exceeds limit",
                ));
                return false;
            }
            if let Err(err) = sink.send_chunk_blocking(bytes) {
                failure = Some(err);
                return false;
            }
            true
        }
        _ => true,
    });

    let result = match failure {
        Some(error) => Err(error),
        None if !head_seen && outcome.is_ok() => {
            Err(HttpError::new(HttpFailure::Network, "no HTTP response"))
        }
        None => outcome,
    };
    sink.complete(result)
}
