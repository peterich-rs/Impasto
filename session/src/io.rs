//! Work that leaves the render thread and comes back as an [`IoEvent`].
//!
//! Language globals such as `fetch` share this job list. Native module
//! promises settle through [`crate::IoEvent::Plugin`], not [`IoJob`].

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// JSON-like value exchanged with one native module method.
#[derive(Clone, Debug, PartialEq)]
pub enum PluginValue {
    Null,
    Bool(bool),
    F64(f64),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<PluginValue>),
    Map(Vec<(String, PluginValue)>),
}

/// Completion of one token allocated for a promise.
#[derive(Clone, Debug, PartialEq)]
pub enum PluginResult {
    Done(PluginValue),
    Failed { message: String },
}

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

/// Blocking HTTP used by the host network thread and by session tests.
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

/// `module "<name>" is not linked`, or with the method when the name exists but the method does not.
pub fn module_not_linked(module: &str, method: Option<&str>) -> String {
    match method {
        Some(method) => format!("module \"{module}.{method}\" is not linked"),
        None => format!("module \"{module}\" is not linked"),
    }
}

/// A path relative to the document directory. Rejects `..`, absolute paths, and backslashes.
pub fn document_relative(path: &str) -> Result<String, String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains('\\')
        || path.contains(':')
    {
        return Err("path is outside the document directory".to_owned());
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err("path is outside the document directory".to_owned());
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::document_relative;

    #[test]
    fn document_relative_keeps_nested_files() {
        match document_relative("a/b.bin") {
            Ok(path) => assert_eq!(path, "a/b.bin"),
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn document_relative_rejects_escape() {
        for path in [
            "", "/", "\\a", "a\\b", "C:foo", "..", "a/../b", "a/./b", "/abs",
        ] {
            match document_relative(path) {
                Err(message) => {
                    assert!(message.contains("outside"), "{path}: {message}");
                }
                Ok(ok) => panic!("{path} should fail, got {ok}"),
            }
        }
    }
}
