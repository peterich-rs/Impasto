//! Debounced native file notifications and on-demand snapshot reconciliation.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{app::App, dev::Event};

const QUIET: Duration = Duration::from_millis(40);
const MAX_BATCH: Duration = Duration::from_millis(120);
const CONFIG: &[&str] = &[
    "app.toml",
    "impasto.lock",
    "package.json",
    "pnpm-lock.yaml",
    "package-lock.json",
    "tsconfig.json",
];
const IGNORED: &[&str] = &[
    "node_modules",
    "build",
    "dist",
    "target",
    ".git",
    ".impasto",
    ".gradle",
    ".kotlin",
    ".build",
    ".swiftpm",
    "xcuserdata",
    "local.properties",
    ".DS_Store",
];

#[derive(Default)]
pub(crate) struct Batch {
    paths: BTreeSet<PathBuf>,
    replaced: BTreeSet<PathBuf>,
    rescan: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Scope {
    root: PathBuf,
    trees: BTreeSet<PathBuf>,
    files: BTreeSet<PathBuf>,
}

impl Scope {
    pub(crate) fn new(root: &Path, dirs: &[&str], files: &[&str]) -> Self {
        Self {
            root: root.to_owned(),
            trees: dirs.iter().map(|name| root.join(name)).collect(),
            files: files.iter().map(|name| root.join(name)).collect(),
        }
    }

    pub(crate) fn app(app: &App, extra: &[PathBuf]) -> Self {
        let mut scope = Self::new(
            &app.root,
            &[
                "src", "assets", "locales", "rust", "android", "ios", "macos",
            ],
            CONFIG,
        );
        scope.trees.insert(app.assets().to_owned());
        scope.trees.extend(crate::plugins::watched_paths(app));
        scope.extend(app.sources.values().chain(extra));
        scope
    }

    pub(crate) fn extend<'a>(&mut self, paths: impl IntoIterator<Item = &'a PathBuf>) {
        for path in paths {
            if path.is_file() {
                self.files.insert(path.clone());
            } else {
                self.trees.insert(path.clone());
            }
        }
    }

    fn accepts(&self, path: &Path) -> bool {
        self.files.contains(path)
            || self
                .trees
                .iter()
                .any(|tree| path.starts_with(tree) && !path.strip_prefix(tree).is_ok_and(ignored))
    }

    fn registrations(&self) -> BTreeMap<PathBuf, RecursiveMode> {
        let mut paths = BTreeMap::from([(self.root.clone(), RecursiveMode::NonRecursive)]);
        for path in self.files.iter().chain(self.trees.iter()) {
            for parent in path.ancestors().skip(1) {
                if parent.is_dir() {
                    paths
                        .entry(parent.to_owned())
                        .or_insert(RecursiveMode::NonRecursive);
                    break;
                }
            }
        }
        for tree in &self.trees {
            if tree.is_dir() && tree != &self.root {
                #[cfg(target_os = "linux")]
                linux_registrations(tree, &mut paths);
                #[cfg(not(target_os = "linux"))]
                paths.insert(tree.clone(), RecursiveMode::Recursive);
            }
        }
        let recursive: Vec<_> = paths
            .iter()
            .filter(|(_, mode)| **mode == RecursiveMode::Recursive)
            .map(|(path, _)| path.clone())
            .collect();
        paths.retain(|path, _| {
            !recursive
                .iter()
                .any(|root| root != path && path.starts_with(root))
        });
        paths
    }

    fn snapshot(&self) -> Result<Snapshot> {
        let mut snapshot = Snapshot::new();
        for path in self.trees.iter().chain(&self.files) {
            scan(path, &mut snapshot)?;
        }
        Ok(snapshot)
    }
}

// inotify registers each descendant separately. Keep generated build trees
// out of the kernel watch table as well as out of the event stream.
#[cfg(target_os = "linux")]
fn linux_registrations(directory: &Path, paths: &mut BTreeMap<PathBuf, RecursiveMode>) {
    paths.insert(directory.to_owned(), RecursiveMode::NonRecursive);
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir())
            && !ignored(Path::new(&entry.file_name()))
        {
            linux_registrations(&entry.path(), paths);
        }
    }
}

pub(crate) type Snapshot = BTreeMap<PathBuf, (SystemTime, u64)>;

pub(crate) fn changes(previous: &Snapshot, current: &Snapshot) -> Vec<PathBuf> {
    current
        .iter()
        .filter(|(path, stamp)| previous.get(*path) != Some(*stamp))
        .map(|(path, _)| path)
        .chain(previous.keys().filter(|path| !current.contains_key(*path)))
        .cloned()
        .collect()
}

