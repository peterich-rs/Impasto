//! Document-directory files and the string map behind `AsyncStorage`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use session::document_relative;

const STORE_NAME: &str = "async-storage.tsv";

pub fn read_document(root: &Path, path: &str) -> Result<Vec<u8>, String> {
    let full = joined(root, path)?;
    fs::read(&full).map_err(|error| error.to_string())
}

pub fn write_document(root: &Path, path: &str, bytes: &[u8]) -> Result<(), String> {
    let full = joined(root, path)?;
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&full, bytes).map_err(|error| error.to_string())
}

pub fn delete_document(root: &Path, path: &str) -> Result<(), String> {
    let full = joined(root, path)?;
    if full.exists() {
        fs::remove_file(&full).map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn store_get(root: &Path, keys: &[String]) -> Result<Vec<(String, Option<String>)>, String> {
    let map = load_store(root)?;
    Ok(keys
        .iter()
        .map(|key| {
            let value = map.get(key).cloned();
            (key.clone(), value)
        })
        .collect())
}

pub fn store_set(root: &Path, rows: &[(String, String)]) -> Result<(), String> {
    let mut map = load_store(root)?;
    for (key, value) in rows {
        map.insert(key.clone(), value.clone());
    }
    save_store(root, &map)
}

pub fn store_remove(root: &Path, keys: &[String]) -> Result<(), String> {
    let mut map = load_store(root)?;
    for key in keys {
        map.remove(key);
    }
    save_store(root, &map)
}

pub fn store_clear(root: &Path) -> Result<(), String> {
    let path = root.join(STORE_NAME);
    if path.exists() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn joined(root: &Path, path: &str) -> Result<PathBuf, String> {
    let relative = document_relative(path)?;
    Ok(root.join(relative))
}

fn load_store(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let path = root.join(STORE_NAME);
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
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
    fs::write(root.join(STORE_NAME), text).map_err(|error| error.to_string())
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
