//! Android host. The platform thread is `android_main`. The render thread owns the engine.

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

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    let code = host::run(app);
    std::process::exit(code);
}
