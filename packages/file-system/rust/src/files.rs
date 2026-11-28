use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use impasto::Bytes;
use tempfile::NamedTempFile;

use crate::{DirEntry, FileError, FileKind, FileStat};

pub(super) fn relative_path(path: &str) -> Result<&str, FileError> {
    if path
        .split('/')
        .any(|part| part.is_empty() || matches!(part, "." | ".."))
        || path.contains(['\\', ':', '\0'])
    {
        return Err(FileError::outside(path));
    }
    Ok(path)
}

pub(super) fn resolve(root: &Path, path: &str) -> Result<PathBuf, FileError> {
    let base = fs::canonicalize(root)?;
    let target = base.join(relative_path(path)?);
    if !canonical_candidate(&target)?.starts_with(&base) {
        return Err(FileError::outside(path));
    }
    Ok(target)
}

fn canonical_candidate(target: &Path) -> Result<PathBuf, FileError> {
    for ancestor in target.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => {
                let canonical = fs::canonicalize(ancestor).map_err(|error| {
                    if error.kind() == io::ErrorKind::NotFound {
                        FileError::outside(&target.to_string_lossy())
                    } else {
                        error.into()
                    }
                })?;
                return target
                    .strip_prefix(ancestor)
                    .map(|suffix| canonical.join(suffix))
                    .map_err(|_| FileError::outside(&target.to_string_lossy()));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(FileError::outside(&target.to_string_lossy()))
}

pub(super) fn metadata_kind(meta: &Metadata) -> Result<FileKind, FileError> {
    if meta.is_file() {
        Ok(FileKind::File)
    } else if meta.is_dir() {
        Ok(FileKind::Directory)
    } else {
        Err(FileError::new("not-a-file", "unsupported filesystem entry"))
    }
}

pub(super) fn open_file(path: &Path) -> Result<File, FileError> {
    if !fs::metadata(path)?.is_file() {
        return Err(FileError::new("not-a-file", "expected a regular file"));
    }
    Ok(File::open(path)?)
}

pub(super) fn stat(path: &Path) -> Result<FileStat, FileError> {
    let meta = fs::metadata(path)?;
    let time = |value: io::Result<std::time::SystemTime>| {
        value
            .ok()
            .map(|value| match value.duration_since(std::time::UNIX_EPOCH) {
                Ok(elapsed) => elapsed.as_secs_f64() * 1000.0,
                Err(before) => -before.duration().as_secs_f64() * 1000.0,
            })
    };
    Ok(FileStat {
        kind: metadata_kind(&meta)?,
        size: meta.len() as f64,
        modified_at_ms: time(meta.modified()),
        created_at_ms: time(meta.created()),
        readonly: meta.permissions().readonly(),
    })
}

pub(super) fn list(root: &Path, parent: &str) -> Result<Vec<DirEntry>, FileError> {
    let path = if parent.is_empty() {
        root.to_owned()
    } else {
        resolve(root, parent)?
    };
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = if parent.is_empty() {
            name.clone()
        } else {
            format!("{parent}/{name}")
        };
        let full = match resolve(root, &path) {
            Ok(full) => full,
            Err(error) if error.code == "outside-scope" => continue,
            Err(error) => return Err(error),
        };
        let meta = fs::metadata(full)?;
        entries.push(DirEntry {
            name,
            path,
            kind: metadata_kind(&meta)?,
            size: meta.len() as f64,
        });
    }
    entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

pub(super) fn parent(path: &Path) -> Result<&Path, FileError> {
    path.parent()
        .ok_or_else(|| FileError::outside(&path.to_string_lossy()))
}

pub(super) fn copy(from: &Path, to: &Path, overwrite: bool) -> Result<(), FileError> {
    if from == to {
        return Err(FileError::new(
            "already-exists",
            "source equals destination",
        ));
    }
    reject_nested_destination(from, to, &fs::symlink_metadata(from)?)?;
    fs::create_dir_all(parent(to)?)?;
    copy_entry(from, to, overwrite)
}

fn reject_nested_destination(from: &Path, to: &Path, metadata: &Metadata) -> Result<(), FileError> {
    if metadata.is_dir() && canonical_candidate(to)?.starts_with(fs::canonicalize(from)?) {
        return Err(FileError::outside(&to.to_string_lossy()));
    }
    Ok(())
}

fn copy_entry(from: &Path, to: &Path, overwrite: bool) -> Result<(), FileError> {
    let meta = fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        return Err(FileError::outside(&from.to_string_lossy()));
    }
    if meta.is_dir() {
        if to.starts_with(from) {
            return Err(FileError::outside(&to.to_string_lossy()));
        }
        // Stage directories too, so a failed child copy leaves no partial destination.
        let temp = tempfile::tempdir_in(parent(to)?)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_entry(&entry.path(), &temp.path().join(entry.file_name()), false)?;
        }
        match rename_directory(temp.path(), to, overwrite) {
            Err(error) if overwrite && error.kind() == io::ErrorKind::DirectoryNotEmpty => {
                exchange_directories(temp.path(), to)?;
            }
            result => result?,
        }
    } else {
        let mut staged = NamedTempFile::new_in(parent(to)?)?;
        io::copy(&mut open_file(from)?, &mut staged)?;
        staged.as_file().sync_all()?;
        persist(staged, to, overwrite)?;
    }
    Ok(())
}

