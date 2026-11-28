use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use devtools_protocol::{LocalSessionEntry, PROTOCOL_VERSION_V2};

pub(super) struct Registration(PathBuf);

impl Registration {
    pub(super) fn publish(port: u16, token: &str) -> io::Result<Self> {
        let root = std::env::var_os("IMPASTO_DEVTOOLS_DISCOVERY_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("impasto-devtools"));
        let entry = LocalSessionEntry {
            pid: std::process::id(),
            app_name: app_name(),
            platform: std::env::consts::OS.to_owned(),
            port,
            token: token.to_owned(),
            protocol_version: PROTOCOL_VERSION_V2,
            project_root: std::env::var_os("IMPASTO_DEVTOOLS_PROJECT").map(PathBuf::from),
            framework_root: std::env::var_os("IMPASTO_DEVTOOLS_FRAMEWORK").map(PathBuf::from),
            source_map: std::env::var_os("IMPASTO_DEVTOOLS_SOURCE_MAP").map(PathBuf::from),
            started_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
                .unwrap_or_default(),
        };
        Self::write(&root, &entry)
    }

    fn write(root: &Path, entry: &LocalSessionEntry) -> io::Result<Self> {
        private_directory(root)?;
        let directory = root.join("sessions");
        private_directory(&directory)?;
        let path = directory.join(format!("{}.json", entry.pid));
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer(file.as_file_mut(), entry)?;
        file.flush()?;
        file.persist(&path).map_err(|error| error.error)?;
        Ok(Self(path))
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn private_directory(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(unix)]
    let private = {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o777 == 0o700
    };
    #[cfg(not(unix))]
    let private = true;
    if !metadata.is_dir() || !private {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "DevTools discovery directory must be private",
        ));
    }
    Ok(())
}

pub(super) fn app_name() -> String {
    std::env::var("IMPASTO_DEVTOOLS_APP_NAME")
        .ok()
        .or_else(|| {
            std::env::current_exe().ok().and_then(|exe| {
                exe.file_stem()
                    .map(|name| name.to_string_lossy().into_owned())
            })
        })
        .unwrap_or_else(|| "Impasto".to_owned())
}

#[cfg(any(test, not(target_os = "android")))]
pub(super) fn token() -> io::Result<String> {
    use std::fmt::Write as _;

    let mut random = [0; 32];
    getrandom::fill(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
    let mut token = String::with_capacity(64);
    for byte in random {
        let _ = write!(token, "{byte:02x}");
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_file_is_private_and_removed_on_drop() {
        let root = tempfile::tempdir().unwrap();
        let entry = LocalSessionEntry {
            pid: 123,
            app_name: "Example".to_owned(),
            platform: "macos".to_owned(),
            port: 1234,
            token: token().unwrap(),
            protocol_version: PROTOCOL_VERSION_V2,
            project_root: None,
            framework_root: None,
            source_map: None,
            started_at_ms: 0,
        };
        let registration = Registration::write(&root.path().join("devtools"), &entry).unwrap();
        let path = registration.0.clone();
        let stored: LocalSessionEntry = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored.token, entry.token);
        assert_eq!(stored.token.len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        drop(registration);
        assert!(!path.exists());
    }
}
