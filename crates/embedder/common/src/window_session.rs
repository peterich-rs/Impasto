//! App data shared explicitly by independent windows in one application session.

use std::sync::{Arc, Mutex, Weak};

use crate::{Command, Poster, Restoration};

#[derive(Clone)]
pub struct WindowSession {
    values: Restoration,
    windows: Arc<Mutex<Vec<Weak<Poster>>>>,
}

impl Default for WindowSession {
    fn default() -> Self {
        Self {
            values: Restoration::new(None),
            windows: Arc::default(),
        }
    }
}

impl WindowSession {
    pub fn connect(&self, window: &Arc<Poster>) {
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        windows.retain(|window| window.strong_count() != 0);
        windows.push(Arc::downgrade(window));
    }

    pub(crate) fn get(&self, key: &str) -> Option<String> {
        self.values.get(key)
    }

    pub(crate) fn set(&self, key: &str, value: Option<&str>) -> Result<(), String> {
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let previous = self.values.get(key);
        self.values.set(key, value)?;
        if previous.as_deref() == value {
            return Ok(());
        }
        windows.retain(|window| {
            let Some(window) = window.upgrade() else {
                return false;
            };
            window.post(Command::WindowStateChanged(key.to_owned()));
            true
        });
        Ok(())
    }
}
