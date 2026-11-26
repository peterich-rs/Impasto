//! App and plugin source dependencies, with optional legacy package metadata.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
#[serde(untagged)]
enum Source {
    Builtin(String),
    Local(Local),
    Git(Git),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Local {
    path: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Git {
    git: String,
    tag: String,
}

#[derive(Deserialize, Serialize)]
struct LockedGit {
    git: String,
    tag: String,
    revision: String,
}

pub(crate) fn declared(root: &Path, framework: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let Some(path) = ["app.toml", "plugin.toml"]
        .into_iter()
        .map(|name| root.join(name))
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
    let lock_path = root.join("impasto.lock");
    let locked = match fs::read_to_string(&lock_path) {
        Ok(text) => toml::from_str::<BTreeMap<String, LockedGit>>(&text)
            .with_context(|| format!("解析 {}", lock_path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(error) => return Err(error.into()),
    };
    let mut next_lock = BTreeMap::new();
    let mut resolved = BTreeMap::new();
    for (name, source) in manifest.dependencies {
        if !crate::plugins::valid_package_name(&name) {
            bail!("无效的 Impasto 包名 {name:?}");
        }
        let location = match source {
            Source::Builtin(kind) if kind == "builtin" => builtin(framework, &name)?,
            Source::Builtin(kind) => bail!(
                "依赖 {name} 的来源 {kind:?} 无效，使用 \"builtin\"、{{ path = ... }} 或 {{ git = ..., tag = ... }}"
            ),
            Source::Local(location) => root.join(location.path),
            Source::Git(git) => {
                let previous = locked
                    .get(&name)
                    .filter(|entry| entry.git == git.git && entry.tag == git.tag);
                let (location, revision) = checkout(&git, previous)?;
                next_lock.insert(
                    name.clone(),
                    LockedGit {
                        git: git.git,
                        tag: git.tag,
                        revision,
                    },
                );
                location
            }
        };
        let location = location
            .canonicalize()
            .with_context(|| format!("无法解析依赖 {name} 的路径 {}", location.display()))?;
        if !location.join("package.json").is_file() {
            bail!("依赖 {name} 缺少 package.json 源码入口描述");
        }
        resolved.insert(name, location);
    }
    if !next_lock.is_empty() || lock_path.is_file() {
        crate::plugins::write(&lock_path, &toml::to_string_pretty(&next_lock)?)?;
    }
    Ok(resolved)
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

fn checkout(git: &Git, locked: Option<&LockedGit>) -> Result<(PathBuf, String)> {
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
    let directory = packages.join(key);
    if directory.is_dir() {
        let (revision, tag) = git_revision(&directory, &git.tag)?;
        if revision == tag && locked.is_none_or(|lock| revision == lock.revision) {
            return Ok((directory, revision));
        }
        bail!("Git 缓存标签 {} 与 impasto.lock 不一致", git.tag);
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
    if revision != tag || locked.is_some_and(|lock| revision != lock.revision) {
        bail!("Git 标签 {} 与 impasto.lock 不一致", git.tag);
    }
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