pub(super) fn move_entry(from: &Path, to: &Path, overwrite: bool) -> Result<(), FileError> {
    if from == to {
        return Ok(());
    }
    let meta = fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        return Err(FileError::outside(&from.to_string_lossy()));
    }
    reject_nested_destination(from, to, &meta)?;
    fs::create_dir_all(parent(to)?)?;
    if !overwrite && meta.is_file() {
        match fs::hard_link(from, to) {
            Ok(()) => {
                fs::remove_file(from)?;
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Err(error.into()),
            Err(_) => {}
        }
    } else {
        let result = if meta.is_dir() {
            rename_directory(from, to, overwrite)
        } else {
            fs::rename(from, to)
        };
        match result {
            Ok(()) => return Ok(()),
            Err(error)
                if error.kind() == io::ErrorKind::CrossesDevices
                    || (overwrite
                        && meta.is_dir()
                        && error.kind() == io::ErrorKind::DirectoryNotEmpty) => {}
            Err(error) => return Err(error.into()),
        }
    }
    copy(from, to, overwrite)?;
    if meta.is_dir() {
        fs::remove_dir_all(from)?;
    } else {
        fs::remove_file(from)?;
    }
    Ok(())
}

fn rename_directory(from: &Path, to: &Path, overwrite: bool) -> io::Result<()> {
    if overwrite {
        return fs::rename(from, to);
    }
    #[cfg(any(target_vendor = "apple", target_os = "android", target_os = "linux"))]
    {
        use rustix::fs::{CWD, RenameFlags, renameat_with};
        renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE).map_err(Into::into)
    }
    #[cfg(not(any(target_vendor = "apple", target_os = "android", target_os = "linux")))]
    {
        if to.try_exists()? {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        fs::rename(from, to)
    }
}

#[cfg(any(target_vendor = "apple", target_os = "android", target_os = "linux"))]
fn exchange_directories(from: &Path, to: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    renameat_with(CWD, from, CWD, to, RenameFlags::EXCHANGE).map_err(Into::into)
}

#[cfg(not(any(target_vendor = "apple", target_os = "android", target_os = "linux")))]
fn exchange_directories(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::ErrorKind::DirectoryNotEmpty.into())
}

fn persist(file: NamedTempFile, path: &Path, overwrite: bool) -> Result<(), FileError> {
    let result = if overwrite {
        file.persist(path)
    } else {
        file.persist_noclobber(path)
    };
    result.map(|_| ()).map_err(|error| error.error.into())
}

enum Writer {
    Direct(BufWriter<File>),
    Atomic(BufWriter<NamedTempFile>, PathBuf),
}

/// An open sandbox writer. Dropping an uncommitted atomic writer removes its staging file.
#[impasto::object]
pub struct FileWriter(Mutex<Option<Writer>>);

impl FileWriter {
    pub(super) fn open(path: PathBuf, append: bool, atomic: bool) -> Result<Self, FileError> {
        fs::create_dir_all(parent(&path)?)?;
        let writer = if atomic {
            let mut staged = NamedTempFile::new_in(parent(&path)?)?;
            if append {
                match open_file(&path) {
                    Ok(mut original) => {
                        io::copy(&mut original, &mut staged)?;
                    }
                    Err(error) if error.code == "not-found" => {}
                    Err(error) => return Err(error),
                }
            }
            Writer::Atomic(BufWriter::new(staged), path)
        } else {
            Writer::Direct(BufWriter::new(
                OpenOptions::new()
                    .create(true)
                    .write(true)
                    .append(append)
                    .truncate(!append)
                    .open(path)?,
            ))
        };
        Ok(Self(Mutex::new(Some(writer))))
    }

