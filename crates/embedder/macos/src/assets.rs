//! Asset path lookup. Decoding stays on `io`.

use std::path::{Path, PathBuf};

const EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

/// Density bucket 1, 2, or 3 from `point_scale`.
pub fn scale_bucket(point_scale: f32) -> u8 {
    if !point_scale.is_finite() || point_scale < 1.5 {
        1
    } else if point_scale < 2.5 {
        2
    } else {
        3
    }
}

/// Relative candidates for `asset:name`, highest matching density first.
pub fn asset_candidates(name: &str, bucket: u8) -> Vec<String> {
    let densities: [&str; 3] = match bucket {
        3 => ["@3x", "@2x", ""],
        2 => ["@2x", "@3x", ""],
        _ => ["", "@2x", "@3x"],
    };
    let mut out = Vec::with_capacity(densities.len() * EXTENSIONS.len());
    for density in densities {
        for ext in EXTENSIONS {
            out.push(format!("{name}{density}.{ext}"));
        }
    }
    out
}

fn asset_name(source: &str) -> Option<String> {
    let name = source.strip_prefix("asset:")?;
    if name.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for part in name.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains('\\') {
            return None;
        }
        parts.push(part);
    }
    Some(parts.join("/"))
}

/// First existing file for `source` under `directory`.
pub fn resolve_asset_path(directory: &Path, source: &str, point_scale: f32) -> Option<PathBuf> {
    let name = asset_name(source)?;
    let bucket = scale_bucket(point_scale);
    for candidate in asset_candidates(&name, bucket) {
        let path = directory.join(candidate);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{asset_candidates, asset_name, scale_bucket};

    #[test]
    fn logo_candidates_prefer_the_scale_bucket() {
        assert_eq!(scale_bucket(1.0), 1);
        assert_eq!(scale_bucket(2.0), 2);
        assert_eq!(scale_bucket(3.0), 3);
        assert_eq!(asset_candidates("logo", 2)[0], "logo@2x.png");
        assert_eq!(asset_candidates("logo", 3)[0], "logo@3x.png");
        assert_eq!(asset_candidates("logo", 1)[0], "logo.png");
        assert!(
            asset_candidates("logo", 2)
                .iter()
                .any(|name| name == "logo.webp")
        );
    }

    #[test]
    fn parent_segments_are_rejected() {
        assert!(asset_name("asset:../logo").is_none());
        assert!(asset_name("logo").is_none());
        assert!(asset_name("asset:").is_none());
        assert_eq!(asset_name("asset:logo").as_deref(), Some("logo"));
    }
}
