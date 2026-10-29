use std::sync::atomic::{AtomicU64, Ordering};

use session::{FrameTime, InputEvent, Metrics, PlatformSettings};
use ui_core::PointerEvent;

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
    SemanticsAction(ui_core::SemanticsAction),
    /// The URL that opened this process. The render thread calls `Session::set_launch_url`.
    LaunchUrl(String),
    SurfaceReady(SurfaceTarget),
    SurfaceChanged {
        id: SurfaceId,
        metrics: Metrics,
    },
    SurfaceDestroyed {
        id: SurfaceId,
    },
    ImageReady(ImageReady),
    /// Drop one cache key and ask the render thread for another frame.
    ReloadImage(String),
    /// A `fetch` finished on `uifw-net`.
    HttpDone {
        token: u64,
        result: Result<session::HttpResponse, session::HttpError>,
    },
    /// A streamed response produced its head on `uifw-net`.
    HttpHead {
        token: u64,
        status: u16,
        headers: Vec<(String, String)>,
    },
    /// A streamed response produced one body piece on `uifw-net`.
    HttpChunk {
        token: u64,
        bytes: Vec<u8>,
        done: bool,
        error: Option<session::HttpError>,
    },
    /// A framework module method finished on `uifw-net`.
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
