//! Internal adapter from plugin factories to a window's host services.

use std::sync::{Arc, OnceLock};

use impasto::{
    AppPaths, BlockingTask, FutureTask, PluginContext, PluginRegistrar, Register, Runtime,
    RuntimeBackend,
};
use serde_json::{Value, json};

use crate::{BootError, Command, TaskRunner, queue::Inbox};

struct Installed {
    register: Register,
    expected: &'static str,
}

static PLUGINS: OnceLock<Installed> = OnceLock::new();

/// Process entry points install their linked registrar before creating windows.
pub fn install_plugins(register: Register, expected: &'static str) -> Result<(), BootError> {
    PLUGINS
        .set(Installed { register, expected })
        .map_err(|_| BootError::Platform("native plugins are already installed".into()))
}

#[derive(Default)]
pub struct PluginServices {
    #[cfg(target_os = "android")]
    pub android: Option<impasto::os::android::AndroidContext>,
    pub paths: AppPaths,
    pub http: bool,
}

struct Backend {
    io: Arc<dyn TaskRunner>,
    main: Option<Arc<dyn TaskRunner>>,
}

impl RuntimeBackend for Backend {
    fn spawn(&self, task: FutureTask) {
        if let Ok(handle) = crate::rt::handle() {
            drop(handle.spawn(task));
        }
    }

    fn spawn_blocking(&self, task: BlockingTask) {
        self.io.post(task);
    }

    fn spawn_main(&self, task: BlockingTask) {
        if let Some(main) = &self.main {
            main.post(task);
        }
    }
}

pub(crate) struct PluginSession {
    context: PluginContext,
}

impl PluginSession {
    pub(crate) fn lifecycle(&self, state: engine::session::Lifecycle) {
        use engine::session::Lifecycle as Host;
        use impasto::Lifecycle;
        self.context.dispatch_lifecycle(match state {
            Host::Resumed => Lifecycle::Resumed,
            Host::Inactive => Lifecycle::Inactive,
            Host::Hidden => Lifecycle::Hidden,
            Host::Paused => Lifecycle::Paused,
            Host::Detached => Lifecycle::Detached,
        });
    }
}

impl Drop for PluginSession {
    fn drop(&mut self) {
        self.context
            .dispatch_lifecycle(impasto::Lifecycle::Detached);
    }
}

