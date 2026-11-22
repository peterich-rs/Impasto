//! Raw Impeller C API declarations for the selected Rust target.

#[cfg(feature = "link")]
#[cfg_attr(target_env = "msvc", path = "bindings_msvc.rs")]
#[allow(
    unsafe_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::all,
    reason = "generated declarations mirror the Impeller C header"
)]
mod bindings;

#[cfg(feature = "link")]
pub use bindings::*;

#[cfg(all(feature = "link", target_os = "ios"))]
#[allow(unsafe_code, reason = "standalone iOS Metal interoperability ABI")]
unsafe extern "C" {
    pub fn ImpastoMetalWaitUntilScheduled(context: ImpellerContext) -> bool;
    pub fn ImpastoMetalTextureNew(
        context: ImpellerContext,
        texture: *const core::ffi::c_void,
        release: Option<extern "C" fn(*mut core::ffi::c_void)>,
        user_data: *mut core::ffi::c_void,
    ) -> ImpellerTexture;
}

#[cfg(all(test, feature = "link"))]
mod tests {
    #[test]
    #[expect(unsafe_code, reason = "checks the native SDK version at the C ABI")]
    fn native_version_matches_bindings() {
        // SAFETY: The function accepts no arguments and requires no initialized context.
        let version = unsafe { super::ImpellerGetVersion() };
        assert_eq!(version, super::IMPELLER_VERSION);
    }
}
