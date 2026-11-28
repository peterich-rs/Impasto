use std::collections::VecDeque;

use devtools_protocol::FrameRecord;
use serde::{Deserialize, Serialize};

use crate::types::FrameStats;

const CAPACITY: usize = 2400;

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

pub(crate) fn stats<'a>(frames: impl Iterator<Item = &'a FrameRecord>) -> FrameStats {
    let mut work = Vec::new();
    let mut jank_count = 0;
    let mut composited = 0;
    let mut presented = 0_u32;
    let mut start = f64::INFINITY;
    let mut end: f64 = 0.;
    let mut sum = 0.;
    for frame in frames {
        let ms = duration(frame);
        work.push(ms);
        sum += ms;
        jank_count +=
            u32::from(frame.over_budget || frame.missed_vsync || frame.skip_reason.is_some());
        composited += u32::from(frame.compositor_only);
        if frame.present_ms > 0. && frame.skip_reason.is_none() {
            start = start.min(frame.at_ms);
            end = end.max(frame.at_ms);
            presented += 1;
        }
    }
    if work.is_empty() {
        return FrameStats::default();
    }
    work.sort_by(f64::total_cmp);
    let count = u32::try_from(work.len()).unwrap_or(u32::MAX);
    let percentile = |percent: usize| work[(work.len() * percent).div_ceil(100) - 1];
    FrameStats {
        count,
        mean_ms: sum / f64::from(count),
        p50_ms: percentile(50),
        p95_ms: percentile(95),
        max_ms: percentile(100),
        jank_count,
        compositor_pct: f64::from(composited) / f64::from(count) * 100.,
        fps: if end > start {
            f64::from(presented.saturating_sub(1)) * 1_000. / (end - start)
        } else {
            0.
        },
    }
}

pub(crate) fn duration(frame: &FrameRecord) -> f64 {
    frame.e2e_ms.max(frame.work_ms)
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
}
