//! Attaches the native plugin registry to a window session.

use std::sync::{Arc, OnceLock};

use impasto::{
    AppPaths, BlockingTask, FutureTask, PluginContext, PluginRegistrar, Register, Runtime,
    RuntimeBackend,
};
use serde_json::{Value, json};

use crate::{BootError, Command, TaskRunner, queue::Inbox};

struct Installed {
    register: Register,
    expected: Option<&'static str>,
}

static PLUGINS: OnceLock<Installed> = OnceLock::new();

/// Process entry points install their linked registrar before creating windows.
pub fn install_plugins(register: Register, expected: &'static str) -> Result<(), BootError> {
    PLUGINS
        .set(Installed {
            register,
            expected: Some(expected),
        })
        .map_err(|_| BootError::Platform("native plugins are already installed".into()))
}

/// Installs plugins for a framework-owned development host that loads source directly.
pub fn install_bundled_plugins(register: Register) -> Result<(), BootError> {
    PLUGINS
        .set(Installed {
            register,
            expected: None,
        })
        .map_err(|_| BootError::Platform("native plugins are already installed".into()))
}

#[derive(Default)]
pub struct PluginServices {
    #[cfg(target_os = "android")]
    pub android: Option<impasto::os::android::AndroidContext>,
    pub paths: AppPaths,
    pub initial_lifecycle: impasto::Lifecycle,
    pub native_channel: Option<impasto::os::NativeChannel>,
    pub native_events: Option<impasto::os::NativeEventChannel>,
}

pub(crate) struct WindowBindings {
    pub(crate) initial_url: Option<String>,
    pub(crate) session: crate::WindowSession,
    pub(crate) restoration: Option<crate::Restoration>,
    pub(crate) diagnostics: Option<Arc<crate::DiagnosticsHub>>,
    pub(crate) source_map: Option<Arc<String>>,
    pub(crate) os_version: String,
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
    native_events: Option<impasto::os::NativeEventChannel>,
}

impl PluginSession {
    pub(crate) fn lifecycle(&self, state: engine::session::Lifecycle) {
        use engine::session::Lifecycle as Host;
        use impasto::Lifecycle;
        if state == Host::Detached
            && let Some(events) = &self.native_events
        {
            events.close();
        }
        self.context.dispatch_lifecycle(match state {
            Host::Resumed => Lifecycle::Resumed,
            Host::Inactive => Lifecycle::Inactive,
            Host::Hidden => Lifecycle::Hidden,
            Host::Paused => Lifecycle::Paused,
            Host::Detached => Lifecycle::Detached,
        });
    }

    pub(crate) fn window_focus(&self, focused: bool) {
        self.context.dispatch_window_focus(focused);
    }

    pub(crate) fn open_url(&self, url: &str) {
        self.context.dispatch_open_url(url);
    }

    pub(crate) fn window_state_changed(&self, key: &str) {
        self.context.dispatch_window_state(key);
    }
}

impl Drop for PluginSession {
    fn drop(&mut self) {
        self.lifecycle(engine::session::Lifecycle::Detached);
    }
}

pub(crate) fn attach(
    modules: Vec<impasto::abi::Module>,
    services: PluginServices,
    bindings: WindowBindings,
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
    if let Some(expected) = plugins.expected {
        let expected: Vec<Value> = serde_json::from_str(expected).map_err(plugin_error)?;
        if descriptions.len() != expected.len()
            || descriptions.iter().zip(expected).any(|(actual, expected)| {
                expected["name"] != actual.name
                    || expected["specifier"] != actual.specifier
                    || expected["exports"] != json!(actual.exports)
                    || expected["typescript"]
                        .as_str()
                        .is_some_and(|typescript| typescript != actual.typescript)
            })
        {
            return Err(BootError::Platform(
                "linked plugin descriptors differ from the application bundle".into(),
            ));
        }
    }
    for entry in descriptions {
        if matches!(
            entry.specifier,
            "scene" | "reactive" | "#reactive" | "settings"
        ) {
            return Err(BootError::Platform(format!(
                "native module {} uses a reserved name",
                entry.specifier
            )));
        }
    }
    let runtime = Runtime::new(Backend { io, main });
    let queue = Arc::downgrade(inbox);
    let mut context = PluginContext::new(runtime, services.paths, move |event| {
        let Some(queue) = queue.upgrade() else {
            return false;
        };
        queue.post_plugin_event(event)
    });
    context = context
        .with_initial_url(bindings.initial_url)
        .with_window_state(bindings.session, bindings.restoration);
    if let Some(hub) = bindings.diagnostics {
        context = context.with_diagnostics(Arc::clone(&hub) as _);
        context.insert_extension(crate::diagnostics::snapshot::source(
            hub,
            bindings.source_map,
            context.clone(),
            bindings.os_version,
            std::env::var("IMPASTO_DUMP").ok(),
        ));
    }
    if let Some(channel) = services.native_channel {
        context.insert_extension(channel);
    }
    if let Some(events) = &services.native_events {
        context.insert_extension(events.clone());
    }
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
    context.dispatch_lifecycle(services.initial_lifecycle);
    let session = PluginSession {
        context,
        native_events: services.native_events,
    };
    let extensions = registrar.build(&session.context).map_err(plugin_error)?;
    let registry = if plugins.expected.is_some() {
        impasto::abi::ModuleRegistry::with_extensions(modules, extensions).map_err(plugin_error)?
    } else {
        impasto::abi::ModuleRegistry::try_new(modules.into_iter().chain(extensions).collect())
            .map_err(plugin_error)?
    };
    Ok((registry, Some(session)))
}

fn plugin_error(error: impl std::fmt::Display) -> BootError {
    BootError::Platform(error.to_string())
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
