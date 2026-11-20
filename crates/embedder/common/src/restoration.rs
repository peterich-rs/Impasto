use std::collections::BTreeMap;

use session::{Shared, shared};

const MAX_BYTES: usize = 32 * 1024;
const MAX_KEY_BYTES: usize = 128;

/// Small UI state owned by one platform task. The render thread updates the
/// cache and the platform copies its serialized form when saving instance state.
#[derive(Clone)]
pub struct Restoration(Shared<Store>);

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
        Self(shared(Store {
            committed: snapshot,
            candidate: None,
        }))
    }

    pub fn saved_state(&self) -> Option<String> {
        self.0
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .committed
            .encoded
            .clone()
    }

    pub fn has_entries(&self) -> bool {
        !self
            .0
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .current()
            .entries
            .is_empty()
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.0
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
        let mut cache = self.0.write().unwrap_or_else(|error| error.into_inner());
        let snapshot = match &mut *cache {
            Store {
                candidate: Some((owner, snapshot)),
                ..
            } if *owner == std::thread::current().id() => snapshot,
            store => &mut store.committed,
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
        Ok(())
    }
}

/// Stages restoration writes during evaluation on the render thread.
pub(crate) struct RestorationUpdate<'a>(&'a Restoration);

impl Restoration {
    pub(crate) fn stage_update(&self) -> RestorationUpdate<'_> {
        let mut store = self.0.write().unwrap_or_else(|error| error.into_inner());
        store.candidate = Some((std::thread::current().id(), store.committed.clone()));
        RestorationUpdate(self)
    }
}

impl RestorationUpdate<'_> {
    pub(crate) fn commit(self) {
        let mut store = self.0.0.write().unwrap_or_else(|error| error.into_inner());
        if let Some((_, candidate)) = store.candidate.take() {
            store.committed = candidate;
        }
    }
}

impl Drop for RestorationUpdate<'_> {
    fn drop(&mut self) {
        self.0
            .0
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
