//! Rust side of the UIKit host. Built as a static library for the iOS app.
#![cfg(target_os = "ios")]

#[allow(
    unsafe_code,
    reason = "C entry points and Objective-C object ownership"
)]
mod bridge;
mod bundle;
#[allow(
    unsafe_code,
    reason = "Metal drawables and the CAMetalLayer ownership lease"
)]
mod surface;