pub(crate) struct ProjectWatch {
    watcher: RecommendedWatcher,
    watched: BTreeMap<PathBuf, RecursiveMode>,
    scope: Scope,
    snapshot: Snapshot,
}

impl ProjectWatch {
    pub(crate) fn new(scope: Scope, events: Sender<Event>) -> Result<Self> {
        let (raw, rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let watcher = notify::recommended_watcher(raw)?;
        thread::Builder::new()
            .name("impasto-file-events".into())
            .spawn(move || {
                while let Ok(first) = rx.recv() {
                    let mut batch = Batch::default();
                    let collect =
                        |batch: &mut Batch, event: notify::Result<notify::Event>| match event {
                            Ok(event) if !matches!(event.kind, EventKind::Access(_)) => {
                                batch.rescan |= event.need_rescan();
                                if matches!(
                                    event.kind,
                                    EventKind::Create(_)
                                        | EventKind::Remove(_)
                                        | EventKind::Modify(notify::event::ModifyKind::Name(_))
                                ) {
                                    batch.replaced.extend(event.paths.iter().cloned());
                                }
                                batch.paths.extend(event.paths);
                            }
                            Err(error) => {
                                let _ = events.send(Event::Warning(format!("文件监听: {error}")));
                                batch.rescan = true;
                            }
                            Ok(_) => {}
                        };
                    collect(&mut batch, first);
                    let limit = Instant::now() + MAX_BATCH;
                    while let Ok(event) =
                        rx.recv_timeout(QUIET.min(limit.saturating_duration_since(Instant::now())))
                    {
                        collect(&mut batch, event);
                        if Instant::now() >= limit {
                            break;
                        }
                    }
                    if (batch.rescan || !batch.paths.is_empty())
                        && events.send(Event::Files(batch)).is_err()
                    {
                        break;
                    }
                }
            })?;
        let mut this = Self {
            watcher,
            watched: BTreeMap::new(),
            scope,
            snapshot: Snapshot::new(),
        };
        this.register()?;
        this.snapshot = this.scope.snapshot()?;
        Ok(this)
    }

    pub(crate) fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Add inputs discovered by the linker without acknowledging unprocessed edits.
    pub(crate) fn set_scope(&mut self, scope: Scope) -> Result<()> {
        if self.scope == scope {
            return Ok(());
        }
        let old = std::mem::replace(&mut self.scope, scope);
        self.register()?;
        self.snapshot.retain(|path, _| self.scope.accepts(path));
        for path in self
            .scope
            .trees
            .difference(&old.trees)
            .chain(self.scope.files.difference(&old.files))
        {
            let mut added = Snapshot::new();
            scan(path, &mut added)?;
            for (path, stamp) in added {
                self.snapshot.entry(path).or_insert(stamp);
            }
        }
        Ok(())
    }

    fn register(&mut self) -> Result<()> {
        let wanted = self.scope.registrations();
        self.watched.retain(|path, mode| {
            if wanted.get(path) == Some(mode) {
                true
            } else {
                let _ = self.watcher.unwatch(path);
                false
            }
        });
        for (path, mode) in wanted {
            if !self.watched.contains_key(&path) {
                self.watcher
                    .watch(&path, mode)
                    .with_context(|| format!("监听 {}", path.display()))?;
                self.watched.insert(path, mode);
            }
        }
        Ok(())
    }

    pub(crate) fn reconcile(&mut self, batch: Batch) -> Result<Vec<PathBuf>> {
        let mut registrations_changed = false;
        for path in &batch.replaced {
            if self.watched.contains_key(path) && path != &self.scope.root {
                self.watched.remove(path);
                let _ = self.watcher.unwatch(path);
                registrations_changed = true;
            }
            #[cfg(target_os = "linux")]
            if self.scope.accepts(path) && path.is_dir() {
                registrations_changed = true;
            }
        }
        let rescan = batch.rescan
            || batch.paths.iter().any(|path| {
                let root_config = path.parent() == Some(self.scope.root.as_path())
                    && self.scope.files.contains(path);
                let tree_root = path != &self.scope.root
                    && (self.scope.trees.iter().any(|tree| tree.starts_with(path))
                        || self
                            .scope
                            .files
                            .iter()
                            .any(|file| file != path && file.starts_with(path)));
                root_config || tree_root
            });
        if rescan || registrations_changed {
            self.register()?;
        }
        if rescan {
            let next = self.scope.snapshot()?;
            let changed = changes(&self.snapshot, &next);
            self.snapshot = next;
            return Ok(changed);
        }
        let mut changed = BTreeSet::new();
        for path in batch.paths {
            if !self.scope.accepts(&path) {
                continue;
            }
            let Some(meta) = metadata(&path)? else {
                self.snapshot.retain(|file, _| {
                    if file.starts_with(&path) {
                        changed.insert(file.clone());
                        false
                    } else {
                        true
                    }
                });
                continue;
            };
            if meta.is_file() {
                let stamp = (meta.modified()?, meta.len());
                if self.snapshot.insert(path.clone(), stamp) != Some(stamp) {
                    changed.insert(path);
                }
                continue;
            }
            let mut local = Snapshot::new();
            scan(&path, &mut local)?;
            self.snapshot.retain(|file, _| {
                if file.starts_with(&path) && !local.contains_key(file) {
                    changed.insert(file.clone());
                    false
                } else {
                    true
                }
            });
            for (file, stamp) in local {
                if self.snapshot.insert(file.clone(), stamp) != Some(stamp) {
                    changed.insert(file);
                }
            }
        }
        Ok(changed.into_iter().collect())
    }
}

fn ignored(path: &Path) -> bool {
    path.iter()
        .any(|part| IGNORED.iter().any(|word| part == *word))
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.ends_with('~')
                    || name.ends_with(".swp")
                    || name.ends_with(".swx")
                    || name.starts_with(".#")
            })
}

