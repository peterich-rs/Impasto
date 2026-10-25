//! Android host. The platform thread is the Java UI thread. The render thread owns the engine.

mod a11y;
#[allow(dead_code)]
mod assets;

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
mod insets;
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
mod motion;
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
mod textinput;

#[cfg(target_os = "android")]
mod decode;
#[cfg(target_os = "android")]
mod host;
#[cfg(target_os = "android")]
mod surface;
#[cfg(target_os = "android")]
mod vsync;