pub(crate) fn attach(
    modules: Vec<impasto::abi::Module>,
    services: PluginServices,
    http: Option<Arc<dyn crate::HttpClient>>,
    inbox: &Arc<Inbox>,
    io: Arc<dyn TaskRunner>,
    main: Option<Arc<dyn TaskRunner>>,
) -> Result<(impasto::abi::ModuleRegistry, Option<PluginSession>), BootError> {
    let Some(plugins) = PLUGINS.get() else {
        return Ok((impasto::abi::ModuleRegistry::new(modules), None));
    };
    let mut registrar = PluginRegistrar::default();
    (plugins.register)(&mut registrar);
    let descriptions = registrar.describe().map_err(plugin_error)?;
    let actual = descriptions
        .iter()
        .map(|entry| {
            json!({
                "name": entry.name,
                "specifier": entry.specifier,
                "exports": entry.exports,
                "typescript": entry.typescript,
            })
        })
        .collect::<Vec<_>>();
    let expected: Vec<Value> = serde_json::from_str(plugins.expected).map_err(plugin_error)?;
    if actual != expected {
        return Err(BootError::Platform(
            "linked plugin descriptors differ from the application bundle".into(),
        ));
    }
    let reserved = crate::first_party_modules();
    for entry in descriptions {
        if reserved
            .iter()
            .any(|module| module.name == entry.name || module.specifier == entry.specifier)
            || matches!(
                entry.specifier,
                "scene" | "reactive" | "#reactive" | "settings"
            )
        {
            return Err(BootError::Platform(format!(
                "native module {} uses a reserved name",
                entry.specifier
            )));
        }
    }
    let runtime = Runtime::new(Backend { io, main });
    let queue = Arc::downgrade(inbox);
    let mut context = PluginContext::new(runtime.clone(), services.paths, move |event| {
        let Some(queue) = queue.upgrade() else {
            return false;
        };
        queue.post_plugin_event(event)
    });
    let queue = Arc::downgrade(inbox);
    context = context.with_textures(impasto::ExternalTextureRegistry::new(move |name, frame| {
        let Some(queue) = queue.upgrade() else {
            return false;
        };
        let source = format!("external:{name}");
        match frame {
            #[cfg(target_os = "ios")]
            Some(impasto::TextureFrame::Metal { texture, fallback }) => {
                queue.post(Command::ExternalGpuTexture {
                    source,
                    frame: Some(metal(texture, fallback)),
                });
            }
            Some(impasto::TextureFrame::Rgba(frame)) => {
                let frame = Some(crate::ExternalFrame {
                    size: [frame.width, frame.height],
                    pixels: frame.pixels,
                });
                queue.post(Command::ExternalTexture { source, frame });
            }
            None => queue.post(Command::ExternalTexture {
                source,
                frame: None,
            }),
        }
        true
    }));
    #[cfg(target_os = "android")]
    if let Some(android) = services.android {
        context = context.with_android(android);
    }
    if let Some(http) = http.filter(|_| services.http) {
        let stream = Arc::clone(&http);
        let client = impasto::HttpClient::new(runtime, move |request, cancel| {
            let result = http
                .perform(&host_request(request, cancel))
                .map_err(http_error)?;
            Ok(impasto::HttpResponse {
                status: result.status,
                url: result.url,
                redirected: result.redirected,
                headers: result.headers,
                body: result.body,
            })
        })
        .with_stream(move |request, cancel, mut output| {
            // This closure runs in the host's blocking pool. Async workers
            // stay free while the producer waits for queue credit.
            let request = host_request(request, cancel);
            let result = stream.stream(&request, &mut |piece| {
                let piece = match piece {
                    crate::http::HttpPiece::Head {
                        status,
                        url,
                        redirected,
                        headers,
                    } => impasto::HttpChunk::Head {
                        status,
                        url: url.to_owned(),
                        redirected,
                        headers: headers.to_owned(),
                    },
                    crate::http::HttpPiece::Chunk(bytes, _) => {
                        impasto::HttpChunk::Bytes(bytes.to_vec())
                    }
                };
                pollster::block_on(output.send(Ok(piece)))
            });
            if let Err(error) = result {
                let _ = pollster::block_on(output.send(Err(http_error(error))));
            }
        });
        context = context.with_http(client);
    }
    let session = PluginSession { context };
    let extensions = registrar.build(&session.context).map_err(plugin_error)?;
    let registry =
        impasto::abi::ModuleRegistry::with_extensions(modules, extensions).map_err(plugin_error)?;
    Ok((registry, Some(session)))
}

fn plugin_error(error: impl std::fmt::Display) -> BootError {
    BootError::Platform(error.to_string())
}

fn host_request(request: impasto::HttpRequest, cancel: impasto::CancelToken) -> crate::HttpRequest {
    crate::HttpRequest {
        method: request.method,
        url: request.url,
        headers: request.headers,
        body: request.body,
        cancel,
        timeout_ms: request.timeout_ms,
        redirect: crate::RedirectMode::Follow,
        max_body_bytes: Some(request.max_body_bytes),
    }
}

fn http_error(error: crate::HttpError) -> impasto::HttpError {
    impasto::HttpError {
        code: error.kind.code(),
        message: error.message,
    }
}

#[cfg(target_os = "ios")]
#[expect(
    unsafe_code,
    reason = "the platform frame owns a finished and immutable Metal texture"
)]
fn metal(
    texture: impasto::MetalTexture,
    fallback: impasto::ExternalFrame,
) -> crate::ImportExternalFrame {
    Box::new(move |raster| {
        let lease = texture.clone();
        let size = [fallback.width, fallback.height];
        // SAFETY: the SDK constructor requires a ready texture whose backing
        // storage remains immutable. The HAL callback holds its lease.
        unsafe { raster.import_metal_texture(texture.as_ptr(), size, move || drop(lease)) }.or_else(
            || {
                raster.texture_factory()?.upload_rgba(
                    fallback.width,
                    fallback.height,
                    &fallback.pixels,
                )
            },
        )
    })
}
