//! Window-scoped HTTP for JavaScript and other Rust packages.

use std::sync::Arc;
#[cfg(not(target_os = "ios"))]
use std::sync::OnceLock;

use impasto::abi::PluginValue;
use impasto::{
    Bytes, CancelToken, DiagnosticsSink, FromPluginValue, ImpastoType, IntoPluginValue,
    PluginContext, PluginInitError, PluginRegistrar, TsTypes, TypedSink,
};

mod client;
#[cfg(target_os = "ios")]
#[expect(unsafe_code, reason = "Foundation URL loading delegate")]
mod ios;
mod options;
mod trace;
mod transfer;
mod wire;

pub use client::{HttpChunk, HttpClient, HttpError, HttpRequest, HttpResponse, HttpStream};
#[cfg(not(target_os = "ios"))]
use wire::OpenWireClient;
use wire::{HttpFailure, HttpPiece, RedirectMode, ResponseSink};

#[derive(ImpastoType)]
struct NativeHttpRequest {
    method: Option<String>,
    url: String,
    headers: Option<Vec<(String, String)>>,
    body: Option<Body>,
    timeout_ms: Option<u64>,
    redirect: Option<RedirectMode>,
    max_body_bytes: Option<u64>,
    progress: Option<bool>,
}

struct Body(Vec<u8>);

impl FromPluginValue for Body {
    fn from_value(value: PluginValue) -> Result<Self, String> {
        match value {
            PluginValue::Bytes(bytes) => Ok(Self(bytes)),
            PluginValue::Str(text) => Ok(Self(text.into_bytes())),
            _ => Err("HTTP request body must be a string or bytes".into()),
        }
    }
}

impl IntoPluginValue for Body {
    fn into_value(self) -> Result<PluginValue, String> {
        Ok(PluginValue::Bytes(self.0))
    }
}

impl ImpastoType for Body {
    fn typescript(_: &mut TsTypes) -> String {
        "Uint8Array | ArrayBuffer | string".into()
    }
}

#[derive(ImpastoType)]
#[impasto(tag = "type")]
enum NativeHttpEvent {
    Head {
        status: u16,
        url: String,
        redirected: bool,
        headers: Vec<(String, String)>,
    },
    Chunk {
        data: Bytes,
    },
    UploadProgress {
        loaded: u64,
        total: Option<u64>,
    },
    DownloadProgress {
        loaded: u64,
        total: Option<u64>,
    },
    File {
        bytes_written: u64,
        resumed: bool,
        path: String,
    },
}

impl NativeHttpRequest {
    fn request(self, cancel: CancelToken) -> Result<wire::HttpRequest, HttpError> {
        let method = self.method.unwrap_or_else(|| "GET".into());
        if method.is_empty() || self.url.is_empty() {
            return Err(HttpError {
                code: "bad-url",
                message: "HTTP URL and method must be non-empty".into(),
            });
        }
        Ok(wire::HttpRequest {
            hops: None,
            method,
            url: self.url,
            headers: self.headers.unwrap_or_default(),
            body: self.body.map_or_else(Vec::new, |body| body.0),
            cancel,
            extra: wire::RequestExtras {
                progress: self.progress.unwrap_or(false),
                ..Default::default()
            },
            timeout_ms: self.timeout_ms,
            redirect: self.redirect.unwrap_or_default(),
            max_body_bytes: self
                .max_body_bytes
                .map(|size| usize::try_from(size).unwrap_or(usize::MAX)),
        })
    }
}

impl From<wire::HttpError> for HttpError {
    fn from(error: wire::HttpError) -> Self {
        Self {
            code: error.kind.code(),
            message: error.message,
        }
    }
}

fn transport_request(request: HttpRequest, cancel: CancelToken) -> wire::HttpRequest {
    wire::HttpRequest {
        hops: None,
        method: request.method,
        url: request.url,
        headers: request.headers,
        body: request.body,
        cancel,
        timeout_ms: request.timeout_ms,
        redirect: RedirectMode::Follow,
        max_body_bytes: Some(request.max_body_bytes),
        extra: wire::RequestExtras::default(),
    }
}

fn unavailable(error: impl ToString) -> HttpError {
    HttpError {
        code: HttpFailure::Network.code(),
        message: error.to_string(),
    }
}

#[cfg(not(target_os = "ios"))]
fn runtime_handle() -> Result<tokio::runtime::Handle, String> {
    static RT: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("impasto-http")
            .enable_all()
            .build()
            .map_err(|error| error.to_string())
    })
    .as_ref()
    .map(|runtime| runtime.handle().clone())
    .map_err(Clone::clone)
}

/// Retrieves the HTTP client configured for this window session.
pub fn client(ctx: &PluginContext) -> Result<Arc<HttpClient>, HttpError> {
    ctx.extension()
        .ok_or_else(|| unavailable("HTTP is not initialized"))
}

