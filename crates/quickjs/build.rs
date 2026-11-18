use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    generate_abi(&manifest, &out_dir)?;
    println!("cargo:rerun-if-changed=abi.yml");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/scene.js");
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
    let mut ts = String::from("// Generated from abi.yml. Do not edit.\n");
    let mut js = String::from("/* Generated from abi.yml. */\nglobalThis.__abi = {\n");
    for (index, (section, items)) in sections.iter().enumerate() {
        let enum_name = enum_name(section);
        rust.push_str("#[repr(u32)]\n#[derive(Clone, Copy, Debug, PartialEq, Eq)]\n#[allow(dead_code, reason = \"generated from abi.yml\")]\n");
        rust.push_str(&format!("pub enum {enum_name} {{\n"));
        for (name, value) in items {
            rust.push_str(&format!("    {name} = {value},\n"));
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
    std::fs::write(out_dir.join("abi.rs"), rust)?;
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
