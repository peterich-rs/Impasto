//! App and plugin source dependencies, with optional legacy package metadata.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum Source {
    Builtin(String),
    Local(Local),
    Git(Git),
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Local {
    path: PathBuf,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Git {
    git: String,
    tag: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct LockedGit {
    git: String,
    tag: String,
    revision: String,
    #[serde(default)]
    dependencies: BTreeSet<String>,
}

#[derive(Deserialize, Serialize)]
struct Lock {
    version: u32,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    roots: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    graph: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    packages: BTreeMap<String, LockedGit>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LockFile {
    Current(Lock),
    Legacy(BTreeMap<String, LockedGit>),
}

pub(crate) struct Resolver<'a> {
    root: &'a Path,
    framework: &'a Path,
    refresh: Option<Option<&'a str>>,
    locked: BTreeMap<String, LockedGit>,
    next_lock: BTreeMap<String, LockedGit>,
    sources: BTreeMap<String, (Source, PathBuf, PathBuf)>,
    edges: BTreeMap<PathBuf, BTreeMap<String, PathBuf>>,
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(
        root: &'a Path,
        framework: &'a Path,
        refresh: Option<Option<&'a str>>,
    ) -> Result<Self> {
        let lock_path = root.join("impasto.lock");
        let locked = match fs::read_to_string(&lock_path) {
            Ok(text) => match toml::from_str::<LockFile>(&text)
                .with_context(|| format!("解析 {}", lock_path.display()))?
            {
                LockFile::Current(lock) if lock.version == 2 => lock.packages,
                LockFile::Current(lock) => bail!("不支持 impasto.lock 版本 {}", lock.version),
                LockFile::Legacy(locked) => locked,
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        if locked.values().any(|entry| {
            !matches!(entry.revision.len(), 40 | 64)
                || !entry.revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            bail!("impasto.lock 包含无效的 Git revision");
        }
        Ok(Self {
            root,
            framework,
            refresh,
            locked,
            next_lock: BTreeMap::new(),
            sources: BTreeMap::new(),
            edges: BTreeMap::new(),
        })
    }

    pub(crate) fn dependencies(&mut self, owner: &Path) -> Result<BTreeMap<String, PathBuf>> {
        let Some(path) = ["app.toml", "plugin.toml"]
            .into_iter()
            .map(|name| owner.join(name))
            .find(|path| path.is_file())
        else {
            return Ok(BTreeMap::new());
        };
        #[derive(Deserialize)]
        struct Manifest {
            #[serde(default)]
            dependencies: BTreeMap<String, Source>,
        }
        let manifest: Manifest = toml::from_str(&fs::read_to_string(&path)?)
            .with_context(|| format!("解析 {}", path.display()))?;
        let mut resolved = BTreeMap::new();
        for (name, mut source) in manifest.dependencies {
            if !crate::plugins::valid_package_name(&name) {
                bail!("无效的 Impasto 包名 {name:?}");
            }
            if let Source::Local(local) = &mut source {
                let location = owner.join(&local.path);
                local.path = location.canonicalize().with_context(|| {
                    format!("无法解析依赖 {name} 的路径 {}", location.display())
                })?;
            }
            if let Some((previous, path, previous_owner)) = self.sources.get(&name) {
                if previous != &source {
                    bail!(
                        "依赖 {name} 的来源冲突：{} 声明 {previous:?}，{} 声明 {source:?}",
                        previous_owner.display(),
                        owner.display()
                    );
                }
                resolved.insert(name, path.clone());
                continue;
            }
            let location = match &source {
                Source::Builtin(kind) if kind == "builtin" => builtin(self.framework, &name)?,
                Source::Builtin(kind) => bail!("依赖 {name} 的来源 {kind:?} 无效"),
                Source::Local(location) => location.path.clone(),
                Source::Git(git) => {
                    let refresh = self
                        .refresh
                        .is_some_and(|which| which.is_none_or(|which| which == name));
                    let previous = self
                        .locked
                        .get(&name)
                        .filter(|entry| entry.git == git.git && entry.tag == git.tag);
                    let (location, revision) = checkout(git, previous, refresh)
                        .with_context(|| format!("解析依赖 {name}"))?;
                    self.next_lock.insert(
                        name.clone(),
                        LockedGit {
                            git: git.git.clone(),
                            tag: git.tag.clone(),
                            revision,
                            dependencies: BTreeSet::new(),
                        },
                    );
                    location
                }
            }
            .canonicalize()?;
            if !location.join("package.json").is_file() {
                bail!("依赖 {name} 缺少 package.json 源码入口描述");
            }
            self.sources
                .insert(name.clone(), (source, location.clone(), owner.to_owned()));
            resolved.insert(name, location);
        }
        self.edges.insert(owner.to_owned(), resolved.clone());
        Ok(resolved)
    }

    pub(crate) fn finish(mut self) -> Result<BTreeMap<String, PathBuf>> {
        fn visit<'a>(
            path: &'a Path,
            edges: &'a BTreeMap<PathBuf, BTreeMap<String, PathBuf>>,
            stack: &mut Vec<(&'a str, &'a Path)>,
            done: &mut BTreeSet<&'a Path>,
        ) -> Result<()> {
            if let Some(children) = edges.get(path) {
                for (name, child) in children {
                    if let Some(start) = stack.iter().position(|(_, path)| *path == child) {
                        let chain = stack[start..]
                            .iter()
                            .map(|(name, _)| *name)
                            .chain([name.as_str()])
                            .collect::<Vec<_>>()
                            .join(" -> ");
                        bail!("Impasto 依赖环路 {chain}");
                    }
                    if done.insert(child) {
                        stack.push((name, child));
                        visit(child, edges, stack, done)?;
                        stack.pop();
                    }
                }
            }
            Ok(())
        }

        let mut done = BTreeSet::new();
        for path in self.edges.keys() {
            if done.insert(path.as_path()) {
                visit(path, &self.edges, &mut vec![("root", path)], &mut done)?;
            }
        }
        if let Some(Some(package)) = self.refresh
            && !self.next_lock.contains_key(package)
        {
            bail!("{package} 不是当前依赖图中的 Git 包");
        }
        for (name, locked) in &mut self.next_lock {
            if let Some((_, location, _)) = self.sources.get(name)
                && let Some(children) = self.edges.get(location)
            {
                locked.dependencies = children.keys().cloned().collect();
            }
        }
        let lock_path = self.root.join("impasto.lock");
        if !self.sources.is_empty() || lock_path.is_file() {
            let roots = self
                .edges
                .get(self.root)
                .into_iter()
                .flat_map(BTreeMap::keys)
                .cloned()
                .collect();
            let graph = self
                .sources
                .iter()
                .map(|(name, (_, path, _))| {
                    let edges = self
                        .edges
                        .get(path)
                        .into_iter()
                        .flat_map(BTreeMap::keys)
                        .cloned()
                        .collect();
                    (name.clone(), edges)
                })
                .collect();
            let lock = Lock {
                version: 2,
                roots,
                graph,
                packages: self.next_lock,
            };
            crate::plugins::write(&lock_path, &toml::to_string_pretty(&lock)?)?;
        }
        Ok(self
            .sources
            .into_iter()
            .map(|(name, (_, path, _))| (name, path))
            .collect())
    }
}

pub(crate) fn builtin(framework: &Path, name: &str) -> Result<PathBuf> {
    let folder = name
        .strip_prefix("@impasto/")
        .context("内置包使用 @impasto/ 前缀")?;
    let root = framework.join("packages").join(folder);
    if !root.join("package.json").is_file() {
        bail!("未知内置包 {name}");
    }
    Ok(root)
}

fn checkout(git: &Git, locked: Option<&LockedGit>, refresh: bool) -> Result<(PathBuf, String)> {
    let url = url::Url::parse(&git.git).context("Git 依赖需要 HTTPS URL")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || git.tag.is_empty()
        || git.tag.starts_with('-')
    {
        bail!("Git 依赖需要无凭据的 HTTPS URL 和有效标签");
    }
    let home = std::env::var_os("IMPASTO_CACHE")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| PathBuf::from(home).join(".impasto"))
        })
        .context("无法定位 Git 包缓存，请设置 IMPASTO_CACHE")?;
    let packages = home.join("packages");
    fs::create_dir_all(&packages)?;
    let key = URL_SAFE_NO_PAD.encode(Sha256::digest(format!("{}\0{}", git.git, git.tag)));
    let legacy = packages.join(&key);
    let by_revision = |revision: &str| packages.join(format!("{key}-{revision}"));
    if !refresh {
        let pinned = locked.map(|lock| by_revision(&lock.revision));
        for directory in pinned.iter().chain([&legacy]).filter(|path| path.is_dir()) {
            let (revision, tag) = git_revision(directory, &git.tag)?;
            if revision == tag && locked.is_none_or(|lock| revision == lock.revision) {
                return Ok((directory.clone(), revision));
            }
        }
    }
    let partial = tempfile::tempdir_in(&packages)?;
    let mut clone = Command::new("git");
    clone
        .args([
            "clone",
            "--quiet",
            "--depth=1",
            "--branch",
            &git.tag,
            "--",
            &git.git,
        ])
        .arg(partial.path());
    crate::toolchain::run(clone, "获取 Git 包")?;
    let (revision, tag) = git_revision(partial.path(), &git.tag)?;
    if revision != tag || (!refresh && locked.is_some_and(|lock| revision != lock.revision)) {
        bail!("Git 标签 {} 与 impasto.lock 不一致", git.tag);
    }
    let directory = by_revision(&revision);
    if let Err(error) = fs::rename(partial.path(), &directory) {
        let (existing, _) = git_revision(&directory, &git.tag).context(error)?;
        if existing != revision {
            bail!("Git 缓存发生并发版本冲突");
        }
    }
    Ok((directory, revision))
}

fn git_revision(root: &Path, tag: &str) -> Result<(String, String)> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()?;
    let tagged = Command::new("git")
        .current_dir(root)
        .args([
            "rev-parse",
            "--verify",
            &format!("refs/tags/{tag}^{{commit}}"),
        ])
        .output()?;
    if !output.status.success() || !tagged.status.success() {
        bail!("无法读取 Git 标签 {tag}");
    }
    Ok((
        String::from_utf8(output.stdout)?.trim().to_owned(),
        String::from_utf8(tagged.stdout)?.trim().to_owned(),
    ))
}
