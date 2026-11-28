//! One font library per window, shared by the UI shaper and the raster queue.
//!
//! The UI queue owns Fontique and publishes immutable font bytes here. Raster
//! resolves each face ID under a short lock and scales the retained blob locally.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use fontique::{Blob, FamilyId, FontInfoOverride, GenericFamily};
use parley::FontContext;

use crate::status::Status;
use engine::scene::FontFaceId;

/// One face file the library keeps readable.
struct Face {
    /// Holds the mapped or copied file. Fontique hands out `Blob` handles
    /// for the same data; keeping one alive keeps the address stable.
    bytes: Blob<u8>,
    index: u32,
    /// Set while a scene, a compiled picture, or a pending frame still
    /// names this face.
    live: Weak<FontFaceId>,
    /// A registered face is kept alive by its family mapping, so it can be
    /// resolved even when the caller dropped the IDs registration returned.
    /// `None` for a system fallback face, or a replaced registration, both
    /// of which are swept once no run names them.
    retained: Option<Arc<FontFaceId>>,
    /// Fontique family this face is mapped to, for registered faces.
    family: Option<String>,
}

#[derive(Default)]
struct Library {
    faces: HashMap<FontFaceId, Face>,
    /// Borrowed identity; each entry is backed by the corresponding live Face.
    by_blob: HashMap<(BlobAddress, u32), FontFaceId>,
    /// Registered family names, most recent first.
    families: Vec<(String, Blob<u8>)>,
    next_id: u64,
}

/// Shared, clonable font library.
#[derive(Clone)]
pub struct FontLibrary {
    inner: Arc<SharedLibrary>,
}

#[derive(Default)]
struct SharedLibrary {
    data: Mutex<Library>,
    registration: Mutex<()>,
    revision: AtomicU64,
}

impl std::fmt::Debug for FontLibrary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let library = self.lock();
        formatter
            .debug_struct("FontLibrary")
            .field("revision", &self.revision())
            .field("faces", &library.faces.len())
            .finish_non_exhaustive()
    }
}

impl FontLibrary {
    /// An empty library. System fallbacks still resolve through Fontique.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SharedLibrary::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Library> {
        self.inner
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Changes whenever a registration altered the family mapping. Shapers
    /// compare this to decide whether cached layouts are still valid.
    pub fn revision(&self) -> u64 {
        self.inner.revision.load(Ordering::Acquire)
    }

    /// Bytes and collection index of one face, read under the library lock.
    /// `None` after the bytes were freed because nothing named the face
    /// anymore. The raster copies what it needs and releases the lock before
    /// scaling.
    pub fn with_face<R>(&self, id: &FontFaceId, read: impl FnOnce(&[u8], u32) -> R) -> Option<R> {
        let (bytes, index) = self.snapshot(id)?;
        Some(read(bytes.data(), index))
    }

    pub(crate) fn snapshot(&self, id: &FontFaceId) -> Option<(Blob<u8>, u32)> {
        let library = self.lock();
        let face = library.faces.get(id)?;
        Some((face.bytes.clone(), face.index))
    }

    /// True while the library still holds bytes for `id`.
    pub fn has_face(&self, id: &FontFaceId) -> bool {
        self.lock().faces.contains_key(id)
    }

    /// Identity of a face Fontique selected, adopting it when it is not a
    /// registered face. Returns a strong ID the caller keeps in a shaped run.
    pub fn resolve(&self, data: &parley::FontData) -> Arc<FontFaceId> {
        let key = (BlobAddress::new(&data.data), data.index);
        let mut library = self.lock();
        if let Some(id) = library.by_blob.get(&key).copied()
            && let Some(face) = library.faces.get_mut(&id)
        {
            // Preserve the identity of a face with the same living blob.
            if let Some(live) = face.live.upgrade() {
                return live;
            }
            let live = Arc::new(id);
            face.live = Arc::downgrade(&live);
            return live;
        }
        library.insert(data.data.clone(), data.index, None)
    }