fn metadata(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn scan(path: &Path, out: &mut Snapshot) -> io::Result<()> {
    let Some(meta) = metadata(path)? else {
        return Ok(());
    };
    if meta.is_dir() {
        let entries = match fs::read_dir(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            result => result?,
        };
        for entry in entries {
            let entry = entry?;
            if !ignored(Path::new(&entry.file_name())) {
                scan(&entry.path(), out)?;
            }
        }
    } else if meta.is_file() {
        out.insert(path.to_owned(), (meta.modified()?, meta.len()));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Tier {
    AssetOnly,
    ScriptAndConfig,
    NativeRust,
    ShellOrManifest,
}

pub(crate) fn classify(app: &App, path: &Path) -> Tier {
    if let Ok(relative) = path.strip_prefix(app.assets()) {
        return if matches!(
            relative.extension().and_then(|ext| ext.to_str()),
            Some("png" | "jpg" | "jpeg" | "webp" | "ttf" | "otf")
        ) {
            Tier::AssetOnly
        } else {
            Tier::ScriptAndConfig
        };
    }
    if [&app.ios.privacy_manifest, &app.ios.launch_storyboard]
        .into_iter()
        .flatten()
        .any(|file| app.root.join(file) == path)
    {
        return Tier::ShellOrManifest;
    }
    if let Ok(relative) = path.strip_prefix(&app.root) {
        match relative.iter().next().and_then(|name| name.to_str()) {
            Some("src" | "locales") => return Tier::ScriptAndConfig,
            Some("rust") => return Tier::NativeRust,
            Some("android" | "ios" | "macos") => return Tier::ShellOrManifest,
            Some(name) if CONFIG.contains(&name) => return Tier::ShellOrManifest,
            _ => {}
        }
    }
    if app.sources.values().any(|root| path.starts_with(root))
        || crate::plugins::is_wrapper_source(path)
    {
        Tier::ScriptAndConfig
    } else if path.components().any(|part| part.as_os_str() == "rust")
        || matches!(path.extension().and_then(|ext| ext.to_str()), Some("rs"))
        || path
            .file_name()
            .is_some_and(|name| name == "Cargo.toml" || name == "Cargo.lock")
    {
        Tier::NativeRust
    } else {
        Tier::ShellOrManifest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_missing_source_tree_and_reconciles_atomic_replaces() {
        let project = tempfile::tempdir().unwrap();
        let root = project.path().canonicalize().unwrap();
        let sources = root.join("src");
        let file = sources.join("main.ts");
        let (send, events) = mpsc::channel();
        let mut watch =
            ProjectWatch::new(Scope::new(&root, &["src"], &["app.toml"]), send).unwrap();
        let mut next_change = || {
            let deadline = Instant::now() + Duration::from_secs(4);
            loop {
                let event = events
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .unwrap();
                if let Event::Files(batch) = event {
                    let changed = watch.reconcile(batch).unwrap();
                    if !changed.is_empty() {
                        return changed;
                    }
                }
            }
        };
        fs::create_dir(&sources).unwrap();
        fs::write(&file, "before").unwrap();
        assert_eq!(next_change(), std::slice::from_ref(&file));
        let swap = sources.join(".main.ts.swp");
        fs::write(&swap, "after the editor replaced the file").unwrap();
        fs::rename(&swap, &file).unwrap();
        assert_eq!(next_change(), std::slice::from_ref(&file));
        fs::remove_file(&file).unwrap();
        assert_eq!(next_change(), [file]);
    }
}
