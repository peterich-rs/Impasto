//! Cached child display lists. Released only after the root list that flattened them.

use std::collections::HashMap;

use engine::scene::{LayerTree, PictureId};

use crate::player::{HeldParagraph, RetainedPicture, Save, release_retained};
use crate::textures::TextureStore;
use crate::typography::Typography;
use crate::{ImagePixels, Status, SubmitStats};

#[derive(Default)]
pub(crate) struct PictureSwap {
    pub(crate) installed: Vec<PictureId>,
    pub(crate) replaced: Vec<(PictureId, RetainedPicture)>,
}

pub(crate) struct PictureCache {
    map: HashMap<PictureId, RetainedPicture>,
    builds: u64,
    warned: bool,
}

impl PictureCache {
    pub(crate) fn new() -> Self {
        Self {
            map: HashMap::new(),
            builds: 0,
            warned: false,
        }
    }

    pub(crate) fn builds(&self) -> u64 {
        self.builds
    }

    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    pub(crate) fn get(&self, id: &PictureId) -> Option<&RetainedPicture> {
        self.map.get(id)
    }

    pub(crate) fn note_limit(&mut self) {
        if self.map.len() > 4096 && !self.warned {
            self.warned = true;
        }
    }

    pub(crate) fn prepare<'pixels>(
        &mut self,
        tree: &LayerTree,
        point_scale: f32,
        resolve: &mut impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
        swap: &mut PictureSwap,
        typography: &mut Typography,
        textures: &mut TextureStore,
        pending: &mut Vec<HeldParagraph>,
        last_submit: &mut SubmitStats,
        impeller: impeller::ImpellerContext,
    ) -> Result<(), Status> {
        let stale = typography.stale();
        let generation = typography.generation();
        for picture in &tree.pictures {
            let reuse = self.map.get(&picture.id).is_some_and(|cached| {
                cached.generation == picture.generation
                    && !cached.has_external_texture
                    && !stale
                    && !waiting_blocks_reuse(&cached.waiting, |source| textures.contains(source))
            });
            if reuse {
                for (source, _) in picture.images() {
                    textures.touch_source(source, generation);
                }
                last_submit.picture_reused = last_submit.picture_reused.saturating_add(1);
                continue;
            }
            let built = crate::painter::build_picture(
                picture,
                point_scale,
                resolve,
                typography,
                textures,
                pending,
                impeller,
            )?;
            self.builds = self.builds.saturating_add(1);
            last_submit.picture_rebuilt = last_submit.picture_rebuilt.saturating_add(1);
            swap.installed.push(picture.id);
            if let Some(previous) = self.map.insert(picture.id, built) {
                swap.replaced.push((picture.id, previous));
            }
        }
        Ok(())
    }

    pub(crate) fn rollback(&mut self, swap: PictureSwap, typography: &mut Typography) {
        for id in swap.installed {
            if let Some(fresh) = self.map.remove(&id) {
                release_retained(fresh, typography);
            }
        }
        for (id, previous) in swap.replaced {
            if let Some(fresh) = self.map.insert(id, previous) {
                release_retained(fresh, typography);
            }
        }
    }

    pub(crate) fn evict_except(&mut self, tree: &LayerTree, typography: &mut Typography) {
        let stale: Vec<PictureId> = self
            .map
            .keys()
            .copied()
            .filter(|id| tree.picture(*id).is_none())
            .collect();
        for id in stale {
            if let Some(picture) = self.map.remove(&id) {
                release_retained(picture, typography);
            }
        }
    }

    pub(crate) fn release_all(&mut self, typography: &mut Typography) {
        let pictures = std::mem::take(&mut self.map);
        for (_, picture) in pictures {
            release_retained(picture, typography);
        }
    }
}

#[allow(
    dead_code,
    reason = "names the Save link so the type stays in this module"
)]
fn _save_link(_: &Save) {}

pub(crate) fn waiting_blocks_reuse(
    waiting: &[std::sync::Arc<str>],
    ready: impl Fn(&str) -> bool,
) -> bool {
    waiting.iter().any(|source| ready(source.as_ref()))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::waiting_blocks_reuse;

    #[test]
    fn a_ready_canvas_image_blocks_picture_reuse() {
        let waiting = [Arc::<str>::from("asset:a"), Arc::<str>::from("asset:b")];
        assert!(!waiting_blocks_reuse(&waiting, |source| source == "asset:c"));
        assert!(waiting_blocks_reuse(&waiting, |source| source == "asset:b"));
        assert!(!waiting_blocks_reuse(&[], |_| true));
    }
}
