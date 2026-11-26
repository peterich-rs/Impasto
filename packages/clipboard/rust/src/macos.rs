use objc2::rc::autoreleasepool;
use objc2_app_kit::NSPasteboard;
use objc2_foundation::NSString;

const TEXT_UTI: &str = "public.utf8-plain-text";

pub(super) fn get() -> String {
    autoreleasepool(|_| {
        NSPasteboard::generalPasteboard()
            .stringForType(&NSString::from_str(TEXT_UTI))
            .map_or_else(String::new, |text| text.to_string())
    })
}

pub(super) fn set(text: &str) {
    autoreleasepool(|_| {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        pasteboard.setString_forType(&NSString::from_str(text), &NSString::from_str(TEXT_UTI));
    });
}
