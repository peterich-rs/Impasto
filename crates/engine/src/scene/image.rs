use std::collections::HashMap;
use std::sync::Arc;

use super::{Command, shader};

/// Media producer sources are supplied by the host.
pub const EXTERNAL_TEXTURE_PREFIX: &str = "external:";
/// Same-device producers managed by Raster use this external texture namespace.
pub const GPU_VIEW_TEXTURE_PREFIX: &str = "external:gpuview:";

/// How an image is fitted into its layout box.
///
/// [`ContentMode::NineSlice`] insets are source pixels. The player slices the
/// decoded image; record still emits one image command.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ContentMode {
    #[default]
    Fill,
    Fit,
    Crop,
    NineSlice {
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
    },
}

/// Preserve first-use order. A required use takes precedence over optional uses.
pub(super) fn dedup<'a>(uses: impl Iterator<Item = (&'a Arc<str>, bool)>) -> Vec<(Arc<str>, bool)> {
    let mut indices = HashMap::<&str, usize>::new();
    let mut requirements: Vec<(Arc<str>, bool)> = Vec::new();
    for (source, required) in uses {
        let index = *indices.entry(source).or_insert(requirements.len());
        if let Some((_, existing)) = requirements.get_mut(index) {
            *existing |= required;
        } else {
            requirements.push((Arc::clone(source), required));
        }
    }
    requirements
}

pub(super) fn requirements<'a>(
    commands: impl Iterator<Item = &'a Command>,
) -> Vec<(Arc<str>, bool)> {
    dedup(commands.flat_map(uses))
}

pub(super) fn uses(command: &Command) -> impl Iterator<Item = (&Arc<str>, bool)> {
    let (image, canvas_images, textures) = match command {
        Command::Image {
            source, required, ..
        } => (Some((source, *required)), &[][..], &[][..]),
        Command::Canvas { list, .. } => (None, list.images(), &[][..]),
        Command::ShaderEffect { textures, .. } => (None, &[][..], textures.as_ref()),
        _ => (None, &[][..], &[][..]),
    };
    image
        .into_iter()
        .chain(canvas_images.iter().map(|image| (&image.source, false)))
        .chain(shader::image_textures(textures).map(|source| (source, false)))
}
