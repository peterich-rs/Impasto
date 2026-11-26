//! Shared platform host. The platform thread posts commands. The render thread owns
//! the raster, the session, and the surface.

mod command;
mod diagnostics;
mod error;
mod exec;
mod gpu_view_gallery;
mod host;
mod images;
mod ime;
mod input;
mod log_sub;
mod plugins;
mod present;
mod ptr;
mod queue;
mod restoration;
mod rt;
mod scheduler;
mod semantics;
mod surface;
mod task_runner;
mod timing;
mod watchdog;
mod window_session;

use std::sync::Arc;

use engine::session::Metrics;

pub use command::{Command, FrameToken, SurfaceId, SurfaceTarget};
pub use diagnostics::telemetry::{ScheduleTiming, TelemetryProfile, mono_ns};
pub use diagnostics::{
    DiagnosticsHub, FrameSample, LogEntry, LogSource, NetEntry, NetKind, PhaseSample, TelemetryHub,
};
pub use engine::session::{FrameTime, Lifecycle};
pub use error::{BootError, SessionError};
pub use exec::{RtExec, settle_factory};
pub use host::{
    DropWait, Link, Poster, console_log, start, start_async, start_async_with_exit, start_on,
};
pub use images::{
    DecodeTarget, ImageDecoder, ImageError, ImageHandle, ImageReady, read_packaged_file,
};
pub use images::{ExternalFrame, ImportExternalFrame, pixel_count_ok, unpremultiply_rgba};
#[cfg(target_os = "android")]
pub use impasto::os::android::AndroidContext;
pub use impasto::{AppPaths, Register as PluginRegister};
pub use log_sub::install as install_log;
pub use log_sub::{log_filter_text, set_log_targets};
pub use plugins::{PluginServices, install_bundled_plugins, install_plugins};
pub use present::{AfterPresent, after_present};
pub use ptr::NativePtr;
pub use restoration::Restoration;
pub use semantics::{ProjectedNode, content_extent, project, scroll_step};
pub use surface::pixel_size;
pub use task_runner::{
    MainThreadPoster, ManualTaskRunner, PlatformTaskRunner, Task, TaskHandle, TaskQueueId,
    TaskRunner, TaskRunners, ThreadTaskRunner,
};
pub use timing::FrameTiming;
pub use tracing_subscriber::filter::{LevelFilter, Targets};
pub use window_session::WindowSession;

/// QuickJS console callback. Level 3 is an error.
pub type ConsoleLog = Box<dyn Fn(u8, &str)>;

/// Builds the session on the render thread. The closure must not capture the raster.
pub struct SessionOpen {
    pub shaper: Box<dyn engine::text::Shaper>,
    pub log: ConsoleLog,
}

pub type SessionFactory = Box<
    dyn FnOnce(
            SessionOpen,
            engine::session::ModuleConfig,
        ) -> Result<Box<dyn engine::session::Session>, SessionError>
        + Send,
>;

/// Opens one platform surface. `Send` so the shell can hand it to the render thread.
pub type SurfaceOpener = dyn FnMut(&mut raster::Rasterizer, SurfaceTarget) -> Result<Box<dyn Surface>, raster::Status>
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
    fn platform_views(&self, _surface: SurfaceId, _views: &[engine::tree::PlatformViewPlacement]) {}
    /// The current surface is ready to receive window-specific output.
    fn surface_attached(&self, _surface: SurfaceId) {}
    /// Apply one keyboard update. Called on the render thread.
    /// The implementation posts the work to the platform thread.
    fn apply_ime(&self, update: &engine::session::ImeUpdate);
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
    fn recover_device(
        &mut self,
        _engine: &mut raster::Rasterizer,
        _pixels: raster::PixelSize,
    ) -> Result<(), raster::Status> {
        Err(raster::Status::DeviceLost)
    }

    /// Acquire, draw, and present.
    ///
    /// `pixels` is the current drawable size in physical pixels.
    fn present(
        &mut self,
        raster: &mut raster::Rasterizer,
        pixels: raster::PixelSize,
    ) -> Result<Presented, raster::Status>;

    fn present_with_views(
        &mut self,
        raster: &mut raster::Rasterizer,
        pixels: raster::PixelSize,
        _views: &[engine::tree::PlatformViewPlacement],
    ) -> Result<Presented, raster::Status> {
        self.present(raster, pixels)
    }
}

