//! Work that leaves the render thread and comes back as a [`PluginResult`].
//!
//! Language globals such as `fetch` and framework modules such as `Clipboard`
//! share this job list. The host decides which thread runs each job.

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
#[derive(Clone, Debug)]
pub struct HttpRequest {
    pub token: u64,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub cancel: Arc<AtomicBool>,
}

/// Bytes and headers from one completed response. The body is capped by the client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// A recoverable HTTP failure. The message is safe to reject a `fetch` promise with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpError {
    pub message: String,
}

impl HttpError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Blocking HTTP used by the host network thread and by session tests.
pub trait HttpClient: Send + Sync {
    fn perform(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError>;
}

/// One unit of work posted from the script.
#[derive(Clone, Debug)]
pub enum IoJob {
    Http(HttpRequest),
    Abort {
        token: u64,
    },
    Plugin {
        token: u64,
        name: String,
        method: String,
        args: Vec<PluginValue>,
    },
    SocketOpen {
        id: u64,
        url: String,
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

/// Largest response body `fetch` will accept.
pub const HTTP_BODY_LIMIT: usize = 4 * 1024 * 1024;
