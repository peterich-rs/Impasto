//! APK asset path candidates. Decoding stays on `io`.

const EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

pub fn scale_bucket(point_scale: f32) -> u8 {
    if !point_scale.is_finite() || point_scale < 1.5 {
        1
    } else if point_scale < 2.5 {
        2
    } else {
        3
    }
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

/// `images/<name>@Nx.<ext>` candidates. First existing path wins.
pub fn asset_image_candidates(source: &str, point_scale: f32) -> Vec<String> {
    let Some(name) = asset_name(source) else {
        return Vec::new();
    };
    let densities: [&str; 3] = match scale_bucket(point_scale) {
        3 => ["@3x", "@2x", ""],
        2 => ["@2x", "@3x", ""],
        _ => ["", "@2x", "@3x"],
    };
    let mut out = Vec::with_capacity(densities.len() * EXTENSIONS.len());
    for density in densities {
        for ext in EXTENSIONS {
            out.push(format!("images/{name}{density}.{ext}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::asset_image_candidates;

    #[test]
    fn logo_tries_density_then_extensions() {
        let names = asset_image_candidates("asset:logo", 2.0);
        assert_eq!(names[0], "images/logo@2x.png");
        assert!(names.contains(&"images/logo.png".to_owned()));
        assert!(names.contains(&"images/logo.webp".to_owned()));
        assert!(asset_image_candidates("asset:../logo", 2.0).is_empty());
    }
}