    pub(super) fn write_bytes(&self, buffer: &[u8]) -> Result<(), FileError> {
        self.with_writer(|writer| writer.write_all(buffer))
    }

    fn with_writer(
        &self,
        op: impl FnOnce(&mut dyn Write) -> io::Result<()>,
    ) -> Result<(), FileError> {
        let mut writer = self.0.lock().map_err(FileError::io)?;
        match writer.as_mut().ok_or_else(closed)? {
            Writer::Direct(writer) => op(writer)?,
            Writer::Atomic(writer, _) => op(writer)?,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_writer_keeps_existing_contents_until_commit() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("notes.txt");
        fs::write(&path, "old").unwrap();

        let writer = FileWriter::open(path.clone(), false, true).unwrap();
        writer.write_bytes(b"aborted").unwrap();
        writer.abort().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);

        let writer = FileWriter::open(path.clone(), true, true).unwrap();
        writer.write_bytes(b" and new").unwrap();
        writer.flush().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "old");
        writer.commit().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "old and new");
    }

    #[cfg(any(target_vendor = "apple", target_os = "android", target_os = "linux"))]
    #[test]
    fn directory_transfers_only_replace_existing_contents_when_requested() {
        let root = tempfile::tempdir().unwrap();
        let from = root.path().join("source");
        let to = root.path().join("destination");
        fs::create_dir(&from).unwrap();
        fs::create_dir(&to).unwrap();
        fs::write(from.join("new.txt"), "new").unwrap();
        fs::write(to.join("old.txt"), "old").unwrap();

        assert_eq!(copy(&from, &to, false).unwrap_err().code, "already-exists");
        assert_eq!(
            move_entry(&from, &to, false).unwrap_err().code,
            "already-exists"
        );
        assert!(to.join("old.txt").is_file());
        copy(&from, &to, true).unwrap();
        assert!(from.join("new.txt").is_file());
        assert!(!to.join("old.txt").exists());
        fs::write(from.join("new.txt"), "moved").unwrap();
        move_entry(&from, &to, true).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read_to_string(to.join("new.txt")).unwrap(), "moved");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_external_links_and_a_directory_copy_into_its_own_alias() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        symlink(external.path(), root.path().join("external")).unwrap();
        symlink(
            external.path().join("missing"),
            root.path().join("dangling"),
        )
        .unwrap();
        for path in ["external/new/file", "dangling"] {
            assert_eq!(
                resolve(root.path(), path).unwrap_err().code,
                "outside-scope"
            );
        }

        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        symlink(&source, root.path().join("alias")).unwrap();
        let destination = root.path().join("alias/new/copy");
        assert_eq!(
            copy(&source, &destination, false).unwrap_err().code,
            "outside-scope"
        );
        assert!(!source.join("new").exists());
    }
}

fn closed() -> FileError {
    FileError::new("io", "file writer is closed")
}

#[impasto::export]
impl FileWriter {
    #[impasto::method(blocking)]
    pub fn write_text(&self, text: String) -> Result<(), FileError> {
        self.write_bytes(text.as_bytes())
    }

    #[impasto::method(blocking)]
    pub fn write(&self, data: Bytes) -> Result<(), FileError> {
        self.write_bytes(&data.0)
    }

    #[impasto::method(blocking)]
    pub fn flush(&self) -> Result<(), FileError> {
        self.with_writer(|writer| writer.flush())
    }

    #[impasto::method(blocking)]
    pub fn commit(&self) -> Result<(), FileError> {
        let writer = self
            .0
            .lock()
            .map_err(FileError::io)?
            .take()
            .ok_or_else(closed)?;
        match writer {
            Writer::Direct(writer) => {
                let file = writer.into_inner().map_err(|error| error.into_error())?;
                file.sync_all()?;
            }
            Writer::Atomic(writer, path) => {
                let file = writer.into_inner().map_err(|error| error.into_error())?;
                file.as_file().sync_all()?;
                persist(file, &path, true)?;
            }
        }
        Ok(())
    }

    #[impasto::method(blocking)]
    pub fn abort(&self) -> Result<(), FileError> {
        let writer = self.0.lock().map_err(FileError::io)?.take();
        if let Some(Writer::Direct(writer)) = writer {
            let _ = writer.into_parts();
        }
        Ok(())
    }
}