    /// Registers every face in `bytes` under `family`.
    ///
    /// Returns one ID per face the file contains, in file order. Invalid
    /// bytes return [`Status::BadScene`] and leave the library unchanged.
    /// A same-family registration replaces the mapping; the previous faces
    /// stay readable until nothing names them.
    pub fn register(
        &self,
        context: &mut FontContext,
        bytes: Arc<[u8]>,
        family: &str,
    ) -> Result<Vec<Arc<FontFaceId>>, Status> {
        if bytes.is_empty() || family.is_empty() {
            return Err(Status::BadScene);
        }
        let blob = Blob::new(Arc::new(Bytes(bytes)));
        let mut next = FontContext::new();
        let registered = register_family(&mut next, &blob, family);
        if registered.is_empty() {
            return Err(Status::BadScene);
        }
        // Fontique merges faces when they share a family. Rebuild the small
        // set of app registrations to replace the entire old family.
        let _registration = self
            .inner
            .registration
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut families = self.lock().families.clone();
        families.retain(|(name, _)| name != family);
        for (name, bytes) in &families {
            register_family(&mut next, bytes, name);
        }
        families.push((family.to_owned(), blob.clone()));
        *context = next;

        let mut library = self.lock();
        for face in library.faces.values_mut() {
            if face.family.as_deref() == Some(family) {
                face.retained = None;
                face.family = None;
            }
        }
        let ids = registered
            .into_iter()
            .flat_map(|(_, infos)| infos)
            .map(|info| library.insert(blob.clone(), info.index(), Some(family)))
            .collect();
        library.families = families;
        self.inner.revision.fetch_add(1, Ordering::Release);
        Ok(ids)
    }

    #[cfg(debug_assertions)]
    pub(crate) fn unregister(&self, context: &mut FontContext, family: &str) {
        let _registration = self
            .inner
            .registration
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut library = self.lock();
        library.families.retain(|(name, _)| name != family);
        let mut next = FontContext::new();
        for (name, blob) in &library.families {
            register_family(&mut next, blob, name);
        }
        for face in library
            .faces
            .values_mut()
            .filter(|face| face.family.as_deref() == Some(family))
        {
            face.retained = None;
            face.family = None;
        }
        *context = next;
        self.inner.revision.fetch_add(1, Ordering::Release);
    }

    /// Drops the bytes of faces nothing names anymore. Called by the raster
    /// between frames; the UI never needs this.
    pub fn sweep(&self) -> usize {
        let mut library = self.lock();
        let before = library.faces.len();
        let Library { faces, by_blob, .. } = &mut *library;
        faces.retain(|_, face| face.retained.is_some() || face.live.strong_count() != 0);
        by_blob.retain(|_, id| faces.contains_key(id));
        before - faces.len()
    }

    /// Live faces, for diagnostics.
    pub fn len(&self) -> usize {
        self.lock().faces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Family names, most recent registration first.
    pub fn families(&self) -> Vec<String> {
        self.lock()
            .families
            .iter()
            .rev()
            .map(|(name, _)| name.clone())
            .collect()
    }
}

impl Library {
    fn insert(&mut self, bytes: Blob<u8>, index: u32, family: Option<&str>) -> Arc<FontFaceId> {
        let id = Arc::new(FontFaceId(self.next_id));
        self.next_id = self.next_id.wrapping_add(1);
        self.by_blob.insert((BlobAddress::new(&bytes), index), *id);
        self.faces.insert(
            *id,
            Face {
                bytes,
                index,
                live: Arc::downgrade(&id),
                retained: family.map(|_| Arc::clone(&id)),
                family: family.map(str::to_owned),
            },
        );
        id
    }
}

fn register_family(
    context: &mut FontContext,
    blob: &Blob<u8>,
    family: &str,
) -> Vec<(FamilyId, Vec<fontique::FontInfo>)> {
    let registered = context.collection.register_fonts(
        blob.clone(),
        Some(FontInfoOverride {
            family_name: Some(family),
            width: None,
            style: None,
            weight: None,
            axes: None,
        }),
    );
    let generic = match family {
        "Sans" => Some(GenericFamily::SansSerif),
        "Serif" => Some(GenericFamily::Serif),
        "Monospace" => Some(GenericFamily::Monospace),
        _ => None,
    };
    if let Some(generic) = generic
        && !registered.is_empty()
    {
        context
            .collection
            .set_generic_families(generic, registered.iter().map(|(id, _)| *id));
    }
    registered
}

impl Default for FontLibrary {
    fn default() -> Self {
        Self::new()
    }
}

/// Owned font bytes behind Fontique's `AsRef<[u8]>` blob interface.
struct Bytes(Arc<[u8]>);

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

// Fontique can issue new handles for the same allocation. The library only
// compares addresses while holding the map lock and while both blobs are alive.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct BlobAddress {
    start: usize,
    len: usize,
}

