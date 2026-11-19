//! QuickJS runtime shared by a headless host and the Impasto UI adapter.
//!
//! The crate depends on `rquickjs` and `js-host`. It does not depend on a UI
//! session, a render tree, or a reactive graph. Engine-specific seams live in
//! [`quickjs`].

#![allow(
    unsafe_code,
    reason = "QuickJS interrupt and clock FFI stay inside this crate and are documented at each call"
)]

pub mod bytecode;
pub mod compile;
pub mod deliver;
pub mod host;
pub mod modules;
pub mod quickjs;
mod runtime;
pub mod standard;

pub use compile::{CompileEnvironment, EmptyCompile};
pub use js_host::IoEvent;
pub use modules::NativeFactory;
pub use quickjs::{JobStop, JsDiagnostic, Timer, Vm, poll_timer, run_jobs};
pub use runtime::{
    DriveReport, JsError, ModuleStatus, ModuleTicket, Runtime, RuntimeBuilder, RuntimeCounts,
    WeakRuntime,
};

/// The one legacy prelude. UI startup and a headless [`Runtime`] both evaluate it.
pub const LEGACY_PRELUDE: &str = include_str!("standard/legacy-prelude.js");

/// Milliseconds from a monotonic clock. Tests must not freeze this with a manual clock.
pub fn monotonic_ms() -> i64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `clock_gettime` writes the local timespec and keeps no pointer.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return 0;
    }
    time.tv_sec
        .saturating_mul(1000)
        .saturating_add(time.tv_nsec / 1_000_000)
}
