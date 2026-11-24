//! Finds exact glyph variants before replay consumes atlas placements.

use super::{Frame, GlyphTransform, Image};
use crate::compiler::{self, Draw, IDENTITY};
use engine::scene::LayerKind;
use std::collections::HashSet;

// Bound speculative CPU work and temporary bitmap memory on text-heavy pages.
const PREFETCH_LIMIT: usize = 768;

impl Frame<'_> {
    pub(super) fn prepare_glyphs(&mut self, target: &Image) {
        if !self.scene.prepare_glyphs {
            return;
        }
        let Some(pool) = crate::prepare::pool() else {
            return;
        };
        if self.scene.slice.is_none() {
            if self.compositor.glyph_prefetch_revision == Some(self.scene.scene_revision) {
                return;
            }
            self.compositor.glyph_prefetch_revision = Some(self.scene.scene_revision);
        }
        let mut keys = HashSet::new();
        let mut remaining = vec![(self.scene.tree.root, IDENTITY)];
        while let Some((id, parent)) = remaining.pop() {
            let Some(layer) = self.scene.tree.layers.get(id.0 as usize) else {
                continue;
            };
            if let Some(next) = layer.next {
                remaining.push((next, parent));
            }
            if self.scene.slice.is_some_and(|slice| !slice.contains(&id)) {
                continue;
            }
            if let LayerKind::Picture { picture } = layer.kind
                && let Some(picture) = self.scene.pictures.get(&picture)
                && picture.has_glyphs
            {
                let origin = compiler::compose(
                    parent,
                    [1.0, 0.0, 0.0, 1.0, picture.origin.0, picture.origin.1],
                );
                let scale = self.scene.scale;
                let bounds = (origin[..4] == IDENTITY[..4] && scale >= picture.scale)
                    .then_some(picture.scope_blocks.as_slice());
                for (index, scopes) in picture
                    .scopes
                    .chunks(compiler::SCOPES_PER_BLOCK)
                    .enumerate()
                {
                    if bounds
                        .and_then(|bounds| bounds.get(index))
                        .is_some_and(|bounds| {
                            self.area(bounds.transform(origin).expand(1.0 / scale), target)
                                .is_none()
                        })
                    {
                        continue;
                    }
                    for scope in scopes {
                        if self
                            .area(scope.bounds_in(origin, self.scene.scale), target)
                            .is_none()
                        {
                            continue;
                        }
                        let transform = GlyphTransform::new(
                            compiler::compose(origin, scope.state.transform),
                            self.scene.scale,
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
                            if glyph.decoration.is_some() || transform.vector(glyph.size) {
                                continue;
                            }
                            let position = transform.position(glyph);
                            let size = glyph.size * transform.pixels;
                            let mut add = |lcd| {
                                if let Some(key) = self
                                    .compositor
                                    .atlas
                                    .missing_glyph(self.gpu, glyph, size, position, lcd)
                                {
                                    keys.insert(key);
                                }
                            };
                            add(false);
                            if transform.aligned
                                && self.gpu.caps.lcd
                                && target.format.is_srgb()
                                && target.view == self.root
                                && glyph.gradient.is_none()
                            {
                                add(true);
                            }
                            if keys.len() >= PREFETCH_LIMIT {
                                self.compositor.atlas.prepare(pool, self.scene.fonts, keys);
                                return;
                            }
                        }
                    }
                }
            }
            if let Some(child) = layer.first_child {
                remaining.push((
                    child,
                    compiler::compose(parent, compiler::layer_transform(&layer.kind)),
                ));
            }
        }
        self.compositor.atlas.prepare(pool, self.scene.fonts, keys);
    }
}
