//! Frame scheduling and the process-local clock shared by the trace producers.

use std::sync::OnceLock;
use std::time::Instant;

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

/// Source that scheduled the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FrameTrigger {
    #[default]
    Wake,
    Vsync,
    Compositor,
}

impl FrameTrigger {
    /// Name exported to diagnostic clients.
    pub fn name(self) -> &'static str {
        match self {
            Self::Wake => "wake",
            Self::Vsync => "vsync",
            Self::Compositor => "compositor",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScheduleTiming {
    pub trigger: FrameTrigger,
    pub vsync_ns: Option<u64>,
    /// Platform presentation target expressed in the process monotonic clock.
    pub target_present_ns: Option<u64>,
    pub ui_start_ns: u64,
    pub ui_done_ns: u64,
    pub raster_start_ns: u64,
    pub submit_done_ns: Option<u64>,
    pub present_start_ns: Option<u64>,
    pub present_done_ns: u64,
    pub budget_ns: u64,
    pub missed_vsync: bool,
    /// Raster submissions already pending when the UI queue admitted this frame.
    pub in_flight_depth: Option<u32>,
    /// Admission failure when this tick produced no surface submission.
    pub skip_reason: Option<&'static str>,
    pub repainted_view_count: Option<u32>,
    /// Raster received the display-link callback directly from the platform.
    pub compositor_only: bool,
    pub compositor_block: Option<engine::scene::CompositorBlockReason>,
    pub topology: Option<&'static str>,
}

impl ScheduleTiming {
    pub fn is_split_pipeline(self) -> bool {
        self.topology != Some("shared")
    }

    pub fn ui_ms(self) -> f64 {
        duration_ms(self.ui_start_ns, self.ui_done_ns)
    }

    pub fn raster_ms(self) -> f64 {
        duration_ms(self.raster_start_ns, self.present_done_ns)
    }

    pub fn queue_ms(self) -> f64 {
        duration_ms(self.ui_done_ns, self.raster_start_ns)
    }

    pub fn e2e_ns(self) -> u64 {
        self.present_done_ns.saturating_sub(self.start_ns())
    }

    pub fn e2e_ms(self) -> f64 {
        self.e2e_ns() as f64 / 1_000_000.0
    }

    pub fn target_lead_ms(self) -> Option<f64> {
        self.target_present_ns
            .map(|target| duration_ms(self.start_ns(), target))
    }

    pub fn deadline_overrun_ms(self) -> Option<f64> {
        self.target_present_ns
            .filter(|_| self.present_done_ns > 0)
            .map(|target| duration_ms(target, self.present_done_ns))
    }

    pub(crate) fn finish_present(&mut self, present_done_ns: u64) {
        self.present_done_ns = present_done_ns;
        let deadline_ns = self.target_present_ns.or_else(|| {
            (self.budget_ns > 0).then(|| self.start_ns().saturating_add(self.budget_ns))
        });
        self.missed_vsync = deadline_ns.is_some_and(|deadline| present_done_ns > deadline);
    }

    fn start_ns(self) -> u64 {
        self.vsync_ns.unwrap_or(self.ui_start_ns)
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
