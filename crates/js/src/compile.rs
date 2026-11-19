//! Compile-time module declarations supplied by the host.
//!
//! The UI adapter installs scene and reactive names. A headless compile uses
//! [`EmptyCompile`] and does not see those modules.

use rquickjs::Runtime;

pub trait CompileEnvironment {
    fn prepare(&self, runtime: &Runtime);
}

/// No framework modules. Bare imports of scene or reactive fail at compile time.
pub struct EmptyCompile;

impl CompileEnvironment for EmptyCompile {
    fn prepare(&self, _runtime: &Runtime) {}
}
