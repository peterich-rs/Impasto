use std::collections::VecDeque;

use devtools_protocol::FrameRecord;
use serde::{Deserialize, Serialize};

use crate::types::FrameStats;

const CAPACITY: usize = 2400;
pub(crate) const LIVE_WINDOW_MS: f64 = 2_000.;
const IDLE_AFTER_MS: f64 = 500.;

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Frames(VecDeque<FrameRecord>);

impl Frames {
    pub(crate) fn update(&mut self, reset: bool, entries: Vec<FrameRecord>) {
        if reset {
            self.0.clear();
        }
        for frame in entries {
            match self.0.binary_search_by_key(&frame.id, |entry| entry.id) {
                Ok(index) if self.0[index].revision < frame.revision => self.0[index] = frame,
                Ok(_) => {}
                Err(0) if self.0.len() >= CAPACITY => {}
                Err(index) => {
                    self.0.insert(index, frame);
                    if self.0.len() > CAPACITY {
                        self.0.pop_front();
                    }
                }
            }
        }
    }

    pub(crate) fn get(&self, id: u64) -> Option<&FrameRecord> {
        self.0
            .binary_search_by_key(&id, |entry| entry.id)
            .ok()
            .and_then(|index| self.0.get(index))
    }

    pub(crate) fn entries(&self) -> &VecDeque<FrameRecord> {
        &self.0
    }

    pub(crate) fn restore(&mut self) {
        let entries = std::mem::take(&mut self.0).into();
        self.update(true, entries);
    }
}

pub(crate) fn refresh_budget(refresh_rate: Option<f32>) -> f64 {
    refresh_rate
        .filter(|hz| hz.is_finite() && *hz > 0.)
        .map_or(0., |hz| 1_000. / f64::from(hz))
}

pub(crate) fn is_split(frame: &FrameRecord) -> bool {
    frame.topology.as_deref() != Some("shared")
}

pub(crate) fn ui_duration(frame: &FrameRecord) -> f64 {
    if frame.compositor_only {
        0.
    } else {
        frame.ui_ms
    }
}

pub(crate) fn cpu_duration(frame: &FrameRecord) -> f64 {
    let ui = ui_duration(frame);
    if is_split(frame) {
        ui.max(frame.raster_ms)
    } else {
        (ui + frame.raster_ms).max(frame.work_ms)
    }
}

pub(crate) fn gpu_duration(frame: &FrameRecord) -> f64 {
    frame.gpu.frame_span_us.unwrap_or_default().max(0.) / 1_000.
}

pub(crate) fn stage_duration(frame: &FrameRecord) -> f64 {
    cpu_duration(frame).max(gpu_duration(frame))
}

pub(crate) fn effective_budget(frame: &FrameRecord, fallback_budget: f64) -> f64 {
    if frame.budget_ms.is_finite() && frame.budget_ms > 0. {
        frame.budget_ms
    } else if fallback_budget.is_finite() && fallback_budget > 0. {
        fallback_budget
    } else {
        0.
    }
}

#[derive(Clone, Copy)]
pub(crate) struct StageFlags {
    pub ui: bool,
    pub raster: bool,
    pub gpu: bool,
}

impl StageFlags {
    pub(crate) fn any(self) -> bool {
        self.ui || self.raster || self.gpu
    }
}

pub(crate) fn stage_flags(frame: &FrameRecord, fallback_budget: f64) -> StageFlags {
    let budget = effective_budget(frame, fallback_budget);
    let ui = ui_duration(frame);
    let raster = frame.raster_ms;
    let (ui_over, raster_over) = if is_split(frame) {
        (budget > 0. && ui > budget, budget > 0. && raster > budget)
    } else {
        let cpu_over = budget > 0. && cpu_duration(frame) > budget;
        let ui_dominant = !frame.compositor_only && ui >= raster;
        (cpu_over && ui_dominant, cpu_over && !ui_dominant)
    };
    StageFlags {
        ui: !frame.compositor_only && (frame.ui_over_budget || ui_over),
        raster: frame.raster_over_budget || raster_over,
        gpu: frame.gpu_over_budget || (budget > 0. && gpu_duration(frame) > budget),
    }
}

