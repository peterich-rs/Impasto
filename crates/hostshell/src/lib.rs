//! Platform shell. The platform thread posts commands. The render thread owns
//! the engine, the session, and the surface.

mod command;
mod diagnostics;
mod error;
mod exec;
mod files;
mod host_stubs;
mod http;
mod images;
mod ime;
mod input;
mod log_sub;
mod modules;
mod net;
mod present;
mod ptr;
mod queue;
mod rt;
mod scheduler;
mod semantics;
mod shaper;
mod shell;
mod surface;
mod timing;
mod watchdog;

use std::sync::Arc;

use session::Metrics;

pub use command::{Command, FrameToken, SurfaceId, SurfaceTarget};
pub use diagnostics::DiagnosticsHub;
pub use error::{BootError, SessionError};
pub use exec::{MainThreadPoster, RtExec, settle_factory};
pub use host_stubs::{
    first_party_modules, host_stub_path, host_stub_source, package_folder, write_host_stubs,
};
pub use http::{OpenWireClient, http_client};
pub use images::{ImageDecoder, ImageError, ImageHandle, ImageReady};
pub use log_sub::install as install_log;
pub use log_sub::set_log_targets;
pub use modules::{
    ClipboardHost, LinkingHost, PermissionsHost, StorageHost, app_state, async_storage, clipboard,
    file_system, linking, memory, permissions,
};
pub use present::{AfterPresent, after_present};
pub use ptr::NativePtr;
pub use semantics::{ProjectedNode, content_extent, project, scroll_step};
pub use session::{FrameTime, Lifecycle};
pub use shaper::ImpellerShaper;
pub use shell::{DropWait, Link, Poster, console_log, start};
pub use surface::pixel_size;
pub use timing::FrameTiming;
pub use tracing_subscriber::filter::{LevelFilter, Targets};

/// QuickJS console callback. Level 3 is an error.
pub type ConsoleLog = Box<dyn Fn(u8, &str)>;

/// Host sink for one [`session::IoJob`].
pub type IoSink = Box<dyn Fn(session::IoJob) + Send>;

/// Builds the session on the render thread. The closure must not capture the engine.
pub struct SessionOpen {
    pub shaper: Box<dyn uicore::Shaper>,
    pub log: ConsoleLog,
}

pub type SessionFactory = Box<
    dyn FnOnce(
            SessionOpen,
            IoSink,
            session::HostConfig,
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
    /// Projected semantics nodes. The default ignores them. Called on the render thread.
    fn semantics(&self, _nodes: &[ProjectedNode]) {}
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
    pub modules: Vec<session::HostModule>,
    pub main_thread: Option<MainThreadPoster>,
    /// Written before `start` so `Linking.getInitialURL` is visible during eval.
    pub launch_url: session::Shared<Option<String>>,
    /// Absent HTTP fails with `network is not linked`. Hosts inject [`http_client`].
    pub http: Option<Arc<dyn session::HttpClient>>,
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
    pub fn with_module(mut self, module: session::HostModule) -> Self {
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
