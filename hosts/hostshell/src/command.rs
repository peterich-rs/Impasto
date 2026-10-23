use session::{InputEvent, Metrics};
use ui_core::PointerEvent;

use crate::Lifecycle;
use crate::ptr::NativePtr;

pub struct SurfaceTarget {
    pub layer: NativePtr,
    pub metrics: Metrics,
}

pub enum Command {
    Pointer(PointerEvent),
    Input(InputEvent),
    /// The user hid the platform keyboard. The session blurs on the next frame.
    ImeDismissed,
    Metrics(Metrics),
    Lifecycle(Lifecycle),
    SurfaceReady(SurfaceTarget),
    SurfaceChanged(SurfaceTarget),
    SurfaceDestroyed,
    Frame {
        time_ms: f64,
    },
    Shutdown,
}
