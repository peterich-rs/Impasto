//! Sandboxed document-directory files.

use std::fs;
use std::path::{Path, PathBuf};

use impasto::tracing;

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

fn io_fail(op: &'static str, path: &str, error: String) -> String {
    tracing::warn!(target: "impasto::io", op, path, error = %error, "file io failed");
    error
}

fn joined(root: &Path, path: &str) -> Result<PathBuf, String> {
    let relative = document_relative(path)?;
    Ok(root.join(relative))
}

/// A path relative to the document directory. Rejects `..`, absolute paths, and backslashes.
pub fn document_relative(path: &str) -> Result<String, String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains('\\')
        || path.contains(':')
    {
        return Err("path is outside the document directory".to_owned());
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err("path is outside the document directory".to_owned());
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::document_relative;

    #[test]
    fn document_relative_keeps_nested_files() {
        match document_relative("a/b.bin") {
            Ok(path) => assert_eq!(path, "a/b.bin"),
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn document_relative_rejects_escape() {
        for path in [
            "", "/", "\\a", "a\\b", "C:foo", "..", "a/../b", "a/./b", "/abs",
        ] {
            match document_relative(path) {
                Err(message) => {
                    assert!(message.contains("outside"), "{path}: {message}");
                }
                Ok(ok) => panic!("{path} should fail, got {ok}"),
            }
        }
    }
}
