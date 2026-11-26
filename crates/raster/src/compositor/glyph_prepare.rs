//! Finds the exact glyph variants for new pictures before the next present.

use super::{Compositor, GlyphTransform};
use crate::compiler::{self, CompiledPicture, Draw, IDENTITY, PictureBounds};
use crate::gpu::Gpu;
use engine::scene::{LayerKind, LayerTree, PictureId};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

// Bound speculative CPU work and temporary bitmap memory on text-heavy pages.
const PREFETCH_LIMIT: usize = 2048;

pub(crate) struct GlyphScene<'scene> {
    pub tree: &'scene LayerTree,
    pub pictures: &'scene HashMap<PictureId, Arc<CompiledPicture>>,
    pub fonts: &'scene crate::FontLibrary,
    pub scale: f32,
}

impl Compositor {
    pub(crate) fn prepare_glyphs(
        &mut self,
        pool: &rayon::ThreadPool,
        gpu: &Gpu,
        scene: GlyphScene<'_>,
        rebuilt: &HashSet<PictureId>,
    ) {
        self.atlas.clear_prepared();
        let viewport = self.last_viewport.map(|[width, height]| {
            [
                0.0,
                0.0,
                width as f32 / scene.scale,
                height as f32 / scene.scale,
            ]
        });
        let mut keys = HashSet::new();
        let mut distance = HashSet::new();
        let mut remaining = vec![(scene.tree.root, IDENTITY)];
        'layers: while let Some((id, parent)) = remaining.pop() {
            let Some(layer) = scene.tree.layers.get(id.0 as usize) else {
                continue;
            };
            if let Some(next) = layer.next {
                remaining.push((next, parent));
            }
            if let Some(child) = layer.first_child {
                remaining.push((
                    child,
                    compiler::compose(parent, compiler::layer_transform(&layer.kind)),
                ));
            }
            let LayerKind::Picture { picture } = layer.kind else {
                continue;
            };
            if !rebuilt.contains(&picture) {
                continue;
            }
            let Some(picture) = scene.pictures.get(&picture) else {
                continue;
            };
            let origin = compiler::compose(
                parent,
                [1.0, 0.0, 0.0, 1.0, picture.origin.0, picture.origin.1],
            );
            for scope in &picture.scopes {
                if viewport.is_some_and(|viewport| {
                    scope
                        .bounds_in(origin, scene.scale)
                        .expand(1.0 / scene.scale)
                        .clip(viewport)
                        == PictureBounds::Empty
                }) {
                    continue;
                }
                let transform = GlyphTransform::new(
                    compiler::compose(origin, scope.state.transform),
                    scene.scale,
                );
                for glyph in scope
                    .draws
                    .iter()
                    .filter_map(|draw| match draw {
                        Draw::Glyphs(glyphs) => Some(glyphs),
                        _ => None,
                    })
                    .flatten()
                {
                    if glyph.decoration.is_some() {
                        continue;
                    }
                    if self.use_msdf && transform.distance_field(glyph.size) {
                        if distance.len() < PREFETCH_LIMIT {
                            distance.insert(crate::outline::OutlineKey::new(glyph));
                        }
                        continue;
                    }
                    if transform.vector(glyph.size) {
                        continue;
                    }
                    let position = transform.position(glyph);
                    let size = glyph.size * transform.pixels;
                    // The output is unknown at submit, so LCD-capable devices
                    // also prefetch the optional LCD variant for plain text.
                    let lcd = transform.aligned && gpu.caps.lcd && glyph.gradient.is_none();
                    for lcd in [false, lcd].into_iter().take(1 + usize::from(lcd)) {
                        if let Some(key) = self.atlas.missing_glyph(gpu, glyph, size, position, lcd)
                        {
                            keys.insert(key);
                        }
                        if keys.len() >= PREFETCH_LIMIT {
                            break 'layers;
                        }
                    }
                }
            }
        }
        let (atlas, msdf) = (&mut self.atlas, &mut self.msdf);
        pool.install(|| {
            rayon::join(
                || atlas.prepare(pool, scene.fonts, keys),
                || msdf.prepare(pool, scene.fonts, distance),
            )
        });
    }
}
