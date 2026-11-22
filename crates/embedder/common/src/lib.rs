//! Shared platform host. The platform thread posts commands. The render thread owns
//! the engine, the session, and the surface.

mod command;
mod diagnostics;
mod error;
mod exec;
mod files;
mod host;
mod http;
mod images;
mod ime;
mod input;
mod log_sub;
mod module_stubs;
mod modules;
mod net;
mod present;
mod ptr;
mod queue;
mod restoration;
mod rt;
mod scheduler;
mod semantics;
mod shaper;
mod surface;
mod timing;
mod watchdog;
mod window_session;

use std::sync::Arc;

use session::Metrics;

pub use command::{Command, FrameToken, SurfaceId, SurfaceTarget};
pub use diagnostics::DiagnosticsHub;
pub use error::{BootError, SessionError};
pub use exec::{MainThreadPoster, RtExec, settle_factory};
pub use host::{DropWait, Link, Poster, console_log, start, start_async, start_async_with_exit};
pub use http::{OpenWireClient, http_client};
pub use images::{ExternalFrame, ImportExternalFrame, pixel_count_ok};
pub use images::{ImageDecoder, ImageError, ImageHandle, ImageReady};
pub use log_sub::install as install_log;
pub use log_sub::set_log_targets;
pub use module_stubs::{
    check_module_stubs, first_party_modules, module_stub_path, module_stub_source, package_folder,
    write_module_stubs,
};
pub use modules::{
    ClipboardHost, LinkingHost, PermissionsHost, StorageHost, app_state, async_storage, clipboard,
    file_system, linking, memory, native_plugins, permission_results, permissions, picked_document,
};
pub use net::{WebSocketCommand, WebSocketEvent, WebSocketTransport};
pub use present::{AfterPresent, after_present};
pub use ptr::NativePtr;
pub use restoration::Restoration;
pub use semantics::{ProjectedNode, content_extent, project, scroll_step};
pub use session::{FrameTime, Lifecycle};
pub use shaper::ImpellerShaper;
pub use surface::pixel_size;
pub use timing::FrameTiming;
pub use tracing_subscriber::filter::{LevelFilter, Targets};
pub use window_session::WindowSession;

/// QuickJS console callback. Level 3 is an error.
pub type ConsoleLog = Box<dyn Fn(u8, &str)>;

/// Host sink for one [`session::IoJob`].
pub type IoSink = Box<dyn Fn(session::IoJob) -> Result<(), Box<session::IoJob>> + Send>;

/// Builds the session on the render thread. The closure must not capture the engine.
pub struct SessionOpen {
    pub shaper: Box<dyn uicore::Shaper>,
    pub log: ConsoleLog,
}

pub type SessionFactory = Box<
    dyn FnOnce(
            SessionOpen,
            IoSink,
            session::ModuleConfig,
        ) -> Result<Box<dyn session::Session>, SessionError>
        + Send,
>;

/// Opens one platform surface. `Send` so the shell can hand it to the render thread.
pub type SurfaceOpener = dyn FnMut(&mut engine::Engine, SurfaceTarget) -> Result<Box<dyn Surface>, engine::Status>
    + Send;

pub type OpenSurface = Box<SurfaceOpener>;

pub enum Launch {
    /// Invoked on the ui thread. The shaper is created on that thread.
    Session(SessionFactory),
}

pub enum Presented {
    Continue,
    /// The surface finished a dump or asked the process to leave the frame loop.
    Stop,
}

pub trait Services: Send {
    fn font(&self, family: &str) -> Option<&[u8]>;
    /// The current task's small saved-state snapshot, when the platform provides one.
    fn restoration(&self) -> Option<Restoration> {
        None
    }
    fn back_enabled(&self, _enabled: bool) {}
    fn window_session(&self) -> WindowSession {
        WindowSession::default()
    }
    /// Platform WebSockets, if the host uses a different network stack.
    fn websocket_transport(&self) -> Option<Arc<dyn WebSocketTransport>> {
        None
    }
    fn source_updated(&self, _revision: &str, _result: Result<(), &str>) {}
    fn unhandled_back(&self) {
        self.request_exit();
    }
    /// Projected semantics nodes. The default ignores them. Called on the render thread.
    fn semantics(&self, _nodes: &[ProjectedNode]) {}
    /// Includes native view slots in the framework's accessibility traversal order.
    fn semantics_include_platform_views(&self) -> bool {
        false
    }
    /// Placement from the most recently submitted frame for a specific surface.
    fn platform_views(&self, _surface: SurfaceId, _views: &[uicore::PlatformViewPlacement]) {}
    /// The current surface is ready to receive window-specific output.
    fn surface_attached(&self, _surface: SurfaceId) {}
    /// Apply one keyboard update. Called on the render thread.
    /// The implementation posts the work to the platform thread.
    fn apply_ime(&self, update: &session::ImeUpdate);
    /// The session wants the process to leave. The default does nothing.
    fn request_exit(&self) {}
    /// `IMPASTO_INSPECT` wrote `frame.json`. The host should leave, like a dump.
    fn inspect_finished(&self) {}
    /// Extra font files registered after the primary family. The default is empty.
    fn extra_fonts(&self) -> Vec<(String, Vec<u8>)> {
        Vec::new()
    }
}

