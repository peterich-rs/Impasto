//! Write or check the generated style types.
//!
//! `cargo run -p uicore --bin style-types` writes the TypeScript module, the C
//! key table, and the utility list. `--check` fails when a file differs.
//! Cargo builds do not rewrite those sources.

use std::path::{Path, PathBuf};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut check = false;
    let mut root = PathBuf::from(env_root());
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--check" => check = true,
            "--root" => {
                root = PathBuf::from(args.next().unwrap_or_else(|| {
                    eprintln!("style-types: --root needs a path");
                    std::process::exit(1);
                }));
            }
            other => {
                eprintln!("style-types: unknown argument {other}");
                std::process::exit(1);
            }
        }
    }
    let files = [
        (
            root.join("packages/solid-renderer/src/style.gen.ts"),
            uicore::style::emit_typescript(),
        ),
        (
            root.join("crates/quickjs/src/style.gen.h"),
            uicore::style::emit_c_header(),
        ),
        (
            root.join("docs/style-utilities.gen.md"),
            uicore::style::emit_utilities(),
        ),
    ];
    let mut failed = false;
    for (path, text) in files {
        if check {
            failed |= !matches_file(&path, &text);
        } else if let Err(error) = write_file(&path, &text) {
            eprintln!("style-types: {path:?}: {error}");
            failed = true;
        }
    }
    if failed {
        std::process::exit(1);
    }
}

fn env_root() -> String {
    std::env::var("CARGO_MANIFEST_DIR")
        .ok()
        .and_then(|dir| {
            Path::new(&dir)
                .parent()
                .and_then(|dir| dir.parent())
                .map(|dir| dir.display().to_string())
        })
        .unwrap_or_else(|| ".".to_owned())
}

fn matches_file(path: &Path, text: &str) -> bool {
    match std::fs::read_to_string(path) {
        Ok(current) if current == text => true,
        Ok(_) => {
            eprintln!("style-types: {} is out of date", path.display());
            false
        }
        Err(error) => {
            eprintln!("style-types: {}: {error}", path.display());
            false
        }
    }
}

fn write_file(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)
}
