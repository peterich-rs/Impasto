//! Document-directory files and the string map behind `AsyncStorage`.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use engine::session::document_relative;

const STORE_NAME: &str = "async-storage.tsv";

// All windows in a process can use the same application storage directory.
static STORE_LOCK: Mutex<()> = Mutex::new(());
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

pub fn read_document(root: &Path, path: &str) -> Result<Vec<u8>, String> {
    let full = joined(root, path).map_err(|error| io_fail("read", path, error))?;
    fs::read(&full).map_err(|error| io_fail("read", path, error.to_string()))
}

pub fn write_document(root: &Path, path: &str, bytes: &[u8]) -> Result<(), String> {
    let full = joined(root, path).map_err(|error| io_fail("write", path, error))?;
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).map_err(|error| io_fail("write", path, error.to_string()))?;
    }
    fs::write(&full, bytes).map_err(|error| io_fail("write", path, error.to_string()))
}

pub fn delete_document(root: &Path, path: &str) -> Result<(), String> {
    let full = joined(root, path).map_err(|error| io_fail("delete", path, error))?;
    if full.exists() {
        fs::remove_file(&full).map_err(|error| io_fail("delete", path, error.to_string()))?;
    }
    Ok(())
}

pub fn store_get(root: &Path, keys: &[String]) -> Result<Vec<(String, Option<String>)>, String> {
    let map = {
        let _guard = lock_store();
        load_store(root)?
    };
    Ok(keys
        .iter()
        .map(|key| {
            let value = map.get(key).cloned();
            (key.clone(), value)
        })
        .collect())
}

pub fn store_set(root: &Path, rows: &[(String, String)]) -> Result<(), String> {
    let _guard = lock_store();
    let mut map = load_store(root).map_err(|error| io_fail("store-set", STORE_NAME, error))?;
    for (key, value) in rows {
        map.insert(key.clone(), value.clone());
    }
    save_store(root, &map).map_err(|error| io_fail("store-set", STORE_NAME, error))
}

pub fn store_remove(root: &Path, keys: &[String]) -> Result<(), String> {
    let _guard = lock_store();
    let mut map = load_store(root).map_err(|error| io_fail("store-remove", STORE_NAME, error))?;
    for key in keys {
        map.remove(key);
    }
    save_store(root, &map).map_err(|error| io_fail("store-remove", STORE_NAME, error))
}

pub fn store_clear(root: &Path) -> Result<(), String> {
    let _guard = lock_store();
    match fs::remove_file(root.join(STORE_NAME)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_fail("store-clear", STORE_NAME, error.to_string())),
    }
}

fn lock_store() -> MutexGuard<'static, ()> {
    // A previous panic leaves either the complete original file or its complete replacement.
    STORE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn io_fail(op: &'static str, path: &str, error: String) -> String {
    tracing::warn!(target: "impasto::io", op, path, error = %error, "file io failed");
    error
}

fn joined(root: &Path, path: &str) -> Result<PathBuf, String> {
    let relative = document_relative(path)?;
    Ok(root.join(relative))
}

fn load_store(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let text = match fs::read_to_string(root.join(STORE_NAME)) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut map = BTreeMap::new();
    for line in text.split('\n') {
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('\t') else {
            continue;
        };
        map.insert(unescape(key), unescape(value));
    }
    Ok(map)
}

fn save_store(root: &Path, map: &BTreeMap<String, String>) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let mut text = String::new();
    for (key, value) in map {
        text.push_str(&escape(key));
        text.push('\t');
        text.push_str(&escape(value));
        text.push('\n');
    }
    replace_store(root, text.as_bytes()).map_err(|error| error.to_string())
}

fn replace_store(root: &Path, value: &[u8]) -> io::Result<()> {
    let (mut file, path) = loop {
        let serial = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!(".{STORE_NAME}.{}.{serial}.tmp", std::process::id()));
        match File::create_new(&path) {
            Ok(file) => break (file, path),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let saved = file.write_all(value).and_then(|()| file.sync_all());
    drop(file);
    let result = saved.and_then(|()| fs::rename(&path, root.join(STORE_NAME)));
    if result.is_err() {
        let _ = fs::remove_file(&path);
    }
    result
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{STORE_NAME, store_get, store_set};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Barrier;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDirectory(PathBuf);

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn concurrent_store_writes_preserve_each_update() -> Result<(), String> {
        const WRITERS: usize = 8;
        const UPDATES: usize = 8;
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("impasto-storage-{}-{serial}", std::process::id()));
        fs::create_dir(&root).map_err(|error| error.to_string())?;
        let directory = TestDirectory(root);
        let root = &directory.0;
        // A preexisting store must survive concurrent read-modify-write operations.
        fs::write(root.join(STORE_NAME), "legacy\tvalue\\nwith\\ttabs\n")
            .map_err(|error| error.to_string())?;

        let start = Barrier::new(WRITERS);
        thread::scope(|scope| {
            let start = &start;
            let workers: Vec<_> = (0..WRITERS)
                .map(|writer| {
                    scope.spawn(move || {
                        start.wait();
                        for index in 0..UPDATES {
                            let key = format!("{writer}:{index}");
                            store_set(root, &[(key, "updated".to_owned())])?;
                        }
                        Ok::<(), String>(())
                    })
                })
                .collect();
            for worker in workers {
                worker.join().map_err(|_| "storage worker panicked")??;
            }
            Ok::<(), String>(())
        })?;

        let keys: Vec<_> = (0..WRITERS)
            .flat_map(|writer| (0..UPDATES).map(move |index| format!("{writer}:{index}")))
            .collect();
        let updated = store_get(root, &keys)?;
        for (key, value) in updated {
            assert_eq!(value.as_deref(), Some("updated"), "lost update for {key}");
        }
        assert_eq!(
            store_get(root, &["legacy".into()])?,
            [("legacy".into(), Some("value\nwith\ttabs".into()))]
        );
        Ok(())
    }
}
