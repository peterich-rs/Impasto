//! Document paths stay with the session. Transport values are the `js_host` types.

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

/// Read a packaged resource directly into the shared buffer consumed by the session.
pub fn read_asset_bytes(
    mut reader: impl std::io::Read,
    length: usize,
    limit: usize,
) -> Result<std::sync::Arc<[u8]>, String> {
    use std::sync::Arc;
    if length == 0 || length > limit {
        return Err("asset is empty or exceeds its byte limit".to_owned());
    }
    let mut bytes: Arc<[u8]> = std::iter::repeat_n(0, length).collect();
    let output = Arc::get_mut(&mut bytes).ok_or_else(|| "asset buffer is shared".to_owned())?;
    reader
        .read_exact(output)
        .map_err(|error| error.to_string())?;
    if reader.read(&mut [0]).map_err(|error| error.to_string())? != 0 {
        return Err("asset grew while it was being read".to_owned());
    }
    Ok(bytes)
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
