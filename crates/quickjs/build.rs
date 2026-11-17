use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    generate_abi(&manifest, &out_dir)?;
    let scene = std::fs::read_to_string(manifest.join("src/scene.js"))?;
    let glue = std::fs::read_to_string(manifest.join("src/glue.c"))?;
    let mut exports = std::collections::BTreeSet::new();
    for line in scene.lines() {
        if let Some(rest) = line.trim().strip_prefix("core.")
            && let Some((name, _)) = rest.split_once(" = function")
        {
            exports.insert(name.to_owned());
        }
    }
    for line in glue
        .lines()
        .filter(|line| line.contains("bind_fn(ctx, core,"))
    {
        if let Some(name) = line.split('"').nth(1).filter(|name| !name.starts_with('_')) {
            exports.insert(name.to_owned());
        }
    }
    for name in [
        "scene",
        "createElement",
        "createTextNode",
        "insertNode",
        "setProp",
        "effect",
        "memo",
        "createComponent",
    ] {
        exports.insert(name.to_owned());
    }
    let names = exports
        .iter()
        .map(|name| format!("\"{name}\",\n"))
        .collect::<String>();
    std::fs::write(out_dir.join("scene_exports.inc"), names)?;
    for file in ["src/scene.js", "src/reactive.js"] {
        println!("cargo:rerun-if-changed={file}");
    }

    let source = manifest
        .parent()
        .and_then(|dir| dir.parent())
        .map(|root| root.join("third_party/quickjs"))
        .ok_or("crates/quickjs sits two directories under the workspace root")?;
    println!("cargo::rustc-check-cfg=cfg(quickjs)");
    println!("cargo::rustc-check-cfg=cfg(quickjs_missing)");
    println!("cargo:rerun-if-changed=abi.yml");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/glue.c");
    println!("cargo:rerun-if-changed=src/modules.inc");
    println!("cargo:rerun-if-changed={}", source.display());
    if !source.join("quickjs.c").is_file() {
        println!("cargo:rustc-cfg=quickjs_missing");
        return Ok(());
    }
    println!("cargo:rustc-cfg=quickjs");
    cc::Build::new()
        .include(&source)
        .include(&out_dir)
        .define("_GNU_SOURCE", None)
        .file(source.join("dtoa.c"))
        .file(source.join("libregexp.c"))
        .file(source.join("libunicode.c"))
        .file(source.join("quickjs.c"))
        .file(manifest.join("src/glue.c"))
        .warnings(false)
        .compile("quickjs_ng");
    println!("cargo:rustc-link-lib=m");
    Ok(())
}

fn enum_name(section: &str) -> &'static str {
    match section {
        "ops" => "MutationOp",
        "style_value_tags" => "StyleValueTag",
        "fields" => "StyleField",
        "creates" => "CreateKind",
        "queries" => "QueryKind",
        "effects" => "EffectKind",
        _ => "AbiValue",
    }
}

fn generate_abi(manifest: &Path, out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let yml = std::fs::read_to_string(manifest.join("abi.yml"))?;
    let mut section = String::new();
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in yml.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_suffix(':') {
            section = name.to_string();
            sections.push((section.clone(), Vec::new()));
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let Some((_, items)) = sections.iter_mut().find(|(key, _)| key == &section) else {
            continue;
        };
        items.push((name.trim().to_string(), value.trim().to_string()));
    }
    let mut rust = String::from("// Generated from abi.yml. Do not edit.\n\n");
    let mut header = String::from("/* Generated from abi.yml. */\n#pragma once\n");
    let mut ts = String::from("// Generated from abi.yml. Do not edit.\n");
    let mut js = String::from("/* Generated from abi.yml. */\nglobalThis.__abi = {\n");
    for (index, (section, items)) in sections.iter().enumerate() {
        let enum_name = enum_name(section);
        rust.push_str("#[repr(u32)]\n#[derive(Clone, Copy, Debug, PartialEq, Eq)]\n#[allow(dead_code, reason = \"generated from abi.yml\")]\n");
        rust.push_str(&format!("pub enum {enum_name} {{\n"));
        for (name, value) in items {
            rust.push_str(&format!("    {name} = {value},\n"));
            let prefix = enum_name
                .chars()
                .flat_map(|ch| {
                    if ch.is_uppercase() {
                        vec!['_', ch]
                    } else {
                        vec![ch.to_ascii_uppercase()]
                    }
                })
                .collect::<String>();
            let prefix = prefix.trim_start_matches('_');
            header.push_str(&format!(
                "#define {}_{} {}\n",
                prefix,
                name.to_uppercase(),
                value
            ));
        }
        rust.push_str("}\n\n");
        let export = if section == "ops" {
            "MutationOp"
        } else if section == "fields" {
            "StyleField"
        } else {
            enum_name
        };
        ts.push_str(&format!("export const {export} = {{\n"));
        js.push_str(&format!("  {export}: {{\n"));
        for (name, value) in items {
            ts.push_str(&format!("  {name}: {value},\n"));
            js.push_str(&format!("    {name}: {value},\n"));
        }
        ts.push_str("} as const;\n");
        js.push_str("  }");
        if index + 1 != sections.len() {
            ts.push('\n');
            js.push_str(",\n");
        }
    }
    js.push_str("\n};\n");
    rust.push_str("#[repr(C)]\n#[derive(Clone, Copy)]\npub struct StyleValueRecord { pub tag: u32, pub children: u32, pub number: f64, pub key: *const std::ffi::c_char, pub key_len: usize, pub text: *const std::ffi::c_char, pub text_len: usize }\n");
    header.push_str("typedef struct StyleValueRecord { uint32_t tag; uint32_t children; double number; const char *key; size_t key_len; const char *text; size_t text_len; } StyleValueRecord;\n");
    std::fs::write(out_dir.join("abi.rs"), rust)?;
    std::fs::write(out_dir.join("mutation.h"), header)?;
    std::fs::write(out_dir.join("abi.js"), js)?;
    let ts_path = manifest
        .parent()
        .and_then(|dir| dir.parent())
        .ok_or("workspace root")?
        .join("packages/solid-renderer/src/mutation.gen.ts");
    if std::fs::read_to_string(&ts_path).ok().as_deref() != Some(ts.as_str()) {
        std::fs::write(&ts_path, ts)?;
    }
    Ok(())
}
