//! Frame scheduling and the process-local clock shared by the trace producers.

use std::sync::OnceLock;
use std::time::Instant;

use crate::ThreadTopology;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum TelemetryProfile {
    #[default]
    Off,
    Counters,
    GpuAndTimeline,
    DeepInspect,
}

impl TelemetryProfile {
    pub(crate) fn configured(devtools: bool) -> Self {
        match std::env::var("IMPASTO_TELEMETRY").as_deref() {
            Ok("off") => Self::Off,
            Ok("counters") => Self::Counters,
            Ok("gpu") => Self::GpuAndTimeline,
            _ if devtools && cfg!(debug_assertions) => Self::Counters,
            _ => Self::Off,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScheduleTiming {
    pub vsync_ns: Option<u64>,
    pub ui_start_ns: u64,
    pub ui_done_ns: u64,
    pub raster_start_ns: u64,
    pub submit_done_ns: Option<u64>,
    pub present_start_ns: Option<u64>,
    pub present_done_ns: u64,
    pub budget_ns: u64,
    pub missed_vsync: bool,
    /// Raster received the display-link callback directly from the platform.
    pub compositor_only: bool,
    pub compositor_block: Option<engine::scene::CompositorBlockReason>,
    pub topology: ThreadTopology,
}

impl ScheduleTiming {
    pub fn ui_ms(self) -> f64 {
        duration_ms(self.ui_start_ns, self.ui_done_ns)
    }

    pub fn raster_ms(self) -> f64 {
        duration_ms(self.raster_start_ns, self.present_done_ns)
    }

    pub fn queue_ms(self) -> f64 {
        duration_ms(self.ui_done_ns, self.raster_start_ns)
    }

    pub fn e2e_ms(self) -> f64 {
        duration_ms(
            self.vsync_ns.unwrap_or(self.ui_start_ns),
            self.present_done_ns,
        )
    }
}

fn duration_ms(start: u64, end: u64) -> f64 {
    end.saturating_sub(start) as f64 / 1_000_000.0
}

pub fn mono_ns() -> u64 {
    mono_ns_at(Instant::now())
}

pub(crate) fn mono_ns_at(at: Instant) -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    u64::try_from(
        at.saturating_duration_since(*START.get_or_init(|| at))
            .as_nanos(),
    )
    .unwrap_or(u64::MAX)
}
