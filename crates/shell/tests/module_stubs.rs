//! Generated `capability declarations` files must match the first-party Module table.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn module_stubs_match_generated_files() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for module in shell::first_party_modules() {
        let Some(path) = shell::module_stub_path(&root, module.specifier) else {
            panic!("no package folder for {}", module.specifier);
        };
        let current = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let fresh = shell::module_stub_source(&module);
        let current = format!("{}\n", current.trim_end());
        let fresh = format!("{}\n", fresh.trim_end());
        assert_eq!(
            current,
            fresh,
            "{} is stale; run `cargo run -p shell --bin module-stubs`",
            path.display()
        );
    }
}

#[test]
fn file_system_delete_is_reserved_safe() {
    let module = shell::first_party_modules()
        .into_iter()
        .find(|module| module.name == "FileSystem")
        .unwrap_or_else(|| panic!("FileSystem"));
    let source = shell::module_stub_source(&module);
    assert!(
        source.contains(
            "declare function deletePath(...args: unknown[]): Promise<unknown>; export { deletePath as delete };"
        ),
        "{source}"
    );
}

#[test]
fn module_declarations_parse_with_node_strip_types() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for module in shell::first_party_modules() {
        let Some(path) = shell::module_stub_path(&root, module.specifier) else {
            panic!("no package folder for {}", module.specifier);
        };
        let output = Command::new("node")
            .args([
                "--input-type=module",
                "-e",
                "import { readFileSync } from 'node:fs'; import { stripTypeScriptTypes } from 'node:module'; stripTypeScriptTypes(readFileSync(process.env.MODULE_DECLARATION, 'utf8'));",
            ])
            .env("MODULE_DECLARATION", &path)
            .env("NODE_NO_WARNINGS", "1")
            .output()
            .unwrap_or_else(|error| panic!("node: {error}"));
        assert!(
            output.status.success(),
            "{}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
