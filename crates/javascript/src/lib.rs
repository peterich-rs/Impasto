//! JavaScript execution, compilation, and host integration.

pub mod host;

#[cfg(feature = "runtime")]
#[allow(unsafe_code, reason = "QuickJS compilation uses the native engine API")]
pub mod compiler;

#[cfg(feature = "runtime")]
#[allow(
    unsafe_code,
    reason = "QuickJS runtime and clock operations use the native API"
)]
pub mod runtime;

#[cfg(feature = "runtime")]
#[allow(
    unsafe_code,
    reason = "standard bindings install native functions into QuickJS"
)]
pub mod standard;
