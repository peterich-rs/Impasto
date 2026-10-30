//! PluginValue stack built from JS host calls.

use session::{Invoke, PluginValue};

pub(super) struct ArgFrame {
    pub(super) map: bool,
    pub(super) items: Vec<PluginValue>,
    pub(super) keys: Vec<String>,
    pub(super) pending_key: Option<String>,
}

pub(super) struct ArgBuild {
    pub(super) stack: Vec<PluginValue>,
    pub(super) frames: Vec<ArgFrame>,
}

impl ArgBuild {
    pub(super) fn new() -> Self {
        Self {
            stack: Vec::new(),
            frames: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, value: PluginValue) {
        if let Some(frame) = self.frames.last_mut() {
            if frame.map {
                let key = frame.pending_key.take().unwrap_or_default();
                frame.keys.push(key);
            }
            frame.items.push(value);
            return;
        }
        self.stack.push(value);
    }

    pub(super) fn begin(&mut self, map: bool) {
        self.frames.push(ArgFrame {
            map,
            items: Vec::new(),
            keys: Vec::new(),
            pending_key: None,
        });
    }

    pub(super) fn end(&mut self) {
        let Some(frame) = self.frames.pop() else {
            return;
        };
        let value = if frame.map {
            PluginValue::Map(frame.keys.into_iter().zip(frame.items).collect())
        } else {
            PluginValue::List(frame.items)
        };
        self.push(value);
    }
}

pub(super) struct PreparedCall {
    pub(super) invoke: Invoke,
    pub(super) token: u64,
    pub(super) args: ArgBuild,
}
