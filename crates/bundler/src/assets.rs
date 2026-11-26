//! Build-time image dimensions and font registration modules.
use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::json;

pub(crate) fn module(assets: &Path, path: &Path) -> Result<Option<String>> {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return Ok(None);
    };
    match extension {
        "png" | "jpg" | "jpeg" | "webp" => image(assets, path).map(Some),
        "ttf" | "otf" => font(path).map(Some),
        "svg" => crate::svg::compile(path).map(Some),
        _ => Ok(None),
    }
}

fn image(assets: &Path, path: &Path) -> Result<String> {
    let asset_root = assets
        .canonicalize()
        .with_context(|| format!("{}: put imported images under assets/", path.display()))?;
    let path = path.canonicalize()?;
    let relative = path
        .strip_prefix(asset_root)
        .with_context(|| format!("{}: put imported images under assets/", path.display()))?;
    let size =
        imagesize::size(&path).with_context(|| format!("read dimensions of {}", path.display()))?;
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .context("image name is not UTF-8")?;
    let parent = path.parent().context("image has no directory")?;
    let extension = path
        .extension()
        .and_then(|name| name.to_str())
        .context("image has no extension")?;
    let mut scales = vec![1];
    for scale in [2, 3] {
        let variant = parent.join(format!("{stem}@{scale}x.{extension}"));
        if variant.is_file() {
            let candidate = imagesize::size(&variant)?;
            if size.width.checked_mul(scale) != Some(candidate.width)
                || size.height.checked_mul(scale) != Some(candidate.height)
            {
                bail!("{}: expected a {scale}x image", variant.display());
            }
            scales.push(scale);
        }
    }
    let uri = format!("asset:{}", relative.to_string_lossy().replace('\\', "/"));
    Ok(format!(
        "export default {};",
        json!({
            "uri": uri, "width": size.width, "height": size.height, "scales": scales,
        })
    ))
}

fn font(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    let face = ttf_parser::Face::parse(&bytes, 0)
        .with_context(|| format!("invalid font {}", path.display()))?;
    let family = face
        .names()
        .into_iter()
        .filter(|name| {
            name.name_id == ttf_parser::name_id::TYPOGRAPHIC_FAMILY
                || name.name_id == ttf_parser::name_id::FAMILY
        })
        .find_map(|name| name.to_string())
        .with_context(|| format!("{}: font has no Unicode family name", path.display()))?;
    let descriptor = json!({"family": family, "weight": face.weight().to_number()});
    let payload = STANDARD.encode(bytes);
    let data = serde_json::to_string(&payload)?;
    let family = serde_json::to_string(&family)?;
    // Decode a known build-time base64 string. The runtime module deliberately
    // keeps browser-only globals out of application code.
    Ok(format!(
        r#"
import {{ scene }} from "scene";
const source = {data};
const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const size = source.length / 4 * 3 - (source.endsWith("==") ? 2 : source.endsWith("=") ? 1 : 0);
const bytes = new Uint8Array(size);
for (let i = 0, out = 0; i < source.length; i += 4) {{
  const bits = (alphabet.indexOf(source[i]) << 18) |
    (alphabet.indexOf(source[i + 1]) << 12) |
    (Math.max(alphabet.indexOf(source[i + 2]), 0) << 6) |
    Math.max(alphabet.indexOf(source[i + 3]), 0);
  bytes[out++] = bits >> 16;
  if (out < size) bytes[out++] = bits >> 8;
  if (out < size) bytes[out++] = bits;
}}
scene.registerFont({family}, bytes);
export default {descriptor};
"#
    ))
}
