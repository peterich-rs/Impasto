//! Asset path lookup. Decoding stays on `io`.

use std::path::{Path, PathBuf};

/// First available density candidate for a packaged asset reference.
pub fn resolve_asset_path(directory: &Path, source: &str, bucket: u8) -> Option<PathBuf> {
    for candidate in embedder::asset_image_candidates(source, bucket) {
        #[cfg(debug_assertions)]
        match embedder::dev_channel::asset_override(&candidate) {
            embedder::dev_channel::AssetOverride::File(path) => return Some(path),
            embedder::dev_channel::AssetOverride::Removed => continue,
            embedder::dev_channel::AssetOverride::Packaged => {}
        }
        let path = directory.join(candidate);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}
