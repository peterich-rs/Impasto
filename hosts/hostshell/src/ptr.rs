use std::ffi::c_void;

/// A platform object the render thread may use.
///
/// The platform thread keeps the object alive until it has observed
/// [`crate::Link::destroy_surface_and_wait`] return. Only the render thread
/// dereferences the pointer.
#[derive(Clone, Copy, Debug)]
pub struct NativePtr(pub *mut c_void);

// SAFETY: see the type's contract. The pointer is not dereferenced by this impl.
#[allow(unsafe_code)]
unsafe impl Send for NativePtr {}

// SAFETY: the platform and render threads do not both dereference it. The
// platform thread only copies the address into a command.
#[allow(unsafe_code)]
unsafe impl Sync for NativePtr {}
