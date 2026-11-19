use std::env;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo::rerun-if-changed=sdk.lock");
    println!("cargo::rerun-if-env-changed=IMPASTO_IMPELLER_SDK_DIR");
    if env::var_os("CARGO_FEATURE_LINK").is_none() {
        return Ok(());
    }

    let target = env::var("TARGET")?;
    let os = env::var("CARGO_CFG_TARGET_OS")?;
    let version = locked("version")?;
    let revision = locked("source")?;
    let sdk = match env::var_os("IMPASTO_IMPELLER_SDK_DIR") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("missing crate directory")?)
            .join("../../third_party/impeller-sdk")
            .join(version)
            .join(&target),
    };
    let marker = sdk.join("sdk.meta");
    println!("cargo::rerun-if-changed={}", marker.display());
    let metadata = fs::read_to_string(&marker).map_err(|error| {
        format!("missing Impeller SDK for {target} ({error}). Run `just impeller {target}`")
    })?;
    if !metadata.lines().eq([version, revision, &target]) {
        return Err(format!(
            "Impeller SDK at {} does not match {version} for {target}",
            sdk.display()
        )
        .into());
    }

    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("missing build output directory")?);
    let libraries = sdk.join("lib");
    let names: &[&str] = match os.as_str() {
        "macos" => &["libimpeller.dylib"],
        "android" | "linux" => &["libimpeller.so"],
        "windows" if env::var("CARGO_CFG_TARGET_ENV")? == "msvc" => {
            &["impeller.lib", "impeller.dll"]
        }
        "ios" => &["libimpeller.a"],
        _ => return Err(format!("Impeller SDK linking is unavailable for {target}").into()),
    };
    for name in names {
        let library = libraries.join(name);
        println!("cargo::rerun-if-changed={}", library.display());
        if !library.is_file() {
            return Err(format!("missing Impeller library {}", library.display()).into());
        }
    }

    if os == "ios" {
        println!("cargo::rustc-link-search=native={}", libraries.display());
        println!("cargo::rustc-link-lib=static=impeller");
        let links = sdk.join("link.txt");
        println!("cargo::rerun-if-changed={}", links.display());
        for library in fs::read_to_string(links)?.lines() {
            println!("cargo::rustc-link-lib={library}");
        }
    } else {
        // Cargo makes native search paths under OUT_DIR available to run and test.
        let staged = out.join("lib");
        fs::create_dir_all(&staged)?;
        for name in names {
            fs::copy(libraries.join(name), staged.join(name))?;
        }
        println!("cargo::rustc-link-search=native={}", staged.display());
        println!("cargo::rustc-link-lib=dylib=impeller");
    }
    Ok(())
}

fn locked(key: &str) -> Result<&'static str, String> {
    include_str!("sdk.lock")
        .lines()
        .filter_map(|line| line.split_once(' '))
        .find_map(|(field, value)| (field == key).then_some(value))
        .ok_or_else(|| format!("missing {key} in Impeller sdk.lock"))
}
