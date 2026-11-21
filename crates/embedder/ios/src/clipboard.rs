//! UIKit implementation of the synchronous clipboard module.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use dispatch2::DispatchQueue;
use embedder::ClipboardHost;
use objc2::MainThreadMarker;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::NSString;

const RESPONSE_TIMEOUT: Duration = Duration::from_millis(250);

pub(super) struct Clipboard;

impl ClipboardHost for Clipboard {
    fn read_clipboard(&self) -> String {
        on_main(read).unwrap_or_default()
    }

    fn write_clipboard(&self, text: &str) {
        let text = text.to_owned();
        on_main(move |main| write(main, &text));
    }
}

fn on_main<T: Send + 'static>(
    work: impl FnOnce(MainThreadMarker) -> T + Send + 'static,
) -> Option<T> {
    if let Some(main) = MainThreadMarker::new() {
        return Some(autoreleasepool(|_| work(main)));
    }

    // The JS API is synchronous, but system paste approval can take an
    // arbitrary amount of time. Expired requests that have not started are
    // skipped, and any response from already running work is discarded.
    let deadline = Instant::now() + RESPONSE_TIMEOUT;
    let (send, receive) = mpsc::sync_channel(1);
    DispatchQueue::main().exec_async(move || {
        if Instant::now() < deadline
            && let Some(main) = MainThreadMarker::new()
        {
            let _ = send.send(autoreleasepool(|_| work(main)));
        }
    });
    receive
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .ok()
}

#[allow(
    unsafe_code,
    reason = "UIKit pasteboard selectors are called on the main thread"
)]
fn general_pasteboard(_main: MainThreadMarker) -> Option<Retained<AnyObject>> {
    let class = AnyClass::get(c"UIPasteboard")?;
    // SAFETY: this class defines +generalPasteboard, which returns the UIKit pasteboard.
    unsafe { objc2::msg_send![class, generalPasteboard] }
}

#[allow(
    unsafe_code,
    reason = "UIKit pasteboard selectors are called on the main thread"
)]
fn read(main: MainThreadMarker) -> String {
    let Some(pasteboard) = general_pasteboard(main) else {
        return String::new();
    };
    // SAFETY: UIPasteboard.hasStrings returns BOOL.
    let has_strings: bool = unsafe { objc2::msg_send![&*pasteboard, hasStrings] };
    if !has_strings {
        return String::new();
    }
    // SAFETY: UIPasteboard.string returns an NSString or nil.
    let text: Option<Retained<NSString>> = unsafe { objc2::msg_send![&*pasteboard, string] };
    text.map_or_else(String::new, |text| text.to_string())
}

#[allow(
    unsafe_code,
    reason = "UIKit pasteboard selectors are called on the main thread"
)]
fn write(main: MainThreadMarker, text: &str) {
    if let Some(pasteboard) = general_pasteboard(main) {
        let text = NSString::from_str(text);
        // SAFETY: UIPasteboard.setString: accepts and copies this NSString.
        let _: () = unsafe { objc2::msg_send![&*pasteboard, setString: &*text] };
    }
}