pub(crate) fn failed_or_skipped(frame: &FrameRecord) -> bool {
    !frame.presented || frame.skip_reason.is_some()
}

pub(crate) fn dropped_vsyncs(frame: &FrameRecord, fallback_budget: f64) -> u32 {
    let budget = effective_budget(frame, fallback_budget);
    let stage = stage_duration(frame);
    let stage_dropped = if budget > 0. && stage > budget {
        ((stage / budget).ceil() - 1.).max(1.) as u32
    } else {
        0
    };
    stage_dropped.max(u32::from(
        failed_or_skipped(frame) || stage_flags(frame, fallback_budget).any(),
    ))
}

pub(crate) fn frame_hitch_ms(frame: &FrameRecord, fallback_budget: f64) -> f64 {
    f64::from(dropped_vsyncs(frame, fallback_budget)) * effective_budget(frame, fallback_budget)
}

pub(crate) fn is_idle(now_ms: f64, latest_presented_ms: Option<f64>) -> bool {
    latest_presented_ms.is_none_or(|at_ms| now_ms - at_ms >= IDLE_AFTER_MS)
}

pub(crate) fn stats<'a>(
    frames: impl Iterator<Item = &'a FrameRecord>,
    fallback_budget: f64,
) -> FrameStats {
    summarize(frames, false, None, fallback_budget)
}

pub(crate) fn live_stats<'a>(
    frames: impl Iterator<Item = &'a FrameRecord>,
    idle: bool,
    window_ms: f64,
    fallback_budget: f64,
) -> FrameStats {
    summarize(frames, idle, Some(window_ms), fallback_budget)
}

fn summarize<'a>(
    frames: impl Iterator<Item = &'a FrameRecord>,
    idle: bool,
    window_ms: Option<f64>,
    fallback_budget: f64,
) -> FrameStats {
    let mut entries: Vec<_> = frames.collect();
    entries.sort_unstable_by(|a, b| a.at_ms.total_cmp(&b.at_ms).then(a.id.cmp(&b.id)));
    let mut stats = FrameStats {
        count: u32::try_from(entries.len()).unwrap_or(u32::MAX),
        idle,
        on_time_pct: 100.,
        budget_ms: entries.last().map_or(fallback_budget, |frame| {
            effective_budget(frame, fallback_budget)
        }),
        ..FrameStats::default()
    };
    let Some((first, last)) = entries.first().zip(entries.last()) else {
        return stats;
    };

    let mut work = Vec::with_capacity(entries.len());
    let mut latency = Vec::with_capacity(entries.len());
    let mut composited = 0;
    for frame in &entries {
        work.push(stage_duration(frame));
        latency.push(frame.e2e_ms);
        let flags = stage_flags(frame, fallback_budget);
        stats.ui_jank_count += u32::from(flags.ui);
        stats.raster_jank_count += u32::from(flags.raster);
        stats.gpu_jank_count += u32::from(flags.gpu);
        stats.jank_count += u32::from(flags.any() || failed_or_skipped(frame));
        stats.dropped_frames = stats
            .dropped_frames
            .saturating_add(dropped_vsyncs(frame, fallback_budget));
        stats.hitch_ms += frame_hitch_ms(frame, fallback_budget);
        stats.presented_count += u32::from(frame.presented);
        stats.failed_present_count += u32::from(!frame.presented && frame.present_error.is_some());
        stats.wake_count += u32::from(frame.trigger == "wake");
        composited += u32::from(frame.compositor_only);
    }
    work.sort_unstable_by(f64::total_cmp);
    latency.sort_unstable_by(f64::total_cmp);
    let count = f64::from(stats.count);
    let window_sec = window_ms
        .unwrap_or_else(|| (last.at_ms - first.at_ms).max(stats.budget_ms.max(1.)))
        / 1_000.;
    stats.mean_ms = work.iter().sum::<f64>() / count;
    stats.p50_ms = percentile(&work, 50);
    stats.p95_ms = percentile(&work, 95);
    stats.max_ms = percentile(&work, 100);
    stats.p95_e2e_ms = percentile(&latency, 95);
    stats.hitch_ms_per_sec = stats.hitch_ms / window_sec;
    stats.on_time_pct = f64::from(stats.count - stats.jank_count) / count * 100.;
    stats.compositor_pct = f64::from(composited) / count * 100.;
    if !idle {
        stats.present_rate_hz = f64::from(stats.presented_count) / window_sec;
        stats.burst_fps = detect_burst_fps(&entries, stats.budget_ms, fallback_budget);
    }
    stats
}

