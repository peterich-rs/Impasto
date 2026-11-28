use std::collections::VecDeque;

use devtools_protocol::FrameRecord;
use serde::{Deserialize, Serialize};

use super::trim;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Sample {
    pub at_ms: f64,
    pub atlas_bytes: u64,
    pub atlas_entries: u32,
    pub atlas_hits: u32,
    pub atlas_misses: u32,
    pub atlas_evictions: u32,
    pub offscreen_peak_bytes: u64,
    pub image_resident_bytes: u64,
    pub textures: u32,
    pub layers: u32,
    pub pictures: u32,
    pub offscreen_passes: u32,
    pub direct_present: bool,
}

pub(crate) fn note(samples: &mut VecDeque<Sample>, frames: &[FrameRecord]) {
    for frame in frames {
        if !frame.presented
            || samples
                .back()
                .is_some_and(|last| last.at_ms + 250. > frame.at_ms)
        {
            continue;
        }
        samples.push_back(Sample {
            at_ms: frame.at_ms,
            atlas_bytes: frame.atlas_bytes,
            atlas_entries: frame.atlas_entries,
            atlas_hits: frame.atlas_hits,
            atlas_misses: frame.atlas_misses,
            atlas_evictions: frame.atlas_evictions,
            offscreen_peak_bytes: frame.offscreen_peak_bytes,
            image_resident_bytes: frame.image_resident_bytes,
            textures: frame.textures,
            layers: frame.layers,
            pictures: frame.pictures,
            offscreen_passes: frame.offscreen_passes,
            direct_present: frame.direct_present,
        });
    }
    trim(samples, 300);
}
