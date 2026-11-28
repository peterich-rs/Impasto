use std::sync::atomic::{AtomicU64, Ordering};

use engine::input::PointerEvent;
use engine::session::{FrameTime, InputEvent, IoEvent, Metrics, PlatformSettings};

use crate::Lifecycle;
use crate::images::ImageReady;
use crate::ptr::NativePtr;

/// Identifies one native window for the lifetime of its `Surface`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceId(pub u64);

impl SurfaceId {
    pub fn fresh() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// One accepted vsync. A callback whose token is no longer armed is discarded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameToken(pub u64);

pub struct SurfaceTarget {
    pub id: SurfaceId,
    pub layer: NativePtr,
    pub metrics: Metrics,
    /// Drawable size in physical pixels. Vulkan stores this on the swapchain.
    pub pixels: raster::PixelSize,
}

impl SurfaceTarget {
    /// `pixels` is derived from `metrics`. Hosts that know the window buffer
    /// size should overwrite [`Self::pixels`] with that value.
    pub fn new(id: SurfaceId, layer: NativePtr, metrics: Metrics) -> Self {
        Self {
            id,
            layer,
            metrics,
            pixels: crate::pixel_size(metrics),
        }
    }
}

/// Diagnostics queries that must run on the owning UI queue.
#[cfg(any(debug_assertions, feature = "devtools-server"))]
#[derive(Debug)]
pub enum DevtoolsQuery {
    ViewSnapshot { max_nodes: usize },
    ViewProbe { x: f32, y: f32 },
    SetLayoutScan(bool),
}

pub enum Command {
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    Devtools {
        query: DevtoolsQuery,
        reply: tokio::sync::oneshot::Sender<Result<Vec<u8>, String>>,
    },
    Pointer(PointerEvent),
    #[doc(hidden)]
    PointerMoves {
        event: PointerEvent,
        history: Box<engine::input::PointerHistory>,
    },
    Input(InputEvent),
    BackGesture(engine::session::BackGesture),
    /// The user hid the platform keyboard. The session blurs on the next frame.
    ImeDismissed,
    Metrics(Metrics),
    WindowDisplay(engine::session::WindowDisplay),
    WindowStateChanged(String),
    Settings(PlatformSettings),
    Lifecycle(Lifecycle),
    /// The window is visible again. The installed display list may be off the glass.
    Invalidate,
    /// The platform asked the process to drop caches. The next frame resubmits.
    MemoryWarning,
    WindowFocus(bool),
    SemanticsEnabled(bool),
    AutofillEnabled(bool),
    SemanticsAction(engine::semantics::SemanticsAction),
    /// A URL that arrived after the session exists. The render thread calls `Session::set_launch_url`.
    LaunchUrl(String),
    /// Staged UTF-8 debug update sent by the driver.
    UpdateSource {
        revision: String,
        source: String,
    },
    SurfaceReady(SurfaceTarget),
    SurfaceChanged {
        id: SurfaceId,
        metrics: Metrics,
        pixels: raster::PixelSize,
    },
    SurfaceDestroyed {
        id: SurfaceId,
    },
    ImageReady(ImageReady),
    ExternalTexture {
        source: String,
        frame: Option<crate::images::ExternalFrame>,
    },
    ExternalGpuTexture {
        source: String,
        frame: Option<crate::images::ImportExternalFrame>,
    },
    /// Drop one cache key and ask the render thread for another frame.
    ReloadImage(String),
    JsIo(engine::session::Delivery),
    /// I/O from hosts that submit work without a runtime route.
    Io(IoEvent),
    /// A timer deadline arrived. The render thread runs one frame.
    Wake,
    /// Delayed UI wake stamped with its scheduling ticket.
    Deadline {
        ticket: u64,
    },
    /// Raster is approaching a list window edge and still owns vsync.
    PrefetchCompositorWindow {
        generation: u64,
        time: FrameTime,
    },
    /// Raster reached a motion or scheduling boundary and returned frame ownership to UI.
    CommitCompositorState {
        generation: u64,
        snapshot: Option<std::sync::Arc<engine::scene::CompositorSnapshot>>,
        time: Option<FrameTime>,
    },
    Frame {
        token: FrameToken,
        time: FrameTime,
    },
    Shutdown,
}

/// Short name for the watchdog. Every variant is named so a new command fails to compile.
pub(crate) fn command_tag(command: &Command) -> &'static str {
    match command {
        Command::Pointer(_) | Command::PointerMoves { .. } => "pointer",
        Command::Input(_) => "input",
        Command::BackGesture(_) => "back",
        Command::ImeDismissed => "ime",
        Command::Metrics(_) | Command::WindowDisplay(_) => "metrics",
        Command::Settings(_) => "settings",
        Command::Lifecycle(_) => "lifecycle",
        Command::Invalidate => "invalidate",
        Command::MemoryWarning => "memory",
        Command::WindowFocus(_) => "focus",
        Command::WindowStateChanged(_) => "window state",
        Command::SemanticsEnabled(_)
        | Command::AutofillEnabled(_)
        | Command::SemanticsAction(_) => "semantics",
        Command::LaunchUrl(_) => "launch",
        Command::UpdateSource { .. } => "source update",
        Command::SurfaceReady(_)
        | Command::SurfaceChanged { .. }
        | Command::SurfaceDestroyed { .. } => "surface",
        Command::ImageReady(_)
        | Command::ExternalTexture { .. }
        | Command::ExternalGpuTexture { .. } => "image",
        Command::ReloadImage(_) => "reload",
        Command::JsIo(_) => "js io",
        Command::Io(IoEvent::Plugin { .. }) => "plugin",
        Command::Io(IoEvent::NativeStream { .. }) => "native stream",
        Command::Io(IoEvent::Host(_)) => "host event",
        #[cfg(any(debug_assertions, feature = "devtools-server"))]
        Command::Devtools { .. } => "devtools",
        Command::Wake | Command::Deadline { .. } => "wake",
        Command::CommitCompositorState { .. } => "animation handoff",
        Command::PrefetchCompositorWindow { .. } => "list prefetch",
        Command::Frame { .. } => "frame",
        Command::Shutdown => "shutdown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_and_frame_have_stable_tags() {
        assert_eq!(command_tag(&Command::Wake), "wake");
        assert_eq!(
            command_tag(&Command::Frame {
                token: FrameToken(1),
                time: FrameTime::at(0.0),
            }),
            "frame"
        );
        assert_eq!(command_tag(&Command::Shutdown), "shutdown");
    }
}
