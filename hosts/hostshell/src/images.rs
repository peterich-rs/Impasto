//! Image bytes decoded on `uifw-io` and installed on the render thread.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use engine::ImagePixels;
use protocol::DrawList;

/// Owned RGBA8888. The render thread turns this into [`ImagePixels`] for `submit`.
#[derive(Clone, Debug)]
pub struct ImageHandle {
    pub source: Arc<str>,
    pub generation: u64,
    pub size: [u32; 2],
    pub pixels: Arc<[u8]>,
}

#[derive(Clone, Debug)]
pub enum ImageReady {
    Decoded(ImageHandle),
    Missing {
        source: Arc<str>,
        generation: u64,
    },
    Failed {
        source: Arc<str>,
        generation: u64,
        message: String,
    },
}

#[derive(Debug)]
pub enum ImageError {
    Missing,
    Failed(String),
}

/// Reads one source. Called only on the `uifw-io` thread.
pub trait ImageDecoder: Send + Sync {
    fn decode(&self, source: &str) -> Result<ImageHandle, ImageError>;

    fn decode_at(&self, source: &str, scale_bucket: u8) -> Result<ImageHandle, ImageError> {
        let _ = scale_bucket;
        self.decode(source)
    }

    fn decode_bytes(&self, source: &str, bytes: &[u8]) -> Result<ImageHandle, ImageError> {
        let _ = (source, bytes);
        Err(ImageError::Failed(
            "image bytes decoder is not linked".into(),
        ))
    }
}

