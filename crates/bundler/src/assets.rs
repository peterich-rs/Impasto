//! Build-time image dimensions and font registration modules.
use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use serde_json::json;

pub(crate) fn module(assets: &Path, path: &Path) -> Result<Option<String>> {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return Ok(None);
    };
    match extension {
        "png" | "jpg" | "jpeg" | "webp" => Ok(Some(format!(
            "export default /* @__PURE__ */ Object.freeze({});",
            image(assets, path)?
        ))),
        "ttf" | "otf" => font(assets, path).map(Some),
        "svg" => crate::svg::compile(path).map(Some),
        _ => Ok(None),
    }
}

pub(crate) fn image(assets: &Path, path: &Path) -> Result<serde_json::Value> {
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
    Ok(json!({
        "uri": uri, "width": size.width, "height": size.height, "scales": scales,
    }))
}

fn font(assets: &Path, path: &Path) -> Result<String> {
    let outside = || format!("{}: put imported fonts under assets/", path.display());
    let assets = assets.canonicalize().with_context(outside)?;
    let path = path.canonicalize()?;
    let relative = path.strip_prefix(&assets).with_context(outside)?;
    if fs::metadata(&path)?.len() > engine::text::MAX_APP_FONT_BYTES as u64 {
        bail!("{}: font exceeds the runtime size limit", path.display());
    }
    let bytes = fs::read(&path)?;
    if bytes.len() > engine::text::MAX_APP_FONT_BYTES {
        bail!("{}: font exceeds the runtime size limit", path.display());
    }
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
    let uri = format!("asset:{}", relative.to_string_lossy().replace('\\', "/"));
    Ok(format!(
        "import {{ scene }} from 'scene'; scene.registerFontAsset({}, {}); export default {descriptor};",
        serde_json::to_string(&family)?,
        serde_json::to_string(&uri)?,
    ))
}
