//! Script assets and compatibility payloads used during UI startup.

pub(super) const ABI_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/abi.js"));
pub(super) const SCENE_JS: &str = include_str!("../scene.js");

pub(super) const REACTIVE_BOOT: &str = include_str!("../reactive.js");

pub(super) fn app_state(name: &str) -> &'static str {
    match name {
        "resumed" => "active",
        "inactive" => "inactive",
        "hidden" | "paused" | "detached" => "background",
        _ => "unknown",
    }
}