#[derive(Clone, Debug)]
pub struct ImageRequest {
    pub source: Arc<str>,
    pub generation: u64,
    pub scale_bucket: u8,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct CacheKey {
    source: Arc<str>,
    bucket: u8,
}

#[derive(Debug)]
enum Entry {
    Ready {
        handle: ImageHandle,
        bytes: usize,
        used_frame: u64,
    },
    Missing {
        at: Instant,
    },
    Failed {
        at: Instant,
    },
}

const MISSING_TTL: Duration = Duration::from_secs(30);
const FAILED_TTL: Duration = Duration::from_secs(5);
const READY_LIMIT: usize = 64;
const BYTE_BUDGET: usize = 32 * 1024 * 1024;
pub const IMAGE_BODY_LIMIT: usize = 4 * 1024 * 1024;
pub const IMAGE_PIXEL_LIMIT: u64 = 8_000_000;

/// Density bucket for `point_scale`. 1, 2, or 3.
pub fn scale_bucket(point_scale: f32) -> u8 {
    if !point_scale.is_finite() || point_scale < 1.5 {
        1
    } else if point_scale < 2.5 {
        2
    } else {
        3
    }
}

/// Scheme and host only. Query strings stay out of logs.
pub fn source_log(source: &str) -> String {
    if let Some(rest) = source.strip_prefix("https://") {
        let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        format!("https://{host}")
    } else if let Some(rest) = source.strip_prefix("http://") {
        let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        format!("http://{host}")
    } else if let Some(rest) = source.strip_prefix("asset:") {
        let name = rest.split(['?', '#']).next().unwrap_or(rest);
        format!("asset:{name}")
    } else {
        source.split(['?', '#']).next().unwrap_or(source).to_owned()
    }
}

/// Render-thread cache. Pending sources are not inserted until [`ImageReady`].
pub struct ImageCache {
    entries: HashMap<CacheKey, Entry>,
    pending: HashMap<CacheKey, u64>,
    generation: u64,
    frame: u64,
    logged_missing: HashSet<CacheKey>,
    logged_over_budget: bool,
    counts: HashMap<Arc<str>, u32>,
    pinned: HashSet<CacheKey>,
}

impl ImageCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            pending: HashMap::new(),
            generation: 0,
            frame: 0,
            logged_missing: HashSet::new(),
            logged_over_budget: false,
            counts: HashMap::new(),
            pinned: HashSet::new(),
        }
    }

    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    pub fn pin(&mut self, list: &DrawList, bucket: u8) {
        self.pinned.clear();
        for source in list.image_sources() {
            let key = CacheKey {
                source: Arc::from(source),
                bucket,
            };
            if matches!(self.entries.get(&key), Some(Entry::Ready { .. })) {
                self.pinned.insert(key);
            }
        }
    }

    pub fn snapshot(&mut self, list: &DrawList, bucket: u8) -> Vec<(String, u32, u32, Arc<[u8]>)> {
        self.pin(list, bucket);
        let mut out = Vec::new();
        for source in list.image_sources() {
            let key = CacheKey {
                source: Arc::from(source),
                bucket,
            };
            if let Some(Entry::Ready {
                handle, used_frame, ..
            }) = self.entries.get_mut(&key)
            {
                *used_frame = self.frame;
                out.push((
                    source.to_owned(),
                    handle.size[0],
                    handle.size[1],
                    Arc::clone(&handle.pixels),
                ));
            }
        }
        out
    }

    pub fn resolve<'a>(&'a self, source: &str, bucket: u8) -> Option<ImagePixels<'a>> {
        let key = CacheKey {
            source: Arc::from(source),
            bucket,
        };
        match self.entries.get(&key) {
            Some(Entry::Ready { handle, .. }) => Some(ImagePixels {
                width: handle.size[0],
                height: handle.size[1],
                rgba: &handle.pixels,
            }),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn contains(&self, source: &str, bucket: u8) -> bool {
        let key = CacheKey {
            source: Arc::from(source),
            bucket,
        };
        self.entries
            .get(&key)
            .is_some_and(|entry| self.fresh(entry))
    }

    pub fn is_pending(&self, source: &str, bucket: u8) -> bool {
        let key = CacheKey {
            source: Arc::from(source),
            bucket,
        };
        self.pending.contains_key(&key)
    }

    pub fn is_negative(&self, source: &str, bucket: u8) -> bool {
        let key = CacheKey {
            source: Arc::from(source),
            bucket,
        };
        match self.entries.get(&key) {
            Some(entry @ (Entry::Missing { .. } | Entry::Failed { .. })) => self.fresh(entry),
            _ => false,
        }
    }

    /// One log line for a required image that is missing or failed.
    pub fn take_required_log(&mut self, source: &str, bucket: u8) -> Option<String> {
        let key = CacheKey {
            source: Arc::from(source),
            bucket,
        };
        let kind = match self.entries.get(&key) {
            Some(Entry::Missing { .. }) => "missing",
            Some(Entry::Failed { .. }) => "failed",
            _ => return None,
        };
        self.logged_missing
            .insert(key)
            .then(|| format!("image {kind}: {}", source_log(source)))
    }

    fn fresh(&self, entry: &Entry) -> bool {
        match entry {
            Entry::Ready { .. } => true,
            Entry::Missing { at } => at.elapsed() < MISSING_TTL,
            Entry::Failed { at } => at.elapsed() < FAILED_TTL,
        }
    }

    /// Marks `source` pending and returns the request to send to the IO thread.
    /// A second call while the first is in flight returns `None`.
    pub fn request(&mut self, source: &str, bucket: u8) -> Option<ImageRequest> {
        let key = CacheKey {
            source: Arc::from(source),
            bucket,
        };
        if let Some(entry) = self.entries.get(&key)
            && self.fresh(entry)
        {
            return None;
        }
        if self.pending.contains_key(&key) {
            return None;
        }
        self.entries.remove(&key);
        self.logged_missing.remove(&key);
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.pending.insert(key.clone(), generation);
        Some(ImageRequest {
            source: key.source,
            generation,
            scale_bucket: bucket,
        })
    }

    /// Installs a decode result. A stale generation is ignored.
    /// Returns a log line the first time a missing or failed image is recorded.
    pub fn apply(&mut self, ready: ImageReady) -> Option<String> {
        let (source, generation) = match &ready {
            ImageReady::Decoded(handle) => (Arc::clone(&handle.source), handle.generation),
            ImageReady::Missing { source, generation }
            | ImageReady::Failed {
                source, generation, ..
            } => (Arc::clone(source), *generation),
        };
        let bucket = self
            .pending
            .iter()
            .find_map(|(key, pending_generation)| {
                (key.source == source && *pending_generation == generation).then_some(key.bucket)
            })
            .unwrap_or(1);
        let key = CacheKey {
            source: Arc::clone(&source),
            bucket,
        };
        match self.pending.get(&key).copied() {
            Some(expected) if expected == generation => {
                self.pending.remove(&key);
            }
            _ => return None,
        }
        match ready {
            ImageReady::Decoded(handle) => {
                if !pixel_count_ok(handle.size) {
                    self.entries
                        .insert(key, Entry::Failed { at: Instant::now() });
                    return Some(format!(
                        "image failed: {} (pixel limit)",
                        source_log(&source)
                    ));
                }
                let bytes = handle.pixels.len();
                if !self.admit_ready(bytes) {
                    self.entries
                        .insert(key, Entry::Failed { at: Instant::now() });
                    if !self.logged_over_budget {
                        self.logged_over_budget = true;
                        return Some("image cache over budget".to_owned());
                    }
                    return None;
                }
                self.entries.insert(
                    key,
                    Entry::Ready {
                        handle,
                        bytes,
                        used_frame: 0,
                    },
                );
                None
            }
            ImageReady::Missing { source, .. } => {
                self.entries
                    .insert(key.clone(), Entry::Missing { at: Instant::now() });
                self.logged_missing
                    .insert(key)
                    .then(|| format!("image missing: {}", source_log(&source)))
            }
            ImageReady::Failed {
                source, message, ..
            } => {
                self.entries
                    .insert(key.clone(), Entry::Failed { at: Instant::now() });
                self.logged_missing
                    .insert(key)
                    .then(|| format!("image failed: {} ({message})", source_log(&source)))
            }
        }
    }

    fn admit_ready(&mut self, extra: usize) -> bool {
        if self.fits(extra) {
            return true;
        }
        self.evict_for(extra);
        self.fits(extra)
    }

    fn fits(&self, extra: usize) -> bool {
        let (count, bytes) = self.ready_usage();
        count < READY_LIMIT && bytes.saturating_add(extra) <= BYTE_BUDGET
    }

    fn ready_usage(&self) -> (usize, usize) {
        let mut count = 0;
        let mut bytes = 0;
        for entry in self.entries.values() {
            if let Entry::Ready { bytes: size, .. } = entry {
                count += 1;
                bytes += *size;
            }
        }
        (count, bytes)
    }

    fn evict_for(&mut self, extra: usize) {
        loop {
            if self.fits(extra) {
                return;
            }
            let victim = self
                .entries
                .iter()
                .filter_map(|(key, entry)| match entry {
                    Entry::Ready { used_frame, .. } if !self.pinned.contains(key) => {
                        Some((key.clone(), *used_frame))
                    }
                    _ => None,
                })
                .min_by_key(|(_, used)| *used);
            let Some((key, _)) = victim else {
                return;
            };
            self.entries.remove(&key);
        }
    }

    /// Drop cache entries for `source` and bump generation so an in-flight decode is stale.
    ///
    /// The shell posts [`crate::Command::ReloadImage`], which also arms one frame.
    pub fn reload(&mut self, source: &str) {
        self.entries.retain(|key, _| key.source.as_ref() != source);
        self.pending.retain(|key, _| key.source.as_ref() != source);
        self.logged_missing
            .retain(|key| key.source.as_ref() != source);
        self.generation = self.generation.wrapping_add(1);
    }

    /// Cancel in-flight decodes for sources whose live count became 0.
    /// Does not bump generation when `counts` is unused.
    pub fn sync_counts(&mut self, counts: &HashMap<Arc<str>, u32>) {
        let previous = std::mem::take(&mut self.counts);
        for (source, count) in counts {
            self.counts.insert(Arc::clone(source), *count);
            let was = previous.get(source).copied().unwrap_or(0);
            if was != 0 && *count == 0 {
                self.cancel_source(source);
            }
        }
        for (source, was) in previous {
            if was != 0 && !counts.contains_key(&source) {
                self.cancel_source(&source);
            }
        }
    }

    fn cancel_source(&mut self, source: &str) {
        let mut cancelled = false;
        self.pending.retain(|key, _| {
            if key.source.as_ref() == source {
                cancelled = true;
                false
            } else {
                true
            }
        });
        if cancelled {
            self.generation = self.generation.wrapping_add(1);
        }
    }
}

