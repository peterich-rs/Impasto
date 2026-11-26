use objc2::runtime::AnyClass;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSString, NSURL};

use super::PermissionStatus;

pub(super) fn status(name: &str) -> PermissionStatus {
    let kind = match name {
        "camera" => "vide",
        "microphone" => "soun",
        _ => return PermissionStatus::Denied,
    };
    let Some(device) = AnyClass::get(c"AVCaptureDevice") else {
        return PermissionStatus::Denied;
    };
    let media = NSString::from_str(kind);
    // SAFETY: AVCaptureDevice takes an AVMediaType NSString and returns an NSInteger.
    #[expect(unsafe_code, reason = "AVFoundation media authorization selector")]
    let status: isize =
        unsafe { objc2::msg_send![device, authorizationStatusForMediaType: &*media] };
    match status {
        3 => PermissionStatus::Granted,
        1 => PermissionStatus::Restricted,
        _ => PermissionStatus::Denied,
    }
}

pub(super) fn settings() -> bool {
    let url = NSString::from_str("x-apple.systempreferences:com.apple.preference.security?Privacy");
    NSURL::URLWithString(&url).is_some_and(|url| NSWorkspace::sharedWorkspace().openURL(&url))
}
