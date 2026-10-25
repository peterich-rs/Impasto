//! Clipboard and URL opening. HTTP and sockets stay in the shell's network thread.

/// Platform work that must touch the window or the system pasteboard.
pub trait PlatformBridge: Send + Sync {
    fn read_clipboard(&self) -> String {
        String::new()
    }

    fn write_clipboard(&self, text: &str) {
        let _ = text;
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        let _ = url;
        Err("openURL is not linked".to_owned())
    }
}

/// Bridge used when a host has not installed clipboard or URL opening.
pub struct EmptyBridge;

impl PlatformBridge for EmptyBridge {}
