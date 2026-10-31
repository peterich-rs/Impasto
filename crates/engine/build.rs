use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let sdk = sdk_dir(&manifest);
    let header = sdk.join("include/impeller.h");
    println!("cargo:rerun-if-changed={}", header.display());
    println!("cargo:rustc-check-cfg=cfg(impeller)");
    println!("cargo:rustc-check-cfg=cfg(impeller_missing)");
    let out = PathBuf::from(env::var("OUT_DIR")?);
    if !header.is_file() {
        println!("cargo:rustc-cfg=impeller_missing");
        return Ok(());
    }
    println!("cargo:rustc-cfg=impeller");
    let lib = sdk.join("lib");
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=impeller");
    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("android") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
    }

    let header_text = fs::read_to_string(&header)?;
    let version = impeller_version(&header_text)?;
    fs::write(
        out.join("impeller_version.rs"),
        format!("pub const IMPELLER_VERSION: u32 = {version};\n"),
    )?;

    let bindings = bindgen::Builder::default()
        .header(header.to_string_lossy())
        .allowlist_function("Impeller.*")
        .allowlist_type("Impeller.*")
        .allowlist_var("kImpeller.*")
        .default_enum_style(bindgen::EnumVariation::ModuleConsts)
        .generate_comments(false)
        .layout_tests(false)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()?;
    bindings.write_to_file(out.join("impeller.rs"))?;
    Ok(())
}

fn sdk_dir(manifest: &Path) -> PathBuf {
    let target = env::var("TARGET").unwrap_or_default();
    manifest
        .join("../../third_party/impeller-sdk")
        .join(platform_name(&target))
}

fn platform_name(target: &str) -> &'static str {
    if target.contains("android") {
        if target.starts_with("x86_64") {
            "android-x64"
        } else if target.starts_with("i686") {
            "android-x86"
        } else if target.starts_with("armv7") {
            "android-arm"
        } else {
            "android-arm64"
        }
    } else if target.contains("apple-ios") {
        "ios-arm64"
    } else if target.starts_with("aarch64") || target.starts_with("arm64") {
        "darwin-arm64"
    } else {
        "darwin-x64"
    }
}

fn impeller_version(header: &str) -> Result<u32, String> {
    let variant = define(header, "IMPELLER_VERSION_VARIANT")?;
    let major = define(header, "IMPELLER_VERSION_MAJOR")?;
    let minor = define(header, "IMPELLER_VERSION_MINOR")?;
    let patch = define(header, "IMPELLER_VERSION_PATCH")?;
    Ok((variant << 29) | (major << 22) | (minor << 12) | patch)
}

fn define(header: &str, name: &str) -> Result<u32, String> {
    for line in header.lines() {
        let mut parts = line.split_whitespace();
        if parts.next() == Some("#define") && parts.next() == Some(name) {
            let value = parts.next().unwrap_or("0");
            return value
                .parse()
                .map_err(|_| format!("bad {name} in impeller.h"));
        }
    }
    Err(format!("missing {name} in impeller.h"))
}
