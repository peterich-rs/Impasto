//! Image bytes decoded on `io` and installed on the render thread.

pub(crate) mod remote;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Producer frame in straight-alpha sRGB RGBA8. The render thread owns the latest frame.
pub struct ExternalFrame {
    pub size: [u32; 2],
    pub pixels: Vec<u8>,
}

/// Imports a producer-owned frame on the render thread.
pub type ImportExternalFrame = Box<dyn Fn(&raster::Rasterizer) -> Option<raster::GpuImage> + Send>;

pub(crate) struct ExternalGpuFrame {
    pub import: ImportExternalFrame,
    pub image: Option<raster::GpuImage>,
}

/// Decoded image. `gpu` is set when `io` already uploaded the texture.
#[derive(Debug)]
pub struct ImageHandle {
    pub source: Arc<str>,
    pub generation: u64,
    pub size: [u32; 2],
    pub pixels: Arc<[u8]>,
    /// Present blits this. Empty pixels are fine once it is set.
    pub gpu: Option<raster::GpuImage>,
}

#[derive(Debug)]
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

impl ImageReady {
    pub(crate) fn source(&self) -> &str {
        match self {
            Self::Decoded(handle) => &handle.source,
            Self::Missing { source, .. } | Self::Failed { source, .. } => source,
        }
    }
}

#[derive(Debug)]
pub enum ImageError {
    Missing,
    Failed(String),
}

/// A physical display size rounded up to stable 64-pixel decode buckets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeTarget([u32; 2]);

impl DecodeTarget {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "positive bounds are checked and limited before rounding"
    )]
    pub(crate) fn from_layout(extent: [f32; 2], dpr: f32) -> Option<Self> {
        let extent = extent.map(|value| value * dpr);
        if extent
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0 || *value > 8192.0)
        {
            return None;
        }
        Some(Self(
            extent.map(|value| (value.ceil() as u32).next_multiple_of(64)),
        ))
    }

    /// Aspect-preserving dimensions sufficient for either fit or crop.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "rounded dimensions never exceed u32 source dimensions"
    )]
    pub fn fit(self, source: [u32; 2]) -> [u32; 2] {
        let factor = self
            .0
            .into_iter()
            .zip(source)
            .map(|(goal, dimension)| f64::from(goal) / f64::from(dimension.max(1)))
            .fold(0.0_f64, f64::max)
            .min(1.0);
        source.map(|value| (f64::from(value) * factor).ceil() as u32)
    }

    fn covers(self, other: Self) -> bool {
        self.0[0] >= other.0[0] && self.0[1] >= other.0[1]
    }
}

fn covers(existing: Option<DecodeTarget>, target: Option<DecodeTarget>) -> bool {
    match (existing, target) {
        (None, _) => true,
        (Some(existing), Some(target)) => existing.covers(target),
        (Some(_), None) => false,
    }
}

/// Reads one source. Called only on the `io` thread.
pub trait ImageDecoder: Send + Sync {
    fn read_packaged_asset(&self, _relative: &str, _limit: usize) -> Result<Arc<[u8]>, String> {
        Err("the host has no packaged assets".to_owned())
    }

    fn decode(&self, source: &str) -> Result<ImageHandle, ImageError>;

    fn decode_at(&self, source: &str, scale_bucket: u8) -> Result<ImageHandle, ImageError> {
        let _ = scale_bucket;
        self.decode(source)
    }

    fn decode_sized(
        &self,
        source: &str,
        scale_bucket: u8,
        _target: Option<DecodeTarget>,
    ) -> Result<ImageHandle, ImageError> {
        self.decode_at(source, scale_bucket)
    }

    fn decode_bytes_sized(
        &self,
        source: &str,
        bytes: &[u8],
        _target: Option<DecodeTarget>,
    ) -> Result<ImageHandle, ImageError> {
        self.decode_bytes(source, bytes)
    }

    fn decode_bytes(&self, source: &str, bytes: &[u8]) -> Result<ImageHandle, ImageError> {
        let _ = (source, bytes);
        Err(ImageError::Failed(
            "image bytes decoder is not linked".into(),
        ))
    }
}

/// Read a bounded resource from a directory-backed application bundle.
pub fn read_packaged_file(
    root: &std::path::Path,
    relative: &str,
    limit: usize,
) -> Result<Arc<[u8]>, String> {
    use std::fs::File;

    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err("asset is outside the package".to_owned());
    }
    let file = File::open(path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    let length = usize::try_from(length).map_err(|error| error.to_string())?;
    engine::session::read_asset_bytes(file, length, limit)
}

