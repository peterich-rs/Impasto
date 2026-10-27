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

    fn document_dir(&self) -> Result<String, String> {
        Err("module \"FileSystem\" is not linked".to_owned())
    }

    fn pick_document(&self) -> Result<Option<String>, String> {
        Ok(None)
    }

    /// `granted`, `denied`, `restricted`, `permanentlyDenied`, `limited`, or `provisional`.
    fn permission_status(&self, name: &str) -> String {
        let _ = name;
        "denied".to_owned()
    }

    fn request_permissions(&self, names: &[String]) -> Vec<(String, String)> {
        names
            .iter()
            .map(|name| (name.clone(), self.permission_status(name)))
            .collect()
    }

    fn permission_rationale(&self, name: &str) -> bool {
        let _ = name;
        false
    }

    /// `enabled`, `disabled`, or `notApplicable`.
    fn service_status(&self, name: &str) -> String {
        let _ = name;
        "notApplicable".to_owned()
    }

    fn open_app_settings(&self) -> bool {
        false
    }
}

/// Bridge used when a host has not installed clipboard or URL opening.
pub struct EmptyBridge;

impl PlatformBridge for EmptyBridge {}
