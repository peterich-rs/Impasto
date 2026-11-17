use std::sync::atomic::{AtomicU64, Ordering};

use session::{FrameTime, InputEvent, Metrics, PlatformSettings};
use uicore::PointerEvent;

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
    pub pixels: engine::PixelSize,
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

pub enum Command {
    Pointer(PointerEvent),
    Input(InputEvent),
    /// The user hid the platform keyboard. The session blurs on the next frame.
    ImeDismissed,
    Metrics(Metrics),
    Settings(PlatformSettings),
    Lifecycle(Lifecycle),
    /// The window is visible again. The installed display list may be off the glass.
    Invalidate,
    /// The platform asked the process to drop caches. The next frame resubmits.
    MemoryWarning,
    WindowFocus(bool),
    SemanticsAction(uicore::SemanticsAction),
    /// A URL that arrived after the session exists. The render thread calls `Session::set_launch_url`.
    LaunchUrl(String),
    SurfaceReady(SurfaceTarget),
    SurfaceChanged {
        id: SurfaceId,
        metrics: Metrics,
        pixels: engine::PixelSize,
    },
    SurfaceDestroyed {
        id: SurfaceId,
    },
    ImageReady(ImageReady),
    /// Drop one cache key and ask the render thread for another frame.
    ReloadImage(String),
    /// A `fetch` finished on `net`.
    HttpDone {
        token: u64,
        result: Result<session::HttpResponse, session::HttpError>,
    },
    /// A streamed response produced its head on `net`.
    HttpHead {
        token: u64,
        status: u16,
        headers: Vec<(String, String)>,
    },
    /// A streamed response produced one body piece on `net`.
    HttpChunk {
        token: u64,
        bytes: Vec<u8>,
        done: bool,
        error: Option<session::HttpError>,
    },
    /// A framework module method finished on `net`.
    PluginDone {
        token: u64,
        result: session::PluginResult,
    },
    /// A socket the script opened produced one event.
    Socket {
        id: u64,
        event: String,
        text: bool,
        data: Vec<u8>,
    },
    /// A timer deadline arrived. The render thread runs one frame.
    Wake,
    Frame {
        token: FrameToken,
        time: FrameTime,
    },
    Shutdown,
}

/// Short name for the watchdog. Every variant is named so a new command fails to compile.
pub(crate) fn command_tag(command: &Command) -> &'static str {
    match command {
        Command::Pointer(_) => "pointer",
        Command::Input(_) => "input",
        Command::ImeDismissed => "ime",
        Command::Metrics(_) => "metrics",
        Command::Settings(_) => "settings",
        Command::Lifecycle(_) => "lifecycle",
        Command::Invalidate => "invalidate",
        Command::MemoryWarning => "memory",
        Command::WindowFocus(_) => "focus",
        Command::SemanticsAction(_) => "semantics",
        Command::LaunchUrl(_) => "launch",
        Command::SurfaceReady(_)
        | Command::SurfaceChanged { .. }
        | Command::SurfaceDestroyed { .. } => "surface",
        Command::ImageReady(_) => "image",
        Command::ReloadImage(_) => "reload",
        Command::HttpDone { .. } | Command::HttpHead { .. } | Command::HttpChunk { .. } => "http",
        Command::PluginDone { .. } => "plugin",
        Command::Socket { .. } => "socket",
        Command::Wake => "wake",
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