#[derive(Clone, Debug)]
pub struct ImageRequest {
    pub source: Arc<str>,
    pub generation: u64,
    pub scale_bucket: u8,
    pub target: Option<DecodeTarget>,
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
        target: Option<DecodeTarget>,
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
    pending: HashMap<CacheKey, (u64, Option<DecodeTarget>)>,
    targets: HashMap<Arc<str>, DecodeTarget>,
    failed_upgrades: HashMap<CacheKey, Instant>,
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
            targets: HashMap::new(),
            failed_upgrades: HashMap::new(),
            frame: 0,
            logged_missing: HashSet::new(),
            logged_over_budget: false,
            counts: HashMap::new(),
            pinned: HashSet::new(),
        }
    }

    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.failed_upgrades
            .retain(|key, at| at.elapsed() < FAILED_TTL && self.entries.contains_key(key));
    }

    pub(crate) fn set_layout_targets(&mut self, extents: HashMap<Arc<str>, [f32; 2]>, scale: f32) {
        self.targets = extents
            .into_iter()
            .filter_map(|(source, extent)| {
                DecodeTarget::from_layout(extent, scale).map(|target| (source, target))
            })
            .collect();
    }

    pub(crate) fn accepts(&self, source: &str, generation: u64) -> bool {
        self.pending
            .iter()
            .any(|(key, (id, _))| key.source.as_ref() == source && *id == generation)
    }

    pub fn pin_sources<'a>(&mut self, sources: impl IntoIterator<Item = &'a str>, bucket: u8) {
        self.pinned.clear();
        for source in sources {
            let key = CacheKey {
                source: Arc::from(source),
                bucket,
            };
            if matches!(self.entries.get(&key), Some(Entry::Ready { .. })) {
                self.pinned.insert(key);
            }
        }
    }

    pub fn snapshot_requirements(
        &mut self,
        requirements: &[(Arc<str>, bool)],
        bucket: u8,
    ) -> Vec<(String, u32, u32, Arc<[u8]>)> {
        self.pin_sources(
            requirements.iter().map(|(source, _)| source.as_ref()),
            bucket,
        );
        self.snapshot_pinned(
            requirements.iter().map(|(source, _)| source.as_ref()),
            bucket,
        )
    }

    fn snapshot_pinned<'a>(
        &mut self,
        sources: impl IntoIterator<Item = &'a str>,
        bucket: u8,
    ) -> Vec<(String, u32, u32, Arc<[u8]>)> {
        let mut out = Vec::new();
        for source in sources {
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

    pub(crate) fn is_ready(&self, source: &str, bucket: u8) -> bool {
        let key = CacheKey {
            source: Arc::from(source),
            bucket,
        };
        matches!(self.entries.get(&key), Some(Entry::Ready { .. }))
    }

    #[allow(dead_code, reason = "hosts ask whether a decode is already cached")]
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
        let target = self.targets.get(source).copied();
        let upgrading = matches!(self.entries.get(&key), Some(Entry::Ready { .. }));
        if self.pending.contains_key(&key)
            || self
                .failed_upgrades
                .get(&key)
                .is_some_and(|at| at.elapsed() < FAILED_TTL)
        {
            return None;
        }
        match self.entries.get(&key) {
            Some(Entry::Ready {
                target: existing, ..
            }) if covers(*existing, target) => return None,
            Some(entry) if !upgrading && self.fresh(entry) => return None,
            _ => {}
        }
        if !upgrading {
            self.entries.remove(&key);
            self.logged_missing.remove(&key);
        }
        static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
        let generation = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        self.pending.insert(key.clone(), (generation, target));
        Some(ImageRequest {
            source: key.source,
            generation,
            scale_bucket: bucket,
            target,
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
        let (key, target) = self.pending.iter().find_map(|(key, (id, target))| {
            (key.source == source && *id == generation).then(|| (key.clone(), *target))
        })?;
        self.pending.remove(&key);
        let upgrading = matches!(self.entries.get(&key), Some(Entry::Ready { .. }));
        if upgrading
            && !matches!(&ready, ImageReady::Decoded(handle) if pixel_count_ok(handle.size))
        {
            self.failed_upgrades.insert(key, Instant::now());
            return None;
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
                let bytes = if handle.gpu.is_some() {
                    handle.size[0]
                        .saturating_mul(handle.size[1])
                        .saturating_mul(4) as usize
                } else {
                    handle.pixels.len()
                };
                let previous = upgrading.then(|| self.entries.remove(&key)).flatten();
                if !self.admit_ready(bytes) {
                    if let Some(previous) = previous {
                        self.failed_upgrades.insert(key.clone(), Instant::now());
                        self.entries.insert(key, previous);
                        return None;
                    }
                    self.entries
                        .insert(key, Entry::Failed { at: Instant::now() });
                    if !self.logged_over_budget {
                        self.logged_over_budget = true;
                        return Some("image cache over budget".to_owned());
                    }
                    return None;
                }
                self.failed_upgrades.remove(&key);
                self.entries.insert(
                    key,
                    Entry::Ready {
                        handle,
                        bytes,
                        used_frame: 0,
                        target,
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

    /// Drops decoded pixels and in-flight keys. A late decode for the old generation is ignored.
    /// Retains uploaded textures into the raster cache for this scale bucket.
    /// Submit then blits them and does not create textures on the render thread.
    pub fn install_uploaded(&self, raster: &mut raster::Rasterizer, bucket: u8) {
        for (key, entry) in &self.entries {
            if key.bucket != bucket {
                continue;
            }
            if let Entry::Ready { handle, .. } = entry
                && let Some(gpu) = &handle.gpu
            {
                raster.install_uploaded(key.source.as_ref(), gpu);
            }
        }
    }

    pub fn trim(&mut self) {
        self.entries.clear();
        self.pending.clear();
        self.targets.clear();
        self.failed_upgrades.clear();
        self.logged_missing.clear();
        self.pinned.clear();
    }

    /// CPU-only entries can still be uploaded. GPU-only entries are requested
    /// from their source again. Completions for retired work lose their ticket.
    pub fn invalidate_gpu(&mut self) {
        self.entries.retain(|_, entry| match entry {
            Entry::Ready { handle, .. } => handle.gpu.is_none(),
            _ => true,
        });
        self.pending.clear();
    }

    /// Drop cache entries and retire in-flight tickets for `source`.
    ///
    /// The shell posts [`crate::Command::ReloadImage`], which also arms one frame.
    pub fn reload(&mut self, source: &str) {
        self.entries.retain(|key, _| key.source.as_ref() != source);
        self.pending.retain(|key, _| key.source.as_ref() != source);
        self.failed_upgrades
            .retain(|key, _| key.source.as_ref() != source);
        self.logged_missing
            .retain(|key| key.source.as_ref() != source);
    }

    /// Cancel in-flight decodes for sources whose live count became 0.
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
        self.pending.retain(|key, _| key.source.as_ref() != source);
    }
}

/// Uploads `handle` when a factory is available, then drops the CPU copy.
pub fn attach_upload(handle: &mut ImageHandle, factory: Option<&raster::TextureFactory>) {
    let Some(factory) = factory else {
        return;
    };
    let Some(gpu) = factory.upload_rgba(handle.size[0], handle.size[1], &handle.pixels) else {
        return;
    };
    handle.gpu = Some(gpu);
    handle.pixels = Arc::from([]);
}

/// Restores straight-alpha RGBA8 from an OS decoder's premultiplied buffer.
pub fn unpremultiply_rgba(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = u8::try_from((u16::from(*channel) * 255 + alpha / 2) / alpha)
                    .unwrap_or(u8::MAX);
            }
        }
    }
}

pub fn pixel_count_ok(size: [u32; 2]) -> bool {
    u64::from(size[0]).saturating_mul(u64::from(size[1])) <= IMAGE_PIXEL_LIMIT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(source: &str, generation: u64, bytes: usize) -> ImageHandle {
        ImageHandle {
            source: Arc::from(source),
            generation,
            size: [1, 1],
            pixels: Arc::from(vec![0u8; bytes.max(4)]),
            gpu: None,
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
            (request.generation.wrapping_add(1), request.target),
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
    fn trim_drops_pixels_and_ignores_an_in_flight_decode() {
        let mut cache = ImageCache::new();
        let request = cache.request("asset:logo", 1).unwrap();
        cache.apply(ImageReady::Decoded(handle(
            "asset:logo",
            request.generation,
            8,
        )));
        let in_flight = cache.request("asset:other", 1).unwrap();
        cache.trim();
        assert!(!cache.contains("asset:logo", 1));
        assert!(
            cache
                .apply(ImageReady::Decoded(handle(
                    "asset:other",
                    in_flight.generation,
                    4,
                )))
                .is_none()
        );
        assert!(!cache.contains("asset:other", 1));
        assert!(cache.request("asset:logo", 1).is_some());
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
        let pixels = cache.snapshot_requirements(&[(Arc::from("asset:logo"), true)], 1);
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
