//! Platform shell. The platform thread posts commands. The render thread owns
//! the engine, the session, and the surface.

mod command;
mod error;
mod input;
mod present;
mod ptr;
mod queue;
mod render;
mod shaper;

use std::sync::Arc;

use protocol::DrawList;
use session::Metrics;

pub use command::{Command, SurfaceTarget};
pub use error::{BootError, LogLevel, SessionError};
pub use present::{AfterPresent, after_present};
pub use ptr::NativePtr;
pub use render::{EngineSlot, Link, Poster, start};
pub use session::Lifecycle;
pub use shaper::ImpellerShaper;

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
    /// Show or hide the platform keyboard. Called on the render thread.
    /// The implementation posts the work to the platform thread.
    fn set_ime(&self, enabled: bool);
    /// Replace the platform IME buffer with `text` and put the caret at the end.
    /// Called on the render thread when a focused field's committed text changed
    /// outside an IME update. Not called for `Text` or `Composing` events.
    fn set_ime_text(&self, text: &str);
}

pub trait Surface {
    fn present(&mut self, engine: &mut engine::Engine) -> Result<Presented, engine::Status>;
}

pub trait Vsync: Send + Sync {
    fn bind(&self, post_frame: Arc<dyn Fn(f64) + Send + Sync>);
    fn request_frame(&self);
    fn cancel(&self);
}

pub struct Boot {
    pub font_family: String,
    pub launch: Launch,
    pub services: Box<dyn Services>,
    pub vsync: Arc<dyn Vsync>,
    pub open_surface: Box<
        dyn FnMut(&mut engine::Engine, SurfaceTarget) -> Result<Box<dyn Surface>, engine::Status>
            + Send,
    >,
}

pub fn metrics(width: f32, height: f32, device_pixel_ratio: f32) -> Metrics {
    Metrics {
        width,
        height,
        device_pixel_ratio,
        inset_left: 0.0,
        inset_top: 0.0,
        inset_right: 0.0,
        inset_bottom: 0.0,
    }
}
