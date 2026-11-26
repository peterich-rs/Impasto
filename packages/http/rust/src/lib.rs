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
mod trace;
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
            method,
            url: self.url,
            headers: self.headers.unwrap_or_default(),
            body: self.body.map_or_else(Vec::new, |body| body.0),
            cancel,
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
        method: request.method,
        url: request.url,
        headers: request.headers,
        body: request.body,
        cancel,
        timeout_ms: request.timeout_ms,
        redirect: RedirectMode::Follow,
        max_body_bytes: Some(request.max_body_bytes),
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
    #[cfg(target_os = "ios")]
    runtime: impasto::Runtime,
    trace: Option<Arc<dyn DiagnosticsSink>>,
    #[cfg(not(target_os = "ios"))]
    wire: Arc<OpenWireClient>,
}

impl HttpPlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        #[cfg(not(target_os = "ios"))]
        let wire = Arc::new(OpenWireClient::new().map_err(PluginInitError::new)?);
        #[cfg(not(target_os = "ios"))]
        let transport: Arc<dyn wire::HttpClient> = Arc::clone(&wire) as _;
        #[cfg(target_os = "ios")]
        let transport: Arc<dyn wire::HttpClient> = Arc::new(ios::Http);
        let send = Arc::clone(&transport);
        let stream = Arc::clone(&transport);
        let client = HttpClient::new(ctx.runtime().clone(), move |request, cancel| {
            let result = send.perform(&transport_request(request, cancel))?;
            Ok(HttpResponse {
                status: result.status,
                url: result.url,
                redirected: result.redirected,
                headers: result.headers,
                body: result.body,
            })
        })
        .with_stream(move |request, cancel, mut output| {
            let request = transport_request(request, cancel);
            let result = stream.stream(&request, &mut |piece| {
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
                    HttpPiece::Chunk(bytes, last) => {
                        if last && bytes.is_empty() {
                            return true;
                        }
                        HttpChunk::Bytes(bytes.to_vec())
                    }
                };
                pollster::block_on(output.send(Ok(piece)))
            });
            if let Err(error) = result {
                let _ = pollster::block_on(output.send(Err(error.into())));
            }
        });
        ctx.insert_extension(client);
        Ok(Self {
            #[cfg(target_os = "ios")]
            runtime: ctx.runtime().clone(),
            trace: ctx.diagnostics_sink(),
            #[cfg(not(target_os = "ios"))]
            wire,
        })
    }
}

#[impasto::export]
impl HttpPlugin {
    pub async fn send(
        &self,
        request: NativeHttpRequest,
        sink: TypedSink<NativeHttpEvent>,
        cancel: CancelToken,
    ) -> Result<(), HttpError> {
        let request = request.request(cancel)?;
        let trace = self
            .trace
            .as_ref()
            .map(|sink| trace::RequestTrace::new(Arc::clone(sink), &request));
        let sink = ResponseSink::new(sink, trace);
        #[cfg(target_os = "ios")]
        {
            self.runtime
                .spawn_blocking(move || wire::stream_blocking_to_sink(&ios::Http, &request, sink))
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
