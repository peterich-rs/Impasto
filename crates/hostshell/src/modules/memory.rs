//! In-memory hosts for hostshell tests without a platform.

use std::sync::{Arc, Mutex};

use super::{ClipboardHost, LinkingHost, PermissionsHost, StorageHost};

pub struct MemoryClipboard {
    text: Mutex<String>,
}

impl MemoryClipboard {
    pub fn new(text: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            text: Mutex::new(text.into()),
        })
    }
}

impl ClipboardHost for MemoryClipboard {
    fn read_clipboard(&self) -> String {
        self.text
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn write_clipboard(&self, text: &str) {
        *self
            .text
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = text.to_owned();
    }
}

pub struct MemoryStorage {
    dir: String,
    pick: Mutex<Option<String>>,
}

impl MemoryStorage {
    pub fn new(dir: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            dir: dir.into(),
            pick: Mutex::new(None),
        })
    }

    pub fn set_pick(&self, name: Option<String>) {
        *self
            .pick
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = name;
    }
}

impl StorageHost for MemoryStorage {
    fn document_dir(&self) -> Result<String, String> {
        Ok(self.dir.clone())
    }

    fn pick_document(&self) -> Result<Option<String>, String> {
        Ok(self
            .pick
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone())
    }
}

pub struct DeniedPermissions;

impl PermissionsHost for DeniedPermissions {
    fn permission_status(&self, _name: &str) -> String {
        "denied".to_owned()
    }

    fn request_permissions(&self, names: &[String]) -> Vec<(String, String)> {
        names
            .iter()
            .map(|name| (name.clone(), "denied".to_owned()))
            .collect()
    }

    fn permission_rationale(&self, _name: &str) -> bool {
        false
    }

    fn service_status(&self, _name: &str) -> String {
        "notApplicable".to_owned()
    }

    fn open_app_settings(&self) -> bool {
        false
    }
}

pub struct NullLinking;

impl LinkingHost for NullLinking {
    fn open_url(&self, _url: &str) -> Result<(), String> {
        Err("openURL is not linked".to_owned())
    }
}
