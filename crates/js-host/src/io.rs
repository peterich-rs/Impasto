//! Work that leaves the owning thread and comes back as an [`IoEvent`].
//!
//! Language globals such as `fetch` share this job list. Native module
//! promises settle through [`crate::IoEvent::Plugin`], not [`IoJob`].

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::PluginResult;

/// One HTTP call. `cancel` becomes true when the script aborts it.
/// `timeout_ms` of `None` leaves the client default in charge.
#[derive(Clone, Debug)]
pub struct HttpRequest {
    pub token: u64,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub cancel: Arc<AtomicBool>,
    pub timeout_ms: Option<u64>,
}

/// Bytes and headers from one completed response. The body is capped by the client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Why an HTTP call failed. Maps onto the DOMException names a script expects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpFailure {
    /// The request never reached a server, or the connection broke mid-body.
    Network,
    /// No response within the deadline.
    Timeout,
    /// The script aborted the call.
    Canceled,
    /// The body exceeded the client cap.
    TooLarge,
    /// The URL or method is not one this client serves.
    BadUrl,
    /// The body did not decode as the requested text encoding.
    BodyNotUtf8,
}

impl HttpFailure {
    /// The DOMException name the prelude rejects the promise with.
    pub fn dom_name(self) -> &'static str {
        match self {
            Self::Network => "TypeError",
            Self::Timeout => "TimeoutError",
            Self::Canceled => "AbortError",
            Self::TooLarge => "TypeError",
            Self::BadUrl => "TypeError",
            Self::BodyNotUtf8 => "TypeError",
        }
    }
}

/// A recoverable HTTP failure. The message is safe to reject a `fetch` promise with.
#[derive(Clone, Debug, PartialEq, Eq)]
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

    /// The DOMException name for this failure, so callers do not re-derive it.
    pub fn dom_name(&self) -> &'static str {
        self.kind.dom_name()
    }
}

/// One unit of work posted from the script. The host decides which thread
/// runs each job; every variant except `Abort` comes back as an [`IoEvent`].
#[derive(Clone, Debug)]
pub enum IoJob {
    /// Download the whole body, then deliver it as one [`IoEvent::Http`].
    Http(HttpRequest),
    /// Deliver the head as [`IoEvent::HttpHead`], each body piece as
    /// [`IoEvent::HttpChunk`], and the tail as the final chunk.
    HttpStream(HttpRequest),
    Abort {
        token: u64,
    },
    SocketOpen {
        id: u64,
        url: String,
        protocols: Vec<String>,
    },
    SocketSend {
        id: u64,
        text: bool,
        data: Vec<u8>,
    },
    SocketClose {
        id: u64,
    },
}

/// One streamed piece, handed to [`HttpClient::stream`]'s sink.
pub enum HttpPiece<'a> {
    Head(u16, &'a [(String, String)]),
    Chunk(&'a [u8], bool),
}

/// Blocking HTTP used by the host network thread and by tests.
pub trait HttpClient: Send + Sync {
    fn perform(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError>;

    /// Stream one response into `sink`. The sink returns false once the call
    /// is canceled; streaming must stop as soon as it does. The default
    /// reads the whole body through [`Self::perform`] and delivers it as a
    /// single chunk, so simple clients stream for free.
    fn stream(
        &self,
        request: &HttpRequest,
        sink: &mut dyn FnMut(HttpPiece<'_>) -> bool,
    ) -> Result<(), HttpError> {
        let response = self.perform(request)?;
        if sink(HttpPiece::Head(response.status, &response.headers)) && !response.body.is_empty() {
            sink(HttpPiece::Chunk(&response.body, true));
        }
        Ok(())
    }
}

/// Largest response body `fetch` will accept.
pub const HTTP_BODY_LIMIT: usize = 4 * 1024 * 1024;

/// One owned IO completion. The host chooses the thread that delivers it.
pub enum IoEvent {
    Http {
        token: u64,
        result: Result<HttpResponse, HttpError>,
    },
    /// Streamed head: status and headers before any body bytes.
    HttpHead {
        token: u64,
        status: u16,
        headers: Vec<(String, String)>,
    },
    /// One streamed body piece. The final chunk carries `done: true`; a
    /// failed stream ends with a chunk whose `error` is set.
    HttpChunk {
        token: u64,
        bytes: Vec<u8>,
        done: bool,
        error: Option<HttpError>,
    },
    Plugin {
        token: u64,
        result: PluginResult,
    },
    Socket {
        id: u64,
        event: String,
        text: bool,
        data: Vec<u8>,
    },
}
