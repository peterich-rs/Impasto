//! Reuses a blurred backdrop when damage proves its input pixels are unchanged.

use super::{Coverage, Frame, FullPass, Image, Status, expand_pixels, intersect_pixels};
use crate::DamageRect;

const CACHE_BUDGET: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq)]
struct Key {
    size: [u32; 2],
    source: [u32; 4],
    output: [u32; 4],
    radius: f32,
    format: wgpu::TextureFormat,
}

struct Entry {
    key: Key,
    revision: u64,
    source: Image,
    blurred: Image,
    submitted: bool,
}

impl Entry {
    fn bytes(&self) -> u64 {
        self.source.bytes() + self.blurred.bytes()
    }
}

#[derive(Default)]
pub(super) struct BackdropCache {
    entries: Vec<Option<Entry>>,
    cursor: usize,
    bytes: u64,
}

impl BackdropCache {
    pub(super) fn contains(&self, view: &wgpu::TextureView) -> bool {
        self.entries.iter().flatten().any(|entry| {
            entry.source.view.physical() == Some(view)
                || entry.blurred.view.physical() == Some(view)
        })
    }

    pub(super) fn begin(&mut self) {
        self.entries.truncate(self.cursor);
        self.cursor = 0;
        for slot in &mut self.entries {
            if slot.as_ref().is_some_and(|entry| !entry.submitted) {
                *slot = None;
            }
        }
        self.bytes = self.entries.iter().flatten().map(Entry::bytes).sum();
    }

    pub(super) fn submitted(&mut self) {
        for entry in self.entries.iter_mut().flatten() {
            entry.submitted = true;
        }
    }

    fn take(&mut self) -> (usize, Option<Entry>) {
        let index = self.cursor;
        self.cursor += 1;
        if self.entries.len() <= index {
            self.entries.push(None);
        }
        let entry = self.entries[index].take();
        if let Some(entry) = &entry {
            self.bytes -= entry.bytes();
        }
        (index, entry)
    }

    fn insert(&mut self, slot: usize, entry: Entry) -> bool {
        let bytes = entry.bytes();
        if self.bytes + bytes > CACHE_BUDGET {
            return false;
        }
        self.entries[slot] = Some(entry);
        self.bytes += bytes;
        true
    }
}

impl Frame<'_> {
    pub(super) fn paint_backdrop(
        &mut self,
        target: &Image,
        coverage: &Coverage,
        radius: f32,
        material: Option<FullPass>,
    ) -> Result<(), Status> {
        let dst = coverage.scissor.unwrap_or_else(|| target.area());
        let src = expand_pixels(dst, radius, target.area());
        if target.view == self.root {
            self.opaque.clear();
        }
        let kind = material.unwrap_or_else(|| {
            if coverage.mask.is_none() && coverage.count == 0 {
                FullPass::Copy
            } else {
                FullPass::Backdrop
            }
        });
        if radius <= 0.0 {
            let snapshot = self.snapshot(target, src)?;
            self.full(&snapshot, Some(&snapshot), coverage, target, dst, kind)?;
            self.release(snapshot);
            return Ok(());
        }
        let can_cache = target.view == self.main
            && self.scene.slice.is_none()
            && !self.scene.tree.has_renderer_work();
        let slot = if can_cache {
            let (index, entry) = self.compositor.backdrops.take();
            let key = Key {
                size: target.size,
                source: src,
                output: dst,
                radius,
                format: target.format,
            };
            let unchanged = match self.scene.damage {
                DamageRect::Full => false,
                damage => damage
                    .pixels(target.size)
                    .is_none_or(|area| intersect_pixels(area, src).is_none()),
            };
            if let Some(mut entry) = entry
                && entry.key == key
                && (entry.revision == self.scene.previous_revision
                    || entry.revision == self.scene.revision)
                && unchanged
            {
                self.full(
                    &entry.blurred,
                    Some(&entry.source),
                    coverage,
                    target,
                    dst,
                    kind,
                )?;
                entry.revision = self.scene.revision;
                self.compositor.backdrops.insert(index, entry);
                self.compositor.render_stats.backdrop_cache_hits += 1;
                return Ok(());
            }
            // Keep a result only after the source proves stable. During motion the
            // temporary textures should return to the pool every frame.
            unchanged.then_some((index, key))
        } else {
            None
        };
        let snapshot = self.snapshot(target, src)?;
        let blurred = self.blur(&snapshot, radius, true)?;
        if let Some((slot, key)) = slot
            && self.compositor.backdrops.bytes + snapshot.bytes() + blurred.bytes() <= CACHE_BUDGET
        {
            let source = self.retain_attachment(&snapshot)?;
            let filtered = self.retain_attachment(&blurred)?;
            self.full(&filtered, Some(&source), coverage, target, dst, kind)?;
            self.compositor.offscreen_live_bytes -= source.bytes() + filtered.bytes();
            self.compositor.backdrops.insert(
                slot,
                Entry {
                    key,
                    revision: self.scene.revision,
                    source,
                    blurred: filtered,
                    submitted: false,
                },
            );
        } else {
            self.full(&blurred, Some(&snapshot), coverage, target, dst, kind)?;
            self.release(blurred);
            self.release(snapshot);
        }
        Ok(())
    }
}
