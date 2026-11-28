//! Developer diagnostics and telemetry ring buffers.

#[cfg(any(debug_assertions, feature = "devtools-server"))]
mod discovery;
mod hub;
mod inspect;
mod plugin_record;
#[cfg(any(debug_assertions, feature = "devtools-server"))]
pub(crate) mod remote;
#[cfg(any(debug_assertions, feature = "devtools-server"))]
mod remote_frame;
#[cfg(any(debug_assertions, feature = "devtools-server"))]
mod remote_stream;
#[cfg(any(debug_assertions, feature = "devtools-server"))]
pub(crate) mod remote_views;
pub(crate) mod snapshot;

pub(crate) use hub::CaptureFrame;
pub use hub::{DiagnosticsHub, FrameSample, LogEntry, LogSource, NetEntry, NetKind, PhaseSample};
pub use inspect::write_frame;

mod pftrace;
pub(crate) mod system_trace;
pub mod telemetry;
pub(crate) mod timeline;
mod trace_export;
pub use hub::TelemetryHub;
