//! Cached child display lists. Released only after the root list that flattened them.

use std::collections::HashMap;

use protocol::{LayerTree, PictureId};

use crate::player::{RetainedPicture, Save, release_retained};
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

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    pub(crate) fn get(&self, id: &PictureId) -> Option<&RetainedPicture> {
        self.map.get(id)
    }

    pub(crate) fn note_limit(&mut self) {
        if self.map.len() > 4096 && !self.warned {
            self.warned = true;
            eprintln!("engine cache: {} pictures exceeds 4096", self.map.len());
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
        pending: &mut Vec<crate::player::ffi::ImpellerParagraph>,
        last_submit: &mut SubmitStats,
        impeller: crate::player::ffi::ImpellerContext,
    ) -> Result<(), Status> {
        let stale = typography.stale();
        let generation = typography.generation();
        for picture in &tree.pictures {
            if let Some(cached) = self.map.get(&picture.id)
                && cached.generation == picture.generation
                && !stale
            {
                textures.touch_commands(&picture.commands, generation);
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

    pub(crate) fn rollback(&mut self, swap: PictureSwap) {
        for id in swap.installed {
            if let Some(fresh) = self.map.remove(&id) {
                release_retained(fresh);
            }
        }
        for (id, previous) in swap.replaced {
            if let Some(fresh) = self.map.insert(id, previous) {
                release_retained(fresh);
            }
        }
    }

    pub(crate) fn evict_except(&mut self, tree: &LayerTree) {
        let stale: Vec<PictureId> = self
            .map
            .keys()
            .copied()
            .filter(|id| tree.picture(*id).is_none())
            .collect();
        for id in stale {
            if let Some(picture) = self.map.remove(&id) {
                release_retained(picture);
            }
        }
    }

    pub(crate) fn release_all(&mut self) {
        let pictures = std::mem::take(&mut self.map);
        for (_, picture) in pictures {
            release_retained(picture);
        }
    }
}

#[allow(dead_code)]
fn _save_link(_: &Save) {}
