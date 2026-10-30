use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let target = std::env::var("TARGET").unwrap_or_default();
    let platform = if target.starts_with("aarch64") || target.starts_with("arm64") {
        "darwin-arm64"
    } else {
        "darwin-x64"
    };
    let lib = manifest
        .join("../../third_party/impeller-sdk")
        .join(platform)
        .join("lib");
    if lib.join("libimpeller.dylib").is_file() {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
    }
}
