//! An app-private cache of opaque pipeline data returned by wgpu.

use std::{
    fs::{self, OpenOptions},
    hash::{DefaultHasher, Hash, Hasher},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
};

const MAX_BYTES: usize = 16 * 1024 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct PipelineDiskCache {
    pub cache: wgpu::PipelineCache,
    updated: SyncSender<()>,
}

impl PipelineDiskCache {
    #[expect(
        unsafe_code,
        reason = "only prior wgpu cache data from the Android app-private directory is read"
    )]
    pub fn open(device: &wgpu::Device, adapter: &wgpu::Adapter, root: &Path) -> Option<Self> {
        if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
            return None;
        }
        let info = adapter.get_info();
        let adapter_key = wgpu::util::pipeline_cache_key(&info)?;
        let mut fingerprint = DefaultHasher::new();
        info.driver.hash(&mut fingerprint);
        info.driver_info.hash(&mut fingerprint);
        env!("CARGO_PKG_VERSION").hash(&mut fingerprint);
        [
            include_str!("pipelines/ui.wgsl"),
            include_str!("pipelines/text.wgsl"),
            include_str!("pipelines/text_lcd.wgsl"),
            include_str!("pipelines/image.wgsl"),
            include_str!("pipelines/external_image.wgsl"),
            include_str!("pipelines/blur.wgsl"),
            include_str!("pipelines/compute_blur.wgsl"),
            include_str!("pipelines/upload.wgsl"),
            include_str!("pipelines/upload_compute.wgsl"),
            include_str!("pipelines/view.wgsl"),
            include_str!("pipelines/clip.wgsl"),
            include_str!("pipelines/color_precision.wgsl"),
            include_str!("pipelines/gradient.wgsl"),
            include_str!("pipelines/mod.rs"),
        ]
        .hash(&mut fingerprint);
        let directory = root.join("impasto-wgpu");
        fs::create_dir_all(&directory).ok()?;
        let path = directory.join(format!("{adapter_key}-{:016x}", fingerprint.finish()));
        let data = load(&path);
        // SAFETY: the embedder passes Context.getCacheDir(). The only file
        // writer here serializes PipelineCache::get_data for this adapter.
        // The checksum detects corrupted writes and wgpu verifies the driver header.
        let cache = unsafe {
            device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                label: Some("impasto Vulkan pipeline cache"),
                data: data.as_deref(),
                fallback: true,
            })
        };
        let (updated, pending) = sync_channel(1);
        let writer = cache.clone();
        std::thread::Builder::new()
            .name("impasto pipeline cache".into())
            .spawn(move || {
                while pending.recv().is_ok() {
                    if let Some(data) = writer.get_data()
                        && data.len() <= MAX_BYTES
                    {
                        let _ = persist(&path, &data);
                    }
                }
            })
            .ok()?;
        Some(Self { cache, updated })
    }

    pub fn updated(&self) {
        let _ = self.updated.try_send(());
    }
}

fn checksum(data: &[u8]) -> [u8; 8] {
    let mut hash = DefaultHasher::new();
    data.hash(&mut hash);
    hash.finish().to_le_bytes()
}

fn load(path: &Path) -> Option<Vec<u8>> {
    if !fs::symlink_metadata(path).ok()?.is_file() {
        return None;
    }
    let mut file = fs::File::open(path).ok()?;
    let mut expected = [0; 8];
    file.read_exact(&mut expected).ok()?;
    let mut data = Vec::new();
    file.take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut data)
        .ok()?;
    (data.len() <= MAX_BYTES && checksum(&data) == expected).then_some(data)
}

fn persist(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let temporary: PathBuf = path.with_extension(format!("{}-{serial}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(&checksum(data))?;
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
