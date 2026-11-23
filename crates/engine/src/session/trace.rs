//! Frame pipeline slices for system traces.

use std::ffi::CStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

struct Hooks {
    enabled: fn() -> bool,
    begin: fn(&CStr) -> u64,
    finish: fn(u64),
}

static HOOKS: OnceLock<Hooks> = OnceLock::new();
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Install the native bridge once per process. A zero token indicates the
/// system profiler is not collecting the section.
pub fn install(enabled: fn() -> bool, begin: fn(&CStr) -> u64, finish: fn(u64)) {
    if HOOKS
        .set(Hooks {
            enabled,
            begin,
            finish,
        })
        .is_ok()
    {
        INSTALLED.store(true, Ordering::Release);
    }
}

fn hooks() -> Option<&'static Hooks> {
    if INSTALLED.load(Ordering::Relaxed) {
        HOOKS.get()
    } else {
        None
    }
}

#[must_use]
pub struct Section {
    native: u64,
    #[cfg(all(feature = "android-trace", target_os = "android"))]
    enabled: bool,
    // Android requires each section to finish on its calling thread.
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Section {
    pub fn enabled() -> bool {
        if hooks().is_some_and(|hooks| (hooks.enabled)()) {
            return true;
        }
        #[cfg(all(feature = "android-trace", target_os = "android"))]
        #[allow(unsafe_code, reason = "Android platform trace API")]
        {
            // SAFETY: no arguments or thread affinity for the enabled query.
            unsafe { ndk_sys::ATrace_isEnabled() }
        }
        #[cfg(not(all(feature = "android-trace", target_os = "android")))]
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
        #[cfg(all(feature = "android-trace", target_os = "android"))]
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
            native: hooks().map_or(0, |hooks| (hooks.begin)(_name)),
            #[cfg(all(feature = "android-trace", target_os = "android"))]
            enabled,
            _thread: std::marker::PhantomData,
        }
    }
}

impl Drop for Section {
    fn drop(&mut self) {
        if self.native != 0
            && let Some(hooks) = hooks()
        {
            (hooks.finish)(self.native);
        }
        #[cfg(all(feature = "android-trace", target_os = "android"))]
        #[allow(unsafe_code, reason = "Android platform trace API")]
        if self.enabled {
            // SAFETY: begin opened a section on this thread. The guard is !Send.
            unsafe { ndk_sys::ATrace_endSection() };
        }
    }
}
