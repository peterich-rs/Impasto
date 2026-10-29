//! Paragraph cache and the Impeller typography context. One owner on the render thread.

mod fonts;
mod key;
mod paint_store;
mod shape;

use std::cell::RefCell;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ptr;
use std::rc::Rc;

pub(crate) use fonts::FontRegistry;
pub(crate) use paint_store::PaintStore;

use fonts::Face;
use key::ParagraphStore;

use crate::player::ffi;
use crate::{ShapeStats, Status};

/// Fonts, the measure cache, and the colored paint slots.
///
/// `!Send`: the typography context stays on the render thread.
pub struct Typography {
    context: ffi::ImpellerTypographyContext,
    fonts: FontRegistry,
    measured: ParagraphStore,
    paints: PaintStore,
    retired: Vec<ffi::ImpellerParagraph>,
    generation: u64,
    list_installed: bool,
    stale: bool,
    stats: ShapeStats,
    warned_paragraphs: bool,
    probe_rebuilt: bool,
    probe_color: [u8; 4],
    render_thread: PhantomData<*mut ()>,
}

impl Typography {
    pub(crate) fn new(context: ffi::ImpellerTypographyContext) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            context,
            fonts: FontRegistry::new(),
            measured: ParagraphStore::new(),
            paints: PaintStore::new(),
            retired: Vec::new(),
            generation: 0,
            list_installed: false,
            stale: false,
            stats: ShapeStats::default(),
            warned_paragraphs: false,
            probe_rebuilt: false,
            probe_color: [0, 0, 0, 0],
            render_thread: PhantomData,
        }))
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn stale(&self) -> bool {
        self.stale
    }

    pub(crate) fn measured_len(&self) -> usize {
        self.measured.len()
    }

    /// Bumps the generation measure and retain stamp, then returns it.
    pub(crate) fn begin_submit(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    pub(crate) fn set_list_installed(&mut self, installed: bool) {
        self.list_installed = installed;
    }

    pub(crate) fn mark_stale(&mut self) {
        self.stale = true;
    }

    pub(crate) fn clear_stale(&mut self) {
        self.stale = false;
    }

    /// Drops the measure cache immediately. Used when no display list is installed.
    pub(crate) fn discard_measure_cache(&mut self) {
        for shaped in self.measured.drain_all() {
            // SAFETY: callers only discard the measure cache when no display list is installed.
            unsafe { ffi::ImpellerParagraphRelease(shaped.paragraph) };
        }
        self.stale = false;
    }

    pub(crate) fn reap_retired(&mut self) {
        for paragraph in self.retired.drain(..) {
            // SAFETY: the display list that could reference these was released, or none is installed.
            unsafe { ffi::ImpellerParagraphRelease(paragraph) };
        }
    }

    pub(crate) fn slot_color(&self, index: u32) -> Option<[u8; 4]> {
        self.paints.slots.get(index as usize).map(|slot| slot.color)
    }

    /// Swaps the paragraph a retained slot draws. The previous pointer is returned
    /// so the caller can retire it.
    pub(crate) fn replace_retained(
        &mut self,
        id: protocol::ParagraphId,
        color: [u8; 4],
        paragraph: ffi::ImpellerParagraph,
    ) -> Option<ffi::ImpellerParagraph> {
        self.paints
            .slots
            .get_mut(id.index as usize)
            .and_then(|slot| {
                if slot.generation == id.generation && slot.refs > 0 {
                    let old = slot.paragraph;
                    slot.paragraph = paragraph;
                    slot.color = color;
                    Some(old)
                } else {
                    None
                }
            })
    }

    pub(crate) fn store_text_probe(&mut self, rebuilt: bool, color: [u8; 4]) {
        self.probe_rebuilt = rebuilt;
        self.probe_color = color;
    }

    pub(crate) fn note_paragraph_limit(&mut self) {
        if self.measured.len() > 1024 && !self.warned_paragraphs {
            self.warned_paragraphs = true;
            eprintln!(
                "engine cache: {} paragraphs exceeds 1024",
                self.measured.len()
            );
        }
    }

    pub(crate) fn evict(&mut self) {
        let generation = self.generation;
        self.evict_measured(|used| used != generation);
        self.evict_provisional_paints();
    }

    pub(crate) fn evict_over_cap(&mut self) {
        let generation = self.generation;
        if self.measured.len() > 1024 {
            self.evict_measured(|used| used != generation);
        }
    }

    fn evict_measured(&mut self, drop_used: impl Fn(u64) -> bool) {
        let dropped = self.measured.drain_if(|shaped| drop_used(shaped.used));
        for shaped in dropped {
            // SAFETY: a paragraph is evicted only after the display list that drew it
            // was released, or it was never installed.
            unsafe { ffi::ImpellerParagraphRelease(shaped.paragraph) };
        }
    }

    fn evict_provisional_paints(&mut self) {
        let generation = self.generation;
        let stale: Vec<u32> = self
            .paints
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                if slot.refs == 0 && !slot.paragraph.is_null() && slot.used != generation {
                    Some(index as u32)
                } else {
                    None
                }
            })
            .collect();
        for index in stale {
            self.free_paint_slot(index);
        }
    }

    pub(crate) fn free_paint_slot(&mut self, index: u32) {
        let Some(slot) = self.paints.slots.get_mut(index as usize) else {
            return;
        };
        let paragraph = slot.paragraph;
        let key = slot.key.take();
        slot.paragraph = ptr::null_mut();
        slot.refs = 0;
        slot.width = 0.0;
        slot.height = 0.0;
        slot.color = [0, 0, 0, 0];
        self.paints.free.push(index);
        if let Some(key) = key {
            let hash = key::hash_key(&key);
            self.unindex_paint(hash, index);
        }
        if !paragraph.is_null() {
            self.retire_paragraph(paragraph);
        }
    }

    pub(crate) fn release_all_paragraphs(&mut self) {
        for slot in &mut self.paints.slots {
            if !slot.paragraph.is_null() {
                // SAFETY: Drop runs after the display list is released.
                unsafe { ffi::ImpellerParagraphRelease(slot.paragraph) };
                slot.paragraph = ptr::null_mut();
            }
        }
        self.discard_measure_cache();
        self.reap_retired();
    }

    /// Registers face bytes. Does not decide whether pictures are stale.
    pub(crate) fn register_face(&mut self, data: &[u8], family: &str) -> Result<(), Status> {
        if data.is_empty() {
            return Err(Status::BadScene);
        }
        let family = std::ffi::CString::new(family).map_err(|_| Status::BadScene)?;
        if family.as_bytes().is_empty() {
            return Err(Status::BadScene);
        }
        let owned = data.to_vec();
        let mapping = ffi::ImpellerMapping {
            data: owned.as_ptr(),
            length: owned.len() as u64,
            // Impeller invokes this even when registration fails. A null pointer crashes.
            // The callback does not free `owned`.
            on_release: Some(crate::player::keep_font_bytes),
        };
        // SAFETY: `mapping.data` points at `owned` for this call. Failure drops the
        // mapping inside Impeller before `owned` is stored. Success moves `owned` into
        // `faces`, which outlives the typeface that borrowed it.
        let registered = unsafe {
            ffi::ImpellerTypographyContextRegisterFont(
                self.context,
                &mapping,
                ptr::null_mut(),
                family.as_ptr(),
            )
        };
        if !registered {
            return Err(Status::BadScene);
        }
        if self.fonts.default_family.is_none() {
            self.fonts.default_family = Some(family.clone());
        }
        if let Some(face) = self
            .fonts
            .faces
            .iter_mut()
            .find(|face| face.family.as_bytes() == family.as_bytes())
        {
            face.bytes = owned;
            face.family = family;
        } else {
            self.fonts.faces.push(Face {
                bytes: owned,
                family,
            });
        }
        Ok(())
    }
}

impl Drop for Typography {
    fn drop(&mut self) {
        self.release_all_paragraphs();
        // SAFETY: the handle came from ImpellerTypographyContextNew. Release is a no-op on null.
        // Face bytes drop with `fonts` after this returns, so they outlive the context.
        unsafe { ffi::ImpellerTypographyContextRelease(self.context) };
    }
}

/// Keeps the `Rc` from being dropped by `Engine`'s field drop. `Engine::drop` drops it
/// after display lists are released and before the Impeller context.
pub(crate) type TypographyHandle = ManuallyDrop<Rc<RefCell<Typography>>>;