#[impasto::module(name = "Http", specifier = "@impasto/http/native", init = init, object = true)]
struct HttpPlugin {
    context: PluginContext,
    registry: Clients,
    trace: Option<Arc<dyn DiagnosticsSink>>,
    #[cfg(not(target_os = "ios"))]
    wire: Arc<OpenWireClient>,
    #[cfg(target_os = "ios")]
    wire: Arc<ios::Http>,
}

type Clients = Arc<std::sync::Mutex<std::collections::BTreeMap<u64, Arc<Configured>>>>;

struct Configured {
    transport: Arc<dyn wire::HttpClient>,
    cookies: Option<Arc<options::CookieJar>>,
    #[cfg(target_os = "ios")]
    pins: Option<Arc<options::Pins>>,
}
impl Configured {
    fn apply(&self, request: &mut wire::HttpRequest) {
        request.extra.cookies = self.cookies.clone();
        #[cfg(target_os = "ios")]
        {
            request.extra.pins = self.pins.clone();
        }
    }
}

#[impasto::object]
struct NativeHttpClientHandle {
    id: u64,
    registry: Clients,
    state: Arc<Configured>,
}
impl Drop for NativeHttpClientHandle {
    fn drop(&mut self) {
        if let Ok(mut clients) = self.registry.lock() {
            clients.remove(&self.id);
        }
    }
}
#[impasto::export]
impl NativeHttpClientHandle {
    pub fn id(&self) -> u64 {
        self.id
    }
    #[impasto::method(blocking)]
    pub fn get_cookies(&self, url: String) -> Result<Vec<options::CookieEntry>, HttpError> {
        self.state
            .cookies
            .as_ref()
            .ok_or_else(|| options::error("io", "HTTP client has no Cookie Jar"))?
            .get(&url)
    }
    #[impasto::method(blocking)]
    pub fn set_cookie(&self, url: String, raw: String) -> Result<(), HttpError> {
        self.state
            .cookies
            .as_ref()
            .ok_or_else(|| options::error("io", "HTTP client has no Cookie Jar"))?
            .set(&url, &raw)
    }
    #[impasto::method(blocking)]
    pub fn clear_cookies(&self) -> Result<(), HttpError> {
        self.state
            .cookies
            .as_ref()
            .ok_or_else(|| options::error("io", "HTTP client has no Cookie Jar"))?
            .clear()
    }
}

impl HttpPlugin {
    fn default_transport(&self) -> Arc<dyn wire::HttpClient> {
        Arc::clone(&self.wire) as _
    }
    fn transport(
        &self,
        id: u64,
        request: &mut wire::HttpRequest,
    ) -> Result<Arc<dyn wire::HttpClient>, HttpError> {
        if id == 0 {
            return Ok(self.default_transport());
        }
        let clients = self
            .registry
            .lock()
            .map_err(|_| options::error("io", "HTTP client registry unavailable"))?;
        let client = clients
            .get(&id)
            .ok_or_else(|| options::error("io", "HTTP client is closed"))?;
        client.apply(request);
        Ok(Arc::clone(&client.transport))
    }
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        #[cfg(not(target_os = "ios"))]
        let wire = Arc::new(OpenWireClient::new().map_err(PluginInitError::new)?);
        #[cfg(target_os = "ios")]
        let wire =
            Arc::new(ios::Http::new(None).map_err(|error| PluginInitError::new(error.message))?);
        let transport: Arc<dyn wire::HttpClient> = Arc::clone(&wire) as _;
        let send = Arc::clone(&transport);
        let stream = Arc::clone(&transport);
        let diagnostics = ctx.diagnostics_sink();
        let send_trace = diagnostics.clone();
        let stream_trace = diagnostics.clone();
        let client = HttpClient::new(ctx.runtime().clone(), move |request, cancel| {
            let mut request = transport_request(request, cancel);
            let mut trace = trace::start(send_trace.as_ref(), &mut request, "rust-http");
            let result = if trace.is_some() {
                wire::collect_response(&request, |output| {
                    send.stream(&request, &mut |piece| {
                        trace::piece(&mut trace, &piece);
                        output(piece)
                    })
                })
            } else {
                send.perform(&request)
            };
            trace::finish(trace, &result);
            let result = result?;
            Ok(HttpResponse {
                status: result.status,
                url: result.url,
                redirected: result.redirected,
                headers: result.headers,
                body: result.body,
            })
        })
        .with_stream(move |request, cancel, mut output| {
            let mut request = transport_request(request, cancel);
            let mut trace = trace::start(stream_trace.as_ref(), &mut request, "rust-http");
            let result = stream.stream(&request, &mut |piece| {
                trace::piece(&mut trace, &piece);
                let piece = match piece {
                    HttpPiece::Head {
                        status,
                        url,
                        redirected,
                        headers,
                    } => HttpChunk::Head {
                        status,
                        url: url.into(),
                        redirected,
                        headers: headers.to_vec(),
                    },
                    #[cfg(target_os = "ios")]
                    HttpPiece::Upload { .. } => return true,
                    HttpPiece::Chunk(bytes, last) => {
                        if last && bytes.is_empty() {
                            return true;
                        }
                        HttpChunk::Bytes(bytes.to_vec())
                    }
                };
                pollster::block_on(output.send(Ok(piece)))
            });
            trace::finish(trace, &result);
            if let Err(error) = result {
                let _ = pollster::block_on(output.send(Err(error.into())));
            }
        });
        ctx.insert_extension(client);
        Ok(Self {
            context: ctx.clone(),
            registry: Arc::default(),
            trace: diagnostics,
            wire,
        })
    }
}