pub trait Surface {
    /// Retires a lost device and its surface. The next present can open the
    /// window again once the host resubmits its current layer tree.
    fn recover_device(&mut self, _engine: &mut engine::Engine) -> Result<(), engine::Status> {
        Err(engine::Status::DeviceLost)
    }

    /// Acquire, draw, and present.
    ///
    /// `pixels` is the drawable size the host posted with the surface. Vulkan
    /// copies it onto the swapchain before acquire. Metal reads the size from
    /// the layer.
    fn present(
        &mut self,
        engine: &mut engine::Engine,
        pixels: engine::PixelSize,
    ) -> Result<Presented, engine::Status>;

    fn present_with_views(
        &mut self,
        engine: &mut engine::Engine,
        pixels: engine::PixelSize,
        _views: &[uicore::PlatformViewPlacement],
    ) -> Result<Presented, engine::Status> {
        self.present(engine, pixels)
    }
}

pub trait Vsync: Send + Sync {
    fn bind(&self, sink: Arc<dyn Fn(FrameToken, FrameTime) + Send + Sync>);
    fn request_frame(&self, token: FrameToken);
    fn cancel(&self);
}

pub struct Boot {
    pub font_family: String,
    pub launch: Launch,
    pub services: Box<dyn Services>,
    pub vsync: Arc<dyn Vsync>,
    pub decoder: Arc<dyn ImageDecoder>,
    pub modules: Vec<session::Module>,
    pub main_thread: Option<MainThreadPoster>,
    /// Written before `start` so `Linking.getInitialURL` is visible during eval.
    pub launch_url: session::Shared<Option<String>>,
    /// Absent HTTP fails with `network is not linked`. Hosts inject [`http_client`].
    pub http: Option<Arc<dyn session::HttpClient>>,
    /// Creates the graphics context on the render thread.
    pub open_engine: Box<dyn FnOnce() -> Result<engine::Engine, engine::Status> + Send>,
    pub open_surface: OpenSurface,
    /// Log one line of layer-tree shape after a frame records new layers.
    pub dump_list: bool,
    /// `IMPASTO_INSPECT` directory. The render thread writes timing `frame.json` there.
    pub inspect: Option<std::path::PathBuf>,
    /// Directory for the xlog file sink. `None` keeps console and devtools only.
    pub log_dir: Option<std::path::PathBuf>,
    /// Enable the in-app devtools: hub, network recording, and the panic
    /// hook. `None` leaves the process with zero diagnostics overhead.
    pub devtools: Option<Arc<DiagnosticsHub>>,
    /// Bundle source map text for the devtools error page.
    pub devtools_source_map: Option<Arc<String>>,
    /// Directory for one richer `frame.json` after the first successful present.
    /// `IMPASTO_INSPECT`. `None` writes nothing.
    pub inspect_dir: Option<std::path::PathBuf>,
}

impl Boot {
    pub fn with_module(mut self, module: session::Module) -> Self {
        self.modules.push(module);
        self
    }

    /// Turns devtools on. The caller keeps the hub to register the module
    /// on its own; `start` wires recording and the panic hook.
    pub fn with_devtools(mut self) -> Arc<DiagnosticsHub> {
        let hub = Arc::new(DiagnosticsHub::new());
        self.devtools = Some(Arc::clone(&hub));
        hub
    }
}

pub fn metrics(width: f32, height: f32, device_pixel_ratio: f32) -> Metrics {
    Metrics::new(width, height, device_pixel_ratio)
}
