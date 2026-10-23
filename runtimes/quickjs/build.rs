use std::path::PathBuf;

fn main() {
    let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") else {
        // Cargo always sets this. A missing value means the build is not Cargo.
        panic!("CARGO_MANIFEST_DIR is set by cargo");
    };
    let manifest = PathBuf::from(manifest_dir);
    let Some(source) = manifest
        .parent()
        .and_then(|dir| dir.parent())
        .map(|root| root.join("third_party/quickjs"))
    else {
        panic!("runtimes/quickjs sits two directories under the workspace root");
    };
    println!("cargo::rustc-check-cfg=cfg(quickjs)");
    println!("cargo::rustc-check-cfg=cfg(quickjs_missing)");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/glue.c");
    println!("cargo:rerun-if-changed={}", source.display());
    if !source.join("quickjs.c").is_file() {
        println!("cargo:rustc-cfg=quickjs_missing");
        return;
    }
    println!("cargo:rustc-cfg=quickjs");
    cc::Build::new()
        .include(&source)
        .define("_GNU_SOURCE", None)
        .file(source.join("dtoa.c"))
        .file(source.join("libregexp.c"))
        .file(source.join("libunicode.c"))
        .file(source.join("quickjs.c"))
        .file(manifest.join("src/glue.c"))
        .warnings(false)
        .compile("quickjs_ng");
    println!("cargo:rustc-link-lib=m");
}
