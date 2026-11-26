//! TypeScript 7 compiler discovery and verified installation.
use std::{
    env, fs,
    io::Read as _,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use sha2::{Digest as _, Sha512};

const TYPESCRIPT: &str = "7.0.2";

/// The npm package of [`TYPESCRIPT`]'s compiler for each host
/// (`@typescript/typescript-<name>`), and its tarball's `dist.integrity`.
const BUILDS: &[(&str, &str, &str, &str)] = &[
    (
        "macos",
        "aarch64",
        "darwin-arm64",
        "sha512-gowzar9MwS/aRWp6f3a4KUqzRjAZjOsmGNCM6LcTgXum+dBfgsBVMN+AgvOCCbguXyick6LJhpBszxMebJ8syA==",
    ),
    (
        "macos",
        "x86_64",
        "darwin-x64",
        "sha512-SZ9xZInqApNlNGc9s0W1VSsktYSOe9cFqNOIqmN1Gs8SmkjKZYFt017G4VwPxASInODuAdbTW7sXiFUf893RgA==",
    ),
    (
        "linux",
        "x86_64",
        "linux-x64",
        "sha512-EYdf2cNg7rgCWJnxCdJ+F3V39O8ihb37eHAu1LK8oAFizgTQbPOK7zHHXbPt8rX24COqODXeI3sIf0fCXG7H/A==",
    ),
    (
        "linux",
        "aarch64",
        "linux-arm64",
        "sha512-Qh4eU4/y3yDjnfjjyPYihMj5/ODIlmt+Bzu17OI+fiSRDW57QmU5SiN63exPRNJPKUzcc1INa1NXdrJ+MqHjUQ==",
    ),
    (
        "windows",
        "x86_64",
        "win32-x64",
        "sha512-0BQ3HkAHHlKLSp1qRvf3SUhGpGsDuhB/jgFw75guyqbxJqEaS0Cw/VFO8i2nHglJUzQCRtMMR/IBAKE3ETMC4g==",
    ),
    (
        "windows",
        "aarch64",
        "win32-arm64",
        "sha512-Gyl1Vy6OsWesLzmq+EP0Fb7b4Nid5232AvcA2SFcdYreldpNtYFFofPjnt62y9hQy7VTaZp65ICJjuAQRaVcIQ==",
    ),
];

/// The compiler's file in the package (`tsc.exe` on Windows).
const TSC: &str = if cfg!(windows) {
    "package/lib/tsc.exe"
} else {
    "package/lib/tsc"
};

pub(crate) fn executable() -> anyhow::Result<PathBuf> {
    if let Some(path) = env::var_os("IMPASTO_TSC") {
        return Ok(path.into());
    }
    if let Some(path) = crate::toolchain::distribution()
        .map(|root| root.join("typescript").join(TSC))
        .filter(|path| path.is_file())
    {
        return Ok(path);
    }
    if let Some(archive) = crate::toolchain::distribution()
        .map(|root| root.join("typescript.tgz"))
        .filter(|path| path.is_file())
    {
        let archive = fs::read(archive)?;
        let digest =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha512::digest(&archive));
        let dir = cache()?.join(format!("typescript-sdk-{digest}"));
        return install_archive(&archive, &dir);
    }
    let Some(&(_, _, name, integrity)) = BUILDS
        .iter()
        .find(|(os, arch, ..)| *os == env::consts::OS && *arch == env::consts::ARCH)
    else {
        bail!(
            "no TypeScript {TYPESCRIPT} build for {}-{}: pass --tsc <path to tsc>",
            env::consts::OS,
            env::consts::ARCH
        );
    };
    let dir = cache()?.join(format!("typescript-{TYPESCRIPT}-{name}"));
    let tsc = dir.join(TSC);
    if tsc.exists() {
        return Ok(tsc);
    }
    let url = format!(
        "https://registry.npmjs.org/@typescript/typescript-{name}/-/typescript-{name}-{TYPESCRIPT}.tgz"
    );
    eprintln!("impasto: 下载 TypeScript {TYPESCRIPT}");
    let mut tarball = Vec::new();
    let tls = ureq::tls::TlsConfig::builder()
        .unversioned_rustls_crypto_provider(rustls::crypto::aws_lc_rs::default_provider().into())
        .build();
    let agent = ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(std::time::Duration::from_secs(300)))
        .build()
        .new_agent();
    agent
        .get(&url)
        .call()
        .with_context(|| format!("download {url}"))?
        .into_body()
        .into_reader()
        .read_to_end(&mut tarball)
        .with_context(|| format!("download {url}"))?;
    verify(&tarball, integrity).with_context(|| url.clone())?;
    install_archive(&tarball, &dir).with_context(|| url)
}

fn install_archive(bytes: &[u8], dir: &Path) -> anyhow::Result<PathBuf> {
    let tsc = dir.join(TSC);
    if tsc.is_file() {
        return Ok(tsc);
    }
    let parent = dir.parent().context("invalid TypeScript cache path")?;
    fs::create_dir_all(parent)?;
    let partial = tempfile::tempdir_in(parent)?;
    tar::Archive::new(flate2::read::GzDecoder::new(bytes))
        .unpack(partial.path())
        .context("unpack TypeScript")?;
    if !partial.path().join(TSC).is_file() {
        bail!("TypeScript archive has no {TSC}");
    }
    if let Err(error) = fs::rename(partial.path(), dir)
        && !tsc.is_file()
    {
        return Err(error).context("install TypeScript compiler");
    }
    Ok(tsc)
}

pub(crate) fn write_archive(path: &Path) -> anyhow::Result<()> {
    let compiler = executable()?;
    let library = compiler
        .parent()
        .context("TypeScript checker has no library directory")?;
    let mut files = fs::read_dir(library)?.collect::<Result<Vec<_>, _>>()?;
    files.sort_unstable_by_key(fs::DirEntry::file_name);
    let compressed =
        flate2::write::GzEncoder::new(fs::File::create(path)?, flate2::Compression::best());
    let mut archive = tar::Builder::new(compressed);
    archive.append_path_with_name(&compiler, TSC)?;
    for entry in files {
        let path = entry.path();
        if entry.file_type()?.is_file()
            && path.extension().is_some_and(|extension| extension == "ts")
        {
            archive
                .append_path_with_name(path, Path::new("package/lib").join(entry.file_name()))?;
        }
    }
    if let Some(package) = library.parent() {
        for notice in [
            "LICENSE",
            "LICENSE.txt",
            "NOTICE.txt",
            "ThirdPartyNoticeText.txt",
        ] {
            let path = package.join(notice);
            if path.is_file() {
                archive.append_path_with_name(path, Path::new("package").join(notice))?;
            }
        }
    }
    archive.into_inner()?.finish()?;
    Ok(())
}

/// Checks `data` against an npm `dist.integrity` (`sha512-<base64>`).
fn verify(data: &[u8], integrity: &str) -> anyhow::Result<()> {
    let expected = integrity
        .strip_prefix("sha512-")
        .context("not a sha512 integrity")?;
    let actual = B64.encode(Sha512::digest(data));
    if actual != expected {
        bail!("integrity mismatch: expected {integrity}, got sha512-{actual}");
    }
    Ok(())
}

fn cache() -> anyhow::Result<PathBuf> {
    if let Some(dir) = env::var_os("IMPASTO_CACHE") {
        return Ok(PathBuf::from(dir));
    }
    let base = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .context("set IMPASTO_CACHE to locate the TypeScript compiler cache")?;
    Ok(PathBuf::from(base).join(".impasto/cache"))
}
