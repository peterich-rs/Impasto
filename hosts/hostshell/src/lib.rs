//! Platform shell. The platform thread posts commands. The render thread owns
//! the engine, the session, and the surface.

mod command;
mod error;
mod images;
mod ime;
mod input;
mod net;
mod platform;
mod present;
mod ptr;
mod queue;
mod scheduler;
mod shaper;
mod shell;
mod surface;
mod timing;

use std::sync::Arc;

use protocol::DrawList;
use session::Metrics;

pub use command::{Command, FrameToken, SurfaceId, SurfaceTarget};
pub use error::{BootError, LogLevel, SessionError};
pub use images::{ImageDecoder, ImageError, ImageHandle, ImageReady};
pub use platform::{EmptyBridge, PlatformBridge};
pub use present::{AfterPresent, after_present};
pub use ptr::NativePtr;
pub use session::{FrameTime, Lifecycle};
pub use shaper::ImpellerShaper;
pub use shell::{EngineSlot, Link, Poster, start};
pub use timing::FrameTiming;

pub enum Launch {
    /// Submitted once, then presented. The shell does not pump a session.
    Fixture(DrawList),
    /// Invoked on the render thread after the engine exists. The closure must
    /// not capture the engine; [`EngineSlot::shaper`] builds the shaper there.
    Session(Box<dyn FnOnce(&EngineSlot) -> Result<Box<dyn session::Session>, SessionError> + Send>),
}

pub enum Presented {
    Continue,
    /// The surface finished a dump or asked the process to leave the frame loop.
    Stop,
}

pub trait Services: Send {
    fn font(&self, family: &str) -> Option<&[u8]>;
    fn image<'a>(&'a self, source: &str) -> Option<engine::ImagePixels<'a>>;
    fn log(&self, level: LogLevel, message: &str);
    /// A new semantics tree. The default ignores it. Called on the render thread.
    fn semantics(&self, _update: &ui_core::SemanticsUpdate) {}
    /// Apply one keyboard update. Called on the render thread.
    /// The implementation posts the work to the platform thread.
    fn apply_ime(&self, update: &session::ImeUpdate);
    /// The session wants the process to leave. The default does nothing.
    fn request_exit(&self) {}
    /// Writes text to the platform clipboard. The default drops it.
    fn set_clipboard(&self, _text: &str) {}
    /// Extra font files registered after the primary family. The default is empty.
    fn extra_fonts(&self) -> Vec<(String, Vec<u8>)> {
        Vec::new()
    }
}

pub trait Surface {
    fn present(&mut self, engine: &mut engine::Engine) -> Result<Presented, engine::Status>;
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
    pub bridge: Arc<dyn PlatformBridge>,
    pub open_surface: Box<
        dyn FnMut(&mut engine::Engine, SurfaceTarget) -> Result<Box<dyn Surface>, engine::Status>
            + Send,
    >,
}

pub fn metrics(width: f32, height: f32, device_pixel_ratio: f32) -> Metrics {
    Metrics::new(width, height, device_pixel_ratio)
}