impl BlobAddress {
    fn new(blob: &Blob<u8>) -> Self {
        Self {
            start: blob.data().as_ptr() as usize,
            len: blob.data().len(),
        }
    }
}

/// Cache and counters the UI shaper reports to the host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShapeStats {
    /// Shaping passes built in the current interval.
    pub built: u32,
    /// Hit and miss counts of the width-independent shaping cache.
    pub measure_hit: u32,
    pub measure_miss: u32,
    /// Hit and miss counts of the per-width frozen result.
    pub paint_hit: u32,
    pub paint_miss: u32,
    /// Individual entries evicted to enforce the cache budgets.
    pub evicted: u32,
    /// Entries discarded after changing the font mapping.
    pub registration_invalidated: u32,
    /// Shape misses grouped by UTF-8 length: up to 64, 256, 4096, and larger.
    pub size_buckets: [u32; 4],
}

impl ShapeStats {
    pub fn note_shape(&mut self, hit: bool) {
        if hit {
            self.measure_hit = self.measure_hit.saturating_add(1);
        } else {
            self.measure_miss = self.measure_miss.saturating_add(1);
            self.built = self.built.saturating_add(1);
        }
    }

    pub fn note_freeze(&mut self, hit: bool) {
        if hit {
            self.paint_hit = self.paint_hit.saturating_add(1);
        } else {
            self.paint_miss = self.paint_miss.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FontLibrary;
    use parley::FontContext;
    use std::sync::Arc;

    fn invalid() -> Arc<[u8]> {
        Arc::from(b"SECRET-FONT-BYTES-DO-NOT-LOG".as_slice())
    }

    #[test]
    fn rejected_bytes_do_not_bump_the_revision() {
        let library = FontLibrary::new();
        let mut context = FontContext::new();
        let error = library
            .register(&mut context, invalid(), "Custom")
            .unwrap_err();
        assert_eq!(error, crate::Status::BadScene);
        assert_eq!(library.revision(), 0);
        assert!(library.is_empty());
        assert!(library.families().is_empty());
    }

    #[test]
    fn same_family_replaces_the_mapping_and_keeps_old_faces_readable() {
        let bytes: Arc<[u8]> = Arc::from(crate::tests::ROBOTO);
        let library = FontLibrary::new();
        let mut context = FontContext::new();
        let first = library
            .register(&mut context, Arc::clone(&bytes), "Sans")
            .expect("Roboto registers");
        assert_eq!(library.revision(), 1);
        let second = library
            .register(&mut context, Arc::clone(&bytes), "Sans")
            .expect("Roboto registers again");
        assert_eq!(library.revision(), 2);
        assert_ne!(first[0], second[0]);
        let first_id = *first[0];
        let second_id = *second[0];
        assert!(library.with_face(&first_id, |bytes, _| !bytes.is_empty()) == Some(true));
        drop(first);
        drop(second);
        // A replaced registration is swept once nothing names it. The
        // current mapping keeps its bytes even when the caller dropped the
        // IDs, so a later frame can still resolve the family.
        assert_eq!(library.sweep(), 1);
        assert!(!library.has_face(&first_id));
        assert!(library.has_face(&second_id));
        assert_eq!(library.families(), ["Sans"]);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn a_ttc_registers_one_face_per_index() {
        let Ok(bytes) = std::fs::read("/System/Library/Fonts/Hiragino Sans GB.ttc") else {
            return;
        };
        let library = FontLibrary::new();
        let mut context = FontContext::new();
        let ids = library
            .register(&mut context, bytes.into(), "SansCJK")
            .expect("CJK registers");
        assert!(ids.len() >= 2, "TTC faces: {}", ids.len());
        let indexes: Vec<u32> = ids
            .iter()
            .map(|id| library.with_face(id, |_, index| index).expect("face"))
            .collect();
        assert!(indexes.windows(2).any(|pair| pair[0] != pair[1]));
    }
}
