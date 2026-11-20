//! Frame pipeline slices for Android system traces.

use std::ffi::CStr;

#[must_use]
pub struct Section {
    #[cfg(target_os = "android")]
    enabled: bool,
    // Android requires each section to finish on its calling thread.
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Section {
    pub fn enabled() -> bool {
        #[cfg(target_os = "android")]
        #[allow(unsafe_code, reason = "Android platform trace API")]
        {
            // SAFETY: no arguments or thread affinity for the enabled query.
            unsafe { ndk_sys::ATrace_isEnabled() }
        }
        #[cfg(not(target_os = "android"))]
        false
    }

    pub fn event(name: impl FnOnce() -> String) {
        if Self::enabled()
            && let Ok(name) = std::ffi::CString::new(name())
        {
            let _event = Self::begin(&name);
        }
    }

    pub fn begin(_name: &CStr) -> Self {
        #[cfg(target_os = "android")]
        #[allow(unsafe_code, reason = "Android platform trace API")]
        let enabled = unsafe {
            // SAFETY: The NDK trace API is available since API 23. The
            // section name is a null-terminated string and outlives the call.
            let enabled = ndk_sys::ATrace_isEnabled();
            if enabled {
                ndk_sys::ATrace_beginSection(_name.as_ptr());
            }
            enabled
        };
        Self {
            #[cfg(target_os = "android")]
            enabled,
            _thread: std::marker::PhantomData,
        }
    }
}

#[cfg(target_os = "android")]
impl Drop for Section {
    #[allow(unsafe_code, reason = "Android platform trace API")]
    fn drop(&mut self) {
        if self.enabled {
            // SAFETY: begin opened a section on this thread. The guard is !Send.
            unsafe { ndk_sys::ATrace_endSection() };
        }
    }
}

#[cfg(not(target_os = "android"))]
impl Drop for Section {
    fn drop(&mut self) {}
}
