use std::ffi::{CString, c_void};
use std::sync::atomic::{AtomicU32, Ordering};

/// Font bytes Impeller may still read after `RegisterFont` returns.
///
/// The owner holds one reference. Impeller holds the other until it invokes
/// [`release_font_lease`]. The allocation is freed when the count hits zero,
/// on whichever thread drops the last reference.
pub(super) struct FontLease {
    bytes: Vec<u8>,
    refs: AtomicU32,
}

impl FontLease {
    /// Two references: the caller, and the Impeller mapping callback.
    pub(super) fn new(bytes: Vec<u8>) -> *mut Self {
        Box::into_raw(Box::new(Self {
            bytes,
            refs: AtomicU32::new(2),
        }))
    }

    pub(super) fn data(ptr: *mut Self) -> *const u8 {
        // SAFETY: `ptr` is a live lease. The caller still holds a reference.
        unsafe { (*ptr).bytes.as_ptr() }
    }

    pub(super) fn len(ptr: *mut Self) -> u64 {
        // SAFETY: `ptr` is a live lease. The caller still holds a reference.
        unsafe { (*ptr).bytes.len() as u64 }
    }

    pub(super) fn same(ptr: *mut Self, data: &[u8]) -> bool {
        if ptr.is_null() {
            return false;
        }
        // SAFETY: `ptr` is a live lease held by a registered face.
        unsafe { (*ptr).bytes.as_slice() == data }
    }

    pub(super) fn release(ptr: *mut Self) {
        if ptr.is_null() {
            return;
        }
        // SAFETY: `ptr` came from `Box::into_raw`. Exactly one caller frees it,
        // the one that observes the count fall to zero.
        unsafe {
            let previous = (*ptr).refs.fetch_sub(1, Ordering::AcqRel);
            if previous == 1 {
                drop(Box::from_raw(ptr));
            }
        }
    }
}

pub(super) struct Face {
    pub(super) lease: *mut FontLease,
    pub(super) family: CString,
}

impl Drop for Face {
    fn drop(&mut self) {
        FontLease::release(self.lease);
        self.lease = std::ptr::null_mut();
    }
}

pub(crate) struct FontRegistry {
    pub(super) faces: Vec<Face>,
    pub(super) default_family: Option<CString>,
}

impl FontRegistry {
    pub(super) fn new() -> Self {
        Self {
            faces: Vec::new(),
            default_family: None,
        }
    }
}

/// Impeller mapping callback. Drops one lease reference. Does not free the
/// bytes while the face still holds the other reference.
pub(crate) unsafe extern "C" fn release_font_lease(user_data: *mut c_void) {
    FontLease::release(user_data.cast());
}

#[cfg(test)]
mod tests {
    use super::FontLease;

    #[test]
    fn the_last_release_frees_the_lease() {
        let lease = FontLease::new(b"face".to_vec());
        assert_eq!(
            unsafe { (*lease).refs.load(std::sync::atomic::Ordering::Acquire) },
            2
        );
        FontLease::release(lease);
        assert_eq!(
            unsafe { (*lease).refs.load(std::sync::atomic::Ordering::Acquire) },
            1
        );
        FontLease::release(lease);
    }
}