/// Preferred display cadence for a compositor-owned frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FrameRateHint {
    /// Use the display's interactive rate.
    #[default]
    Display,
    /// Prefer 60 frames per second.
    Hz60,
    /// Prefer 30 frames per second.
    Hz30,
}

pub trait Vsync: Send + Sync {
    fn bind(&self, sink: Arc<dyn Fn(FrameToken, FrameTime) + Send + Sync>);
    fn request_frame(&self, token: FrameToken);
    fn request_frame_with_rate(&self, token: FrameToken, hint: FrameRateHint) {
        let _ = hint;
        self.request_frame(token);
    }
    fn cancel(&self);
}

/// Maps the UI and raster roles to physical queues.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThreadTopology {
    #[default]
    SharedRender,
    Split,
    PlatformUi,
}

impl ThreadTopology {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::SharedRender => "shared",
            Self::Split => "split",
            Self::PlatformUi => "platform-ui",
        }
    }

    pub fn configured() -> Result<Self, BootError> {
        match std::env::var("IMPASTO_THREAD_TOPOLOGY") {
            Ok(mode) => mode.parse(),
            Err(std::env::VarError::NotPresent) => Ok(Self::default()),
            Err(error) => Err(BootError::Platform(error.to_string())),
        }
    }
}

impl std::str::FromStr for ThreadTopology {
    type Err = BootError;

    fn from_str(mode: &str) -> Result<Self, Self::Err> {
        match mode {
            "shared" => Ok(Self::SharedRender),
            "split" => Ok(Self::Split),
            "platform-ui" => Ok(Self::PlatformUi),
            _ => Err(BootError::Platform(format!(
                "unknown thread topology: {mode}"
            ))),
        }
    }
}

pub struct Boot {
    pub font_family: String,
    pub launch: Launch,
    pub services: Box<dyn Services>,
    pub vsync: Arc<dyn Vsync>,
    pub decoder: Arc<dyn ImageDecoder>,
    pub modules: Vec<engine::session::Module>,
    pub plugin_services: PluginServices,
    pub platform_runner: Option<Arc<dyn TaskRunner>>,
    pub thread_topology: ThreadTopology,
    /// Written before `start` so `Linking.getInitialURL` is visible during eval.
    pub launch_url: engine::session::Shared<Option<String>>,
    /// Creates the graphics context on the render thread.
    pub open_raster:
        Box<dyn FnOnce(raster::FontLibrary) -> Result<raster::Rasterizer, raster::Status> + Send>,
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
    pub fn with_module(mut self, module: engine::session::Module) -> Self {
        self.modules.push(module);
        self
    }

    /// Installs a native producer when the owning Raster queue opens.
    pub fn with_gpu_view(
        mut self,
        id: raster::GpuViewId,
        producer: impl raster::GpuViewProducer,
    ) -> Self {
        let open = self.open_raster;
        self.open_raster = Box::new(move |fonts| {
            let mut raster = open(fonts)?;
            raster.register_gpu_view(id, producer).map_err(|error| {
                tracing::error!(target: "impasto::effect", %error, "GPU view registration failed");
                raster::Status::BadScene
            })?;
            Ok(raster)
        });
        self
    }

    /// Installs the bundled Gallery producer for the GPU lab.
    #[doc(hidden)]
    pub fn with_gallery_gpu_view(self, enabled: bool) -> Self {
        let requested = matches!(
            engine::session::shared_read(&self.launch_url).as_deref(),
            Some("impasto://gpu-lab" | "impasto://gpu-lab/invalid")
        );
        if (enabled || requested)
            && let Ok(id) = raster::GpuViewId::new("gallery-demo")
        {
            self.with_gpu_view(id, gpu_view_gallery::GalleryGpuView::default())
        } else {
            self
        }
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
