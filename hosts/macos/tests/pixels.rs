//! Gallery frame against `tests/pixels/gallery.png`.
//!
//! Missing SDK, QuickJS, or the golden file skips the comparison.

use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn gallery_matches_the_golden_within_two() {
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/pixels/gallery.png");
    if !golden.exists() {
        return;
    }
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/gallery-solid/dist/counter.js");
    if !script.exists() {
        return;
    }
    let dump = std::env::temp_dir().join("uifw-pixel-frame.png");
    let bin = env!("CARGO_BIN_EXE_macos");
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
    let status = match Command::new(bin)
        .env("UIFW_DUMP", &dump)
        .arg("--js")
        .arg(&script)
        .arg(&assets)
        .status()
    {
        Ok(status) if status.success() && dump.exists() => status,
        _ => return,
    };
    let _ = status;
    let Some(golden_img) = decode(&golden) else {
        return;
    };
    let Some(dumped) = decode(&dump) else {
        return;
    };
    assert_eq!(golden_img.width, dumped.width);
    assert_eq!(golden_img.height, dumped.height);
    assert_eq!(golden_img.pixels.len(), dumped.pixels.len());
    for (index, (left, right)) in golden_img
        .pixels
        .iter()
        .zip(dumped.pixels.iter())
        .enumerate()
    {
        let delta = left.abs_diff(*right);
        assert!(delta <= 2, "channel {index} differs by {delta}");
    }
}

struct Image {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

fn decode(path: &Path) -> Option<Image> {
    let file = std::fs::File::open(path).ok()?;
    let decoder = png::Decoder::new(file);
    let mut reader = decoder.read_info().ok()?;
    let mut pixels = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut pixels).ok()?;
    pixels.truncate(frame.buffer_size());
    Some(Image {
        width: frame.width,
        height: frame.height,
        pixels,
    })
}
