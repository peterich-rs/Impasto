//! Image textures keyed by source. Evicted after the display list that drew them is released.

use std::collections::HashMap;

use protocol::Command;

use crate::player::ffi;

pub(crate) struct CachedTexture {
    pub(crate) texture: ffi::ImpellerTexture,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) used: u64,
}

pub(crate) struct TextureStore {
    map: HashMap<String, CachedTexture>,
    warned: bool,
}

impl TextureStore {
    pub(crate) fn new() -> Self {
        Self {
            map: HashMap::new(),
            warned: false,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    pub(crate) fn get_mut(&mut self, source: &str) -> Option<&mut CachedTexture> {
        self.map.get_mut(source)
    }

    pub(crate) fn insert(&mut self, source: String, texture: CachedTexture) {
        self.map.insert(source, texture);
    }

    pub(crate) fn touch_commands(&mut self, commands: &[Command], generation: u64) {
        for command in commands {
            if let Command::Image { source, .. } = command
                && let Some(cached) = self.map.get_mut(source.as_ref())
            {
                cached.used = generation;
            }
        }
    }

    pub(crate) fn note_limit(&mut self) {
        if self.map.len() > 256 && !self.warned {
            self.warned = true;
            eprintln!("engine cache: {} textures exceeds 256", self.map.len());
        }
    }

    pub(crate) fn evict(&mut self, generation: u64) {
        self.evict_if(|used| used != generation);
    }

    pub(crate) fn evict_over_cap(&mut self, generation: u64) {
        if self.map.len() > 64 * 4 {
            self.evict_if(|used| used != generation);
        }
    }

    fn evict_if(&mut self, drop_used: impl Fn(u64) -> bool) {
        let stale: Vec<String> = self
            .map
            .iter()
            .filter(|(_, cached)| drop_used(cached.used))
            .map(|(key, _)| key.clone())
            .collect();
        for key in stale {
            if let Some(cached) = self.map.remove(&key) {
                // SAFETY: a texture is evicted only after the display list that drew it
                // was released, or it was never installed.
                unsafe { ffi::ImpellerTextureRelease(cached.texture) };
            }
        }
    }

    pub(crate) fn release_all(&mut self) {
        for (_, cached) in self.map.drain() {
            // SAFETY: callers release the display list first, or no list is installed.
            unsafe { ffi::ImpellerTextureRelease(cached.texture) };
        }
    }
}
