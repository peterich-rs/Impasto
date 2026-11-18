//! Document paths stay with the session. Transport values are the `js_host` types.

pub use js_host::{
    HTTP_BODY_LIMIT, HttpClient, HttpError, HttpFailure, HttpPiece, HttpRequest, HttpResponse,
    IoEvent, IoJob, PluginResult, PluginValue, module_not_linked,
};

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
