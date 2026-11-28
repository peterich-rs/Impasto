use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;
use url::Url;

use crate::DesktopError;
use crate::types::string;

fn under_roots(path: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    if path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return None;
    }
    roots.iter().find_map(|root| {
        let root = fs::canonicalize(root).ok()?;
        let file = fs::canonicalize(root.join(path)).ok()?;
        (file.starts_with(&root) && file.is_file()).then_some(file)
    })
}

pub(crate) fn source_url(
    path: &Path,
    roots: &[PathBuf],
    line: u32,
    column: u32,
    editor: &str,
) -> Result<Url, DesktopError> {
    let file = under_roots(path, roots)
        .ok_or_else(|| DesktopError::new("Source file is outside the target's registered roots"))?;
    let mut uri = match editor {
        "vscode" => Url::parse("vscode://file/"),
        "cursor" => Url::parse("cursor://file/"),
        "zed" => Url::parse("zed://file/"),
        _ => return Err(DesktopError::new("Unknown editor")),
    }
    .map_err(DesktopError::new)?;
    uri.set_path(&format!(
        "{}:{}:{}",
        file.display(),
        line.max(1),
        column.max(1)
    ));
    Ok(uri)
}

pub(crate) fn open(target: &str) -> Result<(), DesktopError> {
    let status = Command::new("/usr/bin/open")
        .arg(target)
        .stdin(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(DesktopError::with_code(
            "io",
            "macOS could not open the requested application",
        ))
    }
}

pub(crate) fn reveal(path: &Path) -> Result<(), DesktopError> {
    let status = Command::new("/usr/bin/open")
        .arg("-R")
        .arg(path)
        .stdin(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(DesktopError::with_code(
            "io",
            "macOS could not reveal the exported trace",
        ))
    }
}

pub(crate) fn format_curl(record: &Value) -> String {
    let quote = |text: &str| format!("'{}'", text.replace('\'', "'\\''"));
    let mut command = format!("curl -X {}", quote(string(record, "method")));
    for header in record["request_headers"].as_array().into_iter().flatten() {
        if let Some((name, value)) = header[0].as_str().zip(header[1].as_str()) {
            command.push_str(" -H ");
            command.push_str(&quote(&format!("{name}: {value}")));
        }
    }
    if record["request_body_truncated"] != true
        && let Ok(body) = String::from_utf8(crate::store::network::preview(
            &record["request_body_preview"],
        ))
        && !body.is_empty()
        && !body.contains('\0')
    {
        command.push_str(" --data-raw ");
        command.push_str(&quote(&body));
    }
    command.push_str(" -- ");
    command.push_str(&quote(string(record, "url")));
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_checks_editor_name_and_canonical_scope() -> Result<(), DesktopError> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("app");
        fs::create_dir(&root)?;
        fs::write(temp.path().join("outside"), "")?;
        fs::write(root.join("x y.ts"), "")?;
        std::os::unix::fs::symlink(temp.path().join("outside"), root.join("link"))?;
        let roots = [root];
        assert!(source_url(Path::new("../outside"), &roots, 1, 1, "vscode").is_err());
        assert!(source_url(Path::new("link"), &roots, 1, 1, "vscode").is_err());
        assert!(source_url(Path::new("x y.ts"), &roots, 1, 1, "other").is_err());
        assert!(
            source_url(Path::new("x y.ts"), &roots, 2, 3, "vscode")?
                .as_str()
                .ends_with("x%20y.ts:2:3")
        );
        Ok(())
    }

    #[test]
    fn curl_preserves_redaction_and_quotes_arguments() {
        let record = serde_json::json!({"method":"GET","url":"https://example.test/o'neil","request_headers":[["authorization","***"]]});
        assert_eq!(
            format_curl(&record),
            "curl -X 'GET' -H 'authorization: ***' -- 'https://example.test/o'\\''neil'"
        );
    }
}