pub fn pixel_count_ok(size: [u32; 2]) -> bool {
    u64::from(size[0]).saturating_mul(u64::from(size[1])) <= IMAGE_PIXEL_LIMIT
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{Command, ContentMode, DrawList};

    fn handle(source: &str, generation: u64, bytes: usize) -> ImageHandle {
        ImageHandle {
            source: Arc::from(source),
            generation,
            size: [1, 1],
            pixels: Arc::from(vec![0u8; bytes.max(4)]),
        }
    }

    fn list(source: &str, required: bool) -> DrawList {
        DrawList {
            commands: vec![Command::Image {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
                source: Arc::from(source),
                mode: ContentMode::Fill,
                required,
            }],
        }
    }

    #[test]
    fn request_is_idempotent_while_pending() {
        let mut cache = ImageCache::new();
        assert!(cache.request("asset:logo", 2).is_some());
        assert!(cache.request("asset:logo", 2).is_none());
        assert!(cache.is_pending("asset:logo", 2));
    }

    #[test]
    fn stale_generation_is_ignored() {
        let mut cache = ImageCache::new();
        let request = cache.request("asset:logo", 1).unwrap();
        cache.pending.clear();
        cache.pending.insert(
            CacheKey {
                source: Arc::from("asset:logo"),
                bucket: 1,
            },
            request.generation.wrapping_add(1),
        );
        assert!(
            cache
                .apply(ImageReady::Decoded(handle(
                    "asset:logo",
                    request.generation,
                    4
                )))
                .is_none()
        );
        assert!(!cache.contains("asset:logo", 1));
    }

    #[test]
    fn missing_expires_and_can_be_requested_again() {
        let mut cache = ImageCache::new();
        let request = cache.request("asset:gone", 1).unwrap();
        assert!(
            cache
                .apply(ImageReady::Missing {
                    source: Arc::from("asset:gone"),
                    generation: request.generation,
                })
                .is_some()
        );
        assert!(cache.contains("asset:gone", 1));
        if let Some(Entry::Missing { at }) = cache.entries.values_mut().next() {
            *at = Instant::now() - MISSING_TTL - Duration::from_secs(1);
        }
        assert!(!cache.contains("asset:gone", 1));
        assert!(cache.request("asset:gone", 1).is_some());
    }

    #[test]
    fn reload_drops_the_key() {
        let mut cache = ImageCache::new();
        let request = cache.request("asset:logo", 2).unwrap();
        cache.apply(ImageReady::Decoded(handle(
            "asset:logo",
            request.generation,
            4,
        )));
        assert!(cache.contains("asset:logo", 2));
        cache.reload("asset:logo");
        assert!(!cache.contains("asset:logo", 2));
        assert!(cache.request("asset:logo", 2).is_some());
    }

    #[test]
    fn count_zero_cancels_pending() {
        let mut cache = ImageCache::new();
        cache.request("asset:logo", 1);
        assert!(cache.is_pending("asset:logo", 1));
        let mut counts = HashMap::new();
        counts.insert(Arc::from("asset:logo"), 1);
        cache.sync_counts(&counts);
        counts.insert(Arc::from("asset:logo"), 0);
        cache.sync_counts(&counts);
        assert!(!cache.is_pending("asset:logo", 1));
    }

    #[test]
    fn snapshot_marks_used_frame() {
        let mut cache = ImageCache::new();
        cache.begin_frame();
        let request = cache.request("asset:logo", 1).unwrap();
        cache.apply(ImageReady::Decoded(handle(
            "asset:logo",
            request.generation,
            4,
        )));
        let pixels = cache.snapshot(&list("asset:logo", true), 1);
        assert_eq!(pixels.len(), 1);
        match cache.entries.values().next() {
            Some(Entry::Ready { used_frame, .. }) => assert_eq!(*used_frame, cache.frame),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn source_log_drops_the_query() {
        assert_eq!(
            source_log("https://cdn.example/img.png?token=secret"),
            "https://cdn.example"
        );
        assert_eq!(source_log("asset:logo"), "asset:logo");
    }
}
