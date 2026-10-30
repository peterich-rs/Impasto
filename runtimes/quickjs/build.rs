use std::path::{Path, PathBuf};

fn main() {
    let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") else {
        // Cargo always sets this. A missing value means the build is not Cargo.
        panic!("CARGO_MANIFEST_DIR is set by cargo");
    };
    let manifest = PathBuf::from(manifest_dir);
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
    generate_abi(&manifest, &out_dir);
    let Some(source) = manifest
        .parent()
        .and_then(|dir| dir.parent())
        .map(|root| root.join("third_party/quickjs"))
    else {
        panic!("runtimes/quickjs sits two directories under the workspace root");
    };
    println!("cargo::rustc-check-cfg=cfg(quickjs)");
    println!("cargo::rustc-check-cfg=cfg(quickjs_missing)");
    println!("cargo:rerun-if-changed=abi.yml");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/glue.c");
    println!("cargo:rerun-if-changed=src/runtime_host.inc");
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

fn generate_abi(manifest: &Path, out_dir: &Path) {
    let yml = std::fs::read_to_string(manifest.join("abi.yml")).expect("abi.yml");
    let mut section = "";
    let mut ops = Vec::new();
    let mut fields = Vec::new();
    for line in yml.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_suffix(':') {
            section = if name == "ops" { "ops" } else { "fields" };
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let pair = (name.trim().to_string(), value.trim().to_string());
        if section == "ops" {
            ops.push(pair);
        } else {
            fields.push(pair);
        }
    }
    let mut rust = String::from("// Generated from abi.yml. Do not edit.\n\n");
    rust.push_str("#[repr(u32)]\n#[derive(Clone, Copy, Debug, PartialEq, Eq)]\n#[allow(dead_code)]\npub enum MutationOp {\n");
    for (name, value) in &ops {
        rust.push_str(&format!("    {name} = {value},\n"));
    }
    rust.push_str("}\n\n#[repr(u32)]\n#[derive(Clone, Copy, Debug, PartialEq, Eq)]\n#[allow(dead_code)]\npub enum StyleField {\n");
    for (name, value) in &fields {
        rust.push_str(&format!("    {name} = {value},\n"));
    }
    rust.push_str("}\n");
    std::fs::write(out_dir.join("abi.rs"), &rust).expect("write abi.rs");

    let mut header = String::from("/* Generated from abi.yml. */\n#pragma once\n");
    for (name, value) in &ops {
        header.push_str(&format!(
            "#define MUTATION_OP_{} {}\n",
            name.to_uppercase(),
            value
        ));
    }
    for (name, value) in &fields {
        header.push_str(&format!(
            "#define STYLE_FIELD_{} {}\n",
            name.to_uppercase(),
            value
        ));
    }
    std::fs::write(out_dir.join("mutation.h"), header).expect("write mutation.h");

    let mut ts = String::from("// Generated from abi.yml. Do not edit.\n");
    ts.push_str("export const MutationOp = {\n");
    for (name, value) in &ops {
        ts.push_str(&format!("  {name}: {value},\n"));
    }
    ts.push_str("} as const;\n\nexport const StyleField = {\n");
    for (name, value) in &fields {
        ts.push_str(&format!("  {name}: {value},\n"));
    }
    ts.push_str("} as const;\n");
    let ts_path = manifest
        .parent()
        .and_then(|dir| dir.parent())
        .expect("workspace root")
        .join("frameworks/solid-renderer/src/mutation.gen.ts");
    if std::fs::read_to_string(&ts_path).ok().as_deref() != Some(ts.as_str()) {
        std::fs::write(&ts_path, ts).expect("write mutation.gen.ts");
    }
}