#[impasto::export]
impl HttpPlugin {
    pub fn is_http_host_available(&self) -> bool {
        self.context.is_hosted()
    }

    #[impasto::method(resource)]
    pub async fn configure_client(
        &self,
        config: options::ClientOptions,
    ) -> Result<NativeHttpClientHandle, HttpError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let ctx = self.context.clone();
        let registry = Arc::clone(&self.registry);
        tokio::task::spawn_blocking(move || {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let pins = config
                .pins
                .map(options::Pins::new)
                .transpose()?
                .map(Arc::new);
            #[cfg(target_os = "ios")]
            let transport: Arc<dyn wire::HttpClient> =
                Arc::new(ios::Http::new(config.proxy.as_ref())?);
            #[cfg(not(target_os = "ios"))]
            let transport: Arc<dyn wire::HttpClient> =
                Arc::new(OpenWireClient::with_config(config.proxy.as_ref(), pins)?);
            let cookies = config
                .cookie_jar
                .map(|options| options::CookieJar::new(&ctx, options))
                .transpose()?;
            let state = Arc::new(Configured {
                transport,
                cookies,
                #[cfg(target_os = "ios")]
                pins,
            });
            registry
                .lock()
                .map_err(|_| options::error("io", "HTTP client registry unavailable"))?
                .insert(id, Arc::clone(&state));
            Ok(NativeHttpClientHandle {
                id,
                registry,
                state,
            })
        })
        .await
        .map_err(unavailable)?
    }

    pub async fn send_configured(
        &self,
        id: u64,
        request: NativeHttpRequest,
        sink: TypedSink<NativeHttpEvent>,
        cancel: CancelToken,
    ) -> Result<(), HttpError> {
        let mut request = request.request(cancel)?;
        let transport = self.transport(id, &mut request)?;
        let diagnostics = self.trace.clone();
        tokio::task::spawn_blocking(move || {
            transfer::stream(&*transport, request, sink, diagnostics.as_ref())
        })
        .await
        .map_err(unavailable)?
    }
    pub async fn download(
        &self,
        id: u64,
        request: NativeHttpRequest,
        file: transfer::DownloadOptions,
        sink: TypedSink<NativeHttpEvent>,
        cancel: CancelToken,
    ) -> Result<(), HttpError> {
        let mut request = request.request(cancel)?;
        let transport = self.transport(id, &mut request)?;
        let ctx = self.context.clone();
        let diagnostics = self.trace.clone();
        tokio::task::spawn_blocking(move || {
            transfer::download(&ctx, &*transport, request, file, sink, diagnostics.as_ref())
        })
        .await
        .map_err(unavailable)?
    }
    pub async fn upload(
        &self,
        id: u64,
        request: NativeHttpRequest,
        file: transfer::UploadOptions,
        sink: TypedSink<NativeHttpEvent>,
        cancel: CancelToken,
    ) -> Result<(), HttpError> {
        let mut request = request.request(cancel)?;
        let transport = self.transport(id, &mut request)?;
        let ctx = self.context.clone();
        let diagnostics = self.trace.clone();
        tokio::task::spawn_blocking(move || {
            transfer::upload(&ctx, &mut request, file)?;
            transfer::stream(&*transport, request, sink, diagnostics.as_ref())
        })
        .await
        .map_err(unavailable)?
    }

    pub async fn send(
        &self,
        request: NativeHttpRequest,
        sink: TypedSink<NativeHttpEvent>,
        cancel: CancelToken,
    ) -> Result<(), HttpError> {
        let mut request = request.request(cancel)?;
        if request.extra.progress {
            let transport = self.default_transport();
            let diagnostics = self.trace.clone();
            return tokio::task::spawn_blocking(move || {
                transfer::stream(&*transport, request, sink, diagnostics.as_ref())
            })
            .await
            .map_err(unavailable)?;
        }
        let trace = trace::start(self.trace.as_ref(), &mut request, "fetch");
        let sink = ResponseSink::new(sink, trace);
        #[cfg(target_os = "ios")]
        {
            let transport = Arc::clone(&self.wire);
            self.context
                .runtime()
                .spawn_blocking(move || wire::stream_blocking_to_sink(&*transport, &request, sink))
                .await
                .map_err(unavailable)?
        }
        #[cfg(not(target_os = "ios"))]
        {
            self.wire.execute_to_sink(request, sink).await
        }
    }
}

/// Registers application HTTP and its session-local Rust client.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(HttpPlugin::plugin());
}
