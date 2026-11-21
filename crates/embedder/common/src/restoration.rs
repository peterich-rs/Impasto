use std::collections::BTreeMap;
use std::sync::Arc;

use session::{Shared, shared};

const MAX_BYTES: usize = 32 * 1024;
const MAX_KEY_BYTES: usize = 128;

/// Small UI state owned by one platform task. The render thread updates the
/// cache and the platform copies its serialized form when saving instance state.
#[derive(Clone)]
pub struct Restoration {
    store: Shared<Store>,
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
}

struct Store {
    committed: Snapshot,
    candidate: Option<(std::thread::ThreadId, Snapshot)>,
}

impl Store {
    fn current(&self) -> &Snapshot {
        match &self.candidate {
            Some((owner, snapshot)) if *owner == std::thread::current().id() => snapshot,
            _ => &self.committed,
        }
    }
}

#[derive(Clone, Default)]
struct Snapshot {
    entries: BTreeMap<String, String>,
    encoded: Option<String>,
}

impl Restoration {
    pub fn new(saved: Option<String>) -> Self {
        let snapshot = saved
            .filter(|text| text.len() <= MAX_BYTES)
            .and_then(|encoded| {
                let payload = encoded.strip_prefix("1\n")?;
                let entries: BTreeMap<String, String> = serde_json::from_str(payload).ok()?;
                if entries.keys().any(|key| !valid_key(key)) {
                    return None;
                }
                Some(Snapshot {
                    entries,
                    encoded: Some(encoded),
                })
            })
            .unwrap_or_default();
        Self {
            store: shared(Store {
                committed: snapshot,
                candidate: None,
            }),
            notify: None,
        }
    }

    /// Notifies after a committed change, outside the snapshot lock.
    pub fn new_notifying(saved: Option<String>, notify: impl Fn() + Send + Sync + 'static) -> Self {
        let mut restoration = Self::new(saved);
        restoration.notify = Some(Arc::new(notify));
        restoration
    }

    fn notify_changed(&self) {
        if let Some(notify) = &self.notify {
            notify();
        }
    }

    pub fn saved_state(&self) -> Option<String> {
        self.store
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .committed
            .encoded
            .clone()
    }

    pub fn has_entries(&self) -> bool {
        !self
            .store
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .current()
            .entries
            .is_empty()
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.store
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .current()
            .entries
            .get(key)
            .cloned()
    }

    pub fn set(&self, key: &str, value: Option<&str>) -> Result<(), String> {
        if !valid_key(key) {
            return Err("invalid restoration key".into());
        }
        if value.is_some_and(|text| text.len() > MAX_BYTES) {
            return Err("restoration snapshot exceeds 32 KiB".into());
        }
        let mut cache = self
            .store
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let (snapshot, committed) = match &mut *cache {
            Store {
                candidate: Some((owner, snapshot)),
                ..
            } if *owner == std::thread::current().id() => (snapshot, false),
            store => (&mut store.committed, true),
        };
        if snapshot.entries.get(key).map(String::as_str) == value {
            return Ok(());
        }
        let mut entries = snapshot.entries.clone();
        match value {
            Some(value) => entries.insert(key.into(), value.into()),
            None => entries.remove(key),
        };
        let json = serde_json::to_string(&entries).map_err(|error| error.to_string())?;
        let encoded = format!("1\n{json}");
        if encoded.len() > MAX_BYTES {
            return Err("restoration snapshot exceeds 32 KiB".into());
        }
        *snapshot = Snapshot {
            entries,
            encoded: Some(encoded),
        };
        drop(cache);
        if committed {
            self.notify_changed();
        }
        Ok(())
    }
}

/// Stages restoration writes during evaluation on the render thread.
pub(crate) struct RestorationUpdate<'a>(&'a Restoration);

impl Restoration {
    pub(crate) fn stage_update(&self) -> RestorationUpdate<'_> {
        let mut store = self
            .store
            .write()
            .unwrap_or_else(|error| error.into_inner());
        store.candidate = Some((std::thread::current().id(), store.committed.clone()));
        RestorationUpdate(self)
    }
}

impl RestorationUpdate<'_> {
    pub(crate) fn commit(self) {
        let mut store = self
            .0
            .store
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let changed = store.candidate.take().is_some_and(|(_, candidate)| {
            if store.committed.encoded == candidate.encoded {
                return false;
            }
            store.committed = candidate;
            true
        });
        drop(store);
        if changed {
            self.0.notify_changed();
        }
    }
}

impl Drop for RestorationUpdate<'_> {
    fn drop(&mut self) {
        self.0
            .store
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .candidate = None;
    }
}

fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= MAX_KEY_BYTES && !key.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::{MAX_BYTES, Restoration};
    use std::sync::mpsc;

    #[test]
    fn task_snapshots_round_trip_and_reject_oversized_updates() -> Result<(), String> {
        let task = Restoration::new(None);
        task.set("routes", Some("[1,\"/detail\"]"))?;
        assert!(task.set("form", Some(&"🎨".repeat(MAX_BYTES / 4))).is_err());
        let restored = Restoration::new(task.saved_state());
        assert_eq!(restored.get("routes").as_deref(), Some("[1,\"/detail\"]"));
        assert!(restored.get("form").is_none());
        assert!(Restoration::new(None).get("routes").is_none());
        restored.set("routes", None)?;
        assert!(
            Restoration::new(restored.saved_state())
                .get("routes")
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn restoration_notifies_only_after_changes_commit() -> Result<(), String> {
        let (send, receive) = mpsc::channel();
        let state = Restoration::new_notifying(None, move || {
            let _ = send.send(());
        });
        state.set("routes", Some("/first"))?;
        assert!(receive.try_recv().is_ok());

        let candidate = state.stage_update();
        state.set("routes", Some("/second"))?;
        assert!(receive.try_recv().is_err());
        drop(candidate);
        assert_eq!(state.get("routes").as_deref(), Some("/first"));
        assert!(receive.try_recv().is_err());

        let candidate = state.stage_update();
        state.set("routes", Some("/second"))?;
        candidate.commit();
        assert!(receive.try_recv().is_ok());
        assert_eq!(state.get("routes").as_deref(), Some("/second"));

        let candidate = state.stage_update();
        state.set("routes", Some("/second"))?;
        candidate.commit();
        assert!(receive.try_recv().is_err());
        Ok(())
    }

    #[test]
    fn invalid_or_unknown_snapshots_start_with_empty_state() {
        for snapshot in ["1\n{malformed", "2\n{\"key\":\"value\"}", "1\n{\"key\":42}"] {
            assert!(
                Restoration::new(Some(snapshot.into()))
                    .saved_state()
                    .is_none()
            );
        }
    }
}