fn percentile(sorted: &[f64], percent: usize) -> f64 {
    sorted[(sorted.len() * percent).div_ceil(100) - 1]
}

// Include failed frames between successful presents in the selected burst.
fn detect_burst_fps(entries: &[&FrameRecord], budget: f64, fallback_budget: f64) -> Option<f64> {
    if budget <= 0. {
        return None;
    }
    let gap_ms = (2.5 * budget).max(50.);
    let mut start = 0;
    let mut previous = None;
    let mut presented = 0;
    let mut burst = None;
    for (index, frame) in entries
        .iter()
        .enumerate()
        .filter(|(_, frame)| frame.presented)
    {
        if previous.is_none_or(|at| frame.at_ms - at > gap_ms) {
            start = index;
            presented = 0;
        }
        previous = Some(frame.at_ms);
        presented += 1;
        if presented >= 3 {
            burst = Some((start, index, presented));
        }
    }
    let (start, end, presented) = burst?;
    let dropped: f64 = entries[start..=end]
        .iter()
        .map(|frame| f64::from(dropped_vsyncs(frame, fallback_budget)))
        .sum();
    Some(1_000. / budget * f64::from(presented) / (f64::from(presented) + dropped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_backfill_replaces_only_the_matching_newer_revision() {
        let mut frames = Frames::default();
        let frame = |id, revision| FrameRecord {
            id,
            revision,
            ..FrameRecord::default()
        };
        frames.update(
            false,
            vec![frame(1, 2), frame(2, 3), frame(1, 4), frame(1, 3)],
        );
        assert_eq!(frames.entries().len(), 2);
        assert_eq!(frames.get(1).map(|frame| frame.revision), Some(4));
        frames.update(true, vec![frame(3, 12)]);
        assert!(frames.get(1).is_none());
    }
    fn sample(id: u64, at_ms: f64) -> FrameRecord {
        FrameRecord {
            id,
            at_ms,
            presented: true,
            budget_ms: 1000. / 60.,
            ui_ms: 3.,
            raster_ms: 4.,
            e2e_ms: 7.,
            trigger: "wake".to_owned(),
            ..FrameRecord::default()
        }
    }

    #[test]
    fn no_frames_or_periodic_wakes_do_not_register_hitches() {
        let empty = live_stats(std::iter::empty(), true, 2_000., 1000. / 60.);
        assert!(empty.idle);
        assert_eq!(empty.count, 0);
        assert_eq!(empty.on_time_pct, 100.);
        assert_eq!(empty.present_rate_hz, 0.);
        assert_eq!(empty.burst_fps, None);
        let frames: Vec<_> = (0..10_u32)
            .map(|i| sample(u64::from(i), f64::from(i) * 200.))
            .collect();
        let live = live_stats(frames.iter(), false, 2_000., 0.);
        assert_eq!(live.hitch_ms_per_sec, 0.);
        assert_eq!(live.present_rate_hz, 5.);
        assert_eq!(live.burst_fps, None);
        assert_eq!(live.wake_count, 10);
    }

    #[test]
    fn burst_cadence_excludes_periodic_wakes_and_counts_failures_in_the_burst() {
        let mut frames: Vec<_> = (0..7_u32)
            .map(|i| sample(u64::from(i), f64::from(i) * 200.))
            .collect();
        for i in 0..30_u32 {
            frames.push(FrameRecord {
                trigger: "vsync".to_owned(),
                ..sample(u64::from(i + 7), 1500. + f64::from(i) * (1000. / 60.))
            });
        }
        let live = live_stats(frames.iter(), false, 2_000., 0.);
        assert_eq!(live.present_rate_hz, 18.5);
        assert!((live.burst_fps.unwrap() - 60.).abs() < 1e-9);
        frames[15].presented = false;
        frames[15].present_error = Some("acquire missed".to_owned());
        let live = live_stats(frames.iter(), false, 2_000., 0.);
        assert!((live.burst_fps.unwrap() - 58.).abs() < 1e-9);
        assert_eq!(live.failed_present_count, 1);
        assert_eq!(live.dropped_frames, 1);
        let now = frames.last().unwrap().at_ms + 510.;
        assert!(is_idle(now, frames.last().map(|frame| frame.at_ms)));
        let idle = live_stats(frames.iter(), true, 2_000., 0.);
        assert_eq!(idle.present_rate_hz, 0.);
        assert_eq!(idle.burst_fps, None);
        assert_eq!(idle.hitch_ms_per_sec, live.hitch_ms_per_sec);
        assert_eq!(idle.p95_ms, live.p95_ms);
    }

    #[test]
    fn overlapping_split_stages_report_throughput_and_e2e_separately() {
        let frames: Vec<_> = (0..60_u32)
            .map(|i| FrameRecord {
                ui_ms: 10.,
                raster_ms: 10.,
                e2e_ms: 20.,
                work_ms: 20.,
                missed_vsync: true,
                gpu: devtools_protocol::GpuTiming {
                    frame_span_us: Some(8000.),
                    ..Default::default()
                },
                ..sample(u64::from(i), f64::from(i) * (1000. / 60.))
            })
            .collect();
        let live = live_stats(frames.iter(), false, 1_000., 0.);
        assert_eq!(live.p95_ms, 10.);
        assert_eq!(live.p95_e2e_ms, 20.);
        assert_eq!(live.on_time_pct, 100.);
        assert_eq!(live.present_rate_hz, 60.);
        assert!((live.burst_fps.unwrap() - 60.).abs() < 1e-9);
        assert_eq!(live.dropped_frames, 0);
        assert_eq!(live.hitch_ms_per_sec, 0.);
    }

    #[test]
    fn stage_attribution_and_full_refresh_intervals_use_the_effective_budget() {
        let mut entries: Vec<_> = (0..59_u32)
            .map(|i| sample(u64::from(i), f64::from(i) * (1000. / 60.)))
            .collect();
        entries[1].raster_ms = 25.;
        let live = live_stats(entries.iter(), false, 1_000., 0.);
        assert_eq!(live.dropped_frames, 1);
        assert_eq!(live.hitch_ms_per_sec, 1000. / 60.);
        assert!((live.on_time_pct - 98.3).abs() < 0.01);
        assert_eq!(live.present_rate_hz, 59.);
        assert!((live.burst_fps.unwrap() - 59.).abs() < 1e-9);
        let shared = FrameRecord {
            topology: Some("shared".to_owned()),
            ui_ms: 10.,
            raster_ms: 8.,
            budget_ms: 0.,
            ..sample(1, 0.)
        };
        let gpu = FrameRecord {
            gpu: devtools_protocol::GpuTiming {
                frame_span_us: Some(49_000.),
                ..Default::default()
            },
            ..sample(2, 10.)
        };
        let raster = FrameRecord {
            raster_ms: 50.,
            ..sample(3, 20.)
        };
        let fallback = refresh_budget(Some(60.));
        let stats = live_stats(
            [&shared, &gpu, &raster].into_iter(),
            false,
            1_000.,
            fallback,
        );
        assert_eq!(
            (
                stats.ui_jank_count,
                stats.raster_jank_count,
                stats.gpu_jank_count
            ),
            (1, 1, 1)
        );
        assert_eq!(stats.dropped_frames, 5);
        assert!((stats.hitch_ms_per_sec - 83.3333).abs() < 0.001);
    }
}
