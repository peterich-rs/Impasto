use std::env;
use std::fmt::Write;
use std::fs;
use std::io;
use std::path::PathBuf;

const VIEW: &str = include_str!("src/pipelines/view.wgsl");
const CLIP: &str = include_str!("src/pipelines/clip.wgsl");
const COLOR: &str = include_str!("src/pipelines/color_precision.wgsl");
const GRADIENT: &str = include_str!("src/pipelines/gradient.wgsl");
const TEXT: &str = include_str!("src/pipelines/text.wgsl");
const IMAGE: &str = include_str!("src/pipelines/image.wgsl");
const BLUR: &str = include_str!("src/pipelines/blur.wgsl");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=src/pipelines");
    let output = env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("missing OUT_DIR"))?;
    let mut declarations = String::new();
    for (name, variants) in [
        ("UI", 16),
        ("TEXT", 16),
        ("IMAGE", 4),
        ("IMAGE_ARRAY", 4),
        ("SMALL_IMAGES", 4),
        ("EXTERNAL_IMAGE", 4),
        ("BLUR", 4),
        ("INLINE_IMAGE", 1),
    ] {
        writeln!(declarations, "const {name}_SOURCES: [&str; {variants}] = [")?;
        for variant in 0..variants {
            let source = assemble(name, variant);
            let module = naga::front::wgsl::parse_str(&source).map_err(|error| {
                io::Error::other(format!(
                    "{name} variant {variant}: {}",
                    error.emit_to_string(&source)
                ))
            })?;
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .map_err(|error| {
                io::Error::other(format!(
                    "{name} variant {variant}: {}",
                    error.emit_to_string(&source)
                ))
            })?;
            writeln!(declarations, "{source:?},")?;
        }
        writeln!(declarations, "];")?;
    }
    fs::write(
        PathBuf::from(output).join("pipeline_sources.rs"),
        declarations,
    )?;
    Ok(())
}

fn assemble(name: &str, variant: usize) -> String {
    if name == "INLINE_IMAGE" {
        return include_str!("src/pipelines/inline_image.wgsl").to_owned();
    }
    let reduced = if variant & 1 != 0 {
        "enable f16;\nalias ReducedColor = vec4<f16>;\nalias ReducedScalar = f16;"
    } else {
        "alias ReducedColor = vec4<f32>;\nalias ReducedScalar = f32;"
    };
    let mut enable = "";
    let mut suffix = "";
    let source = match name {
        "UI" => {
            let ui = include_str!("src/pipelines/ui.wgsl");
            if variant & 8 == 0 {
                ui.replace(
                    "@interpolate(perspective, sample)",
                    "@interpolate(perspective, center)",
                )
            } else {
                ui.to_owned()
            }
        }
        "TEXT" => {
            if variant & 4 != 0 {
                enable = "enable dual_source_blending;";
                suffix = include_str!("src/pipelines/text_lcd.wgsl");
            }
            TEXT.to_owned()
        }
        "IMAGE" => IMAGE.to_owned(),
        "IMAGE_ARRAY" => {
            enable = "enable wgpu_binding_array;";
            IMAGE
                .replace(
                    "var image_tex: texture_2d<f32>;",
                    "var image_textures: binding_array<texture_2d<f32>, 32>;",
                )
                .replace(
                    "textureSampleLevel(image_tex,",
                    "textureSampleLevel(image_textures[data.info.x >> 8u],",
                )
                .replace(
                    "textureSampleGrad(image_tex,",
                    "textureSampleGrad(image_textures[data.info.x >> 8u],",
                )
        }
        "SMALL_IMAGES" => {
            let vertex = IMAGE
                .split_once("@fragment")
                .map_or(IMAGE, |(vertex, _)| vertex);
            let vertex = vertex.replace("texture_2d<f32>;", "texture_2d_array<f32>;");
            let vertex = vertex.replace(
                "var clip_mask_tex: texture_2d_array<f32>;",
                "var clip_mask_tex: texture_2d<f32>;",
            );
            format!(
                "{vertex}{}",
                include_str!("src/pipelines/small_images.wgsl")
            )
        }
        "EXTERNAL_IMAGE" => {
            let vertex = IMAGE
                .split_once("@fragment")
                .map_or(IMAGE, |(vertex, _)| vertex);
            format!(
                "{vertex}{}",
                include_str!("src/pipelines/external_image.wgsl")
            )
        }
        "BLUR" if variant & 2 != 0 => {
            BLUR.replace("@group(0) @binding(1) var<uniform>", "var<immediate>")
        }
        _ => BLUR.to_owned(),
    };
    let state = match name {
        "UI" => variant & 4 != 0,
        "TEXT" => variant & 8 != 0,
        "BLUR" => false,
        _ => variant & 2 != 0,
    };
    let view = if state {
        let (header, rest) = VIEW.split_once("@group(0)").unwrap_or((VIEW, ""));
        let (_, common) = rest.split_once("fn to_srgb").unwrap_or((rest, ""));
        format!(
            "{header}{}\nfn to_srgb{common}",
            include_str!("src/pipelines/state_table.wgsl")
        )
    } else {
        VIEW.to_owned()
    };
    let source = if state {
        source.replace(
            "const USE_STATE_TABLE = false",
            "const USE_STATE_TABLE = true",
        )
    } else {
        source
    };
    let clip = if name == "BLUR" {
        ""
    } else if state {
        include_str!("src/pipelines/state_clip.wgsl")
    } else {
        CLIP
    };
    let (gradient, paints) = if matches!(name, "UI" | "TEXT") {
        let storage = if variant & 2 != 0 {
            "@group(0) @binding(2) var<storage, read> paint: array<PaintExtra>;"
        } else {
            "@group(0) @binding(2) var<uniform> paint: array<PaintExtra, 128>;"
        };
        (GRADIENT, storage)
    } else {
        ("", "")
    };
    format!(
        "{enable}\n{reduced}\n{COLOR}\n{view}\n{source}\n{clip}\n{gradient}\n{paints}\n{suffix}"
    )
}
