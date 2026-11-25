use std::sync::Arc;

use futures_channel::mpsc;
use futures_util::{SinkExt as _, StreamExt as _};

use crate::{CancelToken, ImpastoError, Runtime, Task, TaskError, runtime::scoped};

#[derive(Clone, Debug)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout_ms: Option<u64>,
    pub max_body_bytes: usize,
}

impl HttpRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: "GET".into(),
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
            timeout_ms: Some(30_000),
            max_body_bytes: 8 * 1024 * 1024,
        }
    }
}

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub url: String,
    pub redirected: bool,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub struct HttpError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(&self.message)
    }
}

impl std::error::Error for HttpError {}

impl ImpastoError for HttpError {
    fn scope(&self) -> &'static str {
        "impasto:http"
    }
    fn code(&self) -> &'static str {
        self.code
    }
    fn message(&self) -> String {
        self.message.clone()
    }
}

#[derive(Debug)]
pub enum HttpChunk {
    Head {
        status: u16,
        url: String,
        redirected: bool,
        headers: Vec<(String, String)>,
    },
    Bytes(Vec<u8>),
}

pub struct HttpStream {
    items: mpsc::Receiver<Result<HttpChunk, HttpError>>,
    task: Task<()>,
}

impl HttpStream {
    pub async fn next(&mut self) -> Option<Result<HttpChunk, HttpError>> {
        self.items.next().await
    }
}

impl Drop for HttpStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[doc(hidden)]
pub struct HttpStreamSender {
    items: mpsc::Sender<Result<HttpChunk, HttpError>>,
    cancel: CancelToken,
}

impl HttpStreamSender {
    pub async fn send(&mut self, piece: Result<HttpChunk, HttpError>) -> bool {
        matches!(
            scoped(self.items.send(piece), self.cancel.clone(), None).await,
            Some(Ok(()))
        )
    }
}

type Perform = dyn Fn(HttpRequest, CancelToken) -> Result<HttpResponse, HttpError> + Send + Sync;
type Stream = dyn Fn(HttpRequest, CancelToken, HttpStreamSender) + Send + Sync;

#[derive(Clone)]
pub struct HttpClient {
    runtime: Runtime,
    perform: Arc<Perform>,
    stream: Option<Arc<Stream>>,
}

struct AbortTask<T>(Task<T>);

impl<T> Drop for AbortTask<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl HttpClient {
    pub fn new(
        runtime: Runtime,
        perform: impl Fn(HttpRequest, CancelToken) -> Result<HttpResponse, HttpError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            runtime,
            perform: Arc::new(perform),
            stream: None,
        }
    }

    #[doc(hidden)]
    pub fn with_stream(
        mut self,
        stream: impl Fn(HttpRequest, CancelToken, HttpStreamSender) + Send + Sync + 'static,
    ) -> Self {
        self.stream = Some(Arc::new(stream));
        self
    }

    pub async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let transport = Arc::clone(&self.perform);
        let mut task = AbortTask(
            self.runtime
                .spawn_blocking_with_cancel(move |token| transport(request, token)),
        );
        (&mut task.0).await.map_err(|error| HttpError {
            code: match error {
                TaskError::Cancelled => "canceled",
                TaskError::Unavailable => "unavailable",
            },
            message: error.to_string(),
        })?
    }

    pub fn stream(&self, request: HttpRequest) -> Result<HttpStream, HttpError> {
        let transport = self.stream.as_ref().cloned().ok_or_else(|| HttpError {
            code: "unavailable",
            message: "the host does not provide an HTTP stream".into(),
        })?;
        let (send, items) = mpsc::channel(4);
        let task = self.runtime.spawn_blocking_with_cancel(move |cancel| {
            transport(
                request,
                cancel.clone(),
                HttpStreamSender {
                    items: send,
                    cancel,
                },
            );
        });
        Ok(HttpStream { items, task })
    }
}
