use std::ffi::c_void;

/// A platform object the render thread may use.
///
/// The platform thread keeps the object alive until it has observed
/// [`crate::Link::destroy_surface_and_wait`] return. Only the render thread
/// dereferences the pointer.
#[derive(Clone, Copy, Debug)]
pub struct NativePtr(pub *mut c_void);

#[allow(
    unsafe_code,
    reason = "Send is the pointer contract: only the render thread dereferences it"
)]
// SAFETY: the pointer is not dereferenced by this impl. Only the render thread
// reads the object, and the platform thread keeps it alive until destroy returns.
unsafe impl Send for NativePtr {}

#[allow(
    unsafe_code,
    reason = "Sync is the pointer contract: the platform thread only copies the address"
)]
// SAFETY: the platform thread only copies the address into a command. It does
// not dereference the pointer while the render thread does.
unsafe impl Sync for NativePtr {}
