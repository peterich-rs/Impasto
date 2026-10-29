//! Generated `host.gen.ts` files must match the first-party HostModule table.

use std::path::PathBuf;

#[test]
fn host_stubs_match_generated_files() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for module in hostshell::first_party_modules() {
        let Some(path) = hostshell::host_stub_path(&root, module.specifier) else {
            panic!("no package folder for {}", module.specifier);
        };
        let current = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let fresh = hostshell::host_stub_source(&module);
        let current = format!("{}\n", current.trim_end());
        let fresh = format!("{}\n", fresh.trim_end());
        assert_eq!(
            current,
            fresh,
            "{} is stale; run `cargo run -p hostshell --bin host-stubs`",
            path.display()
        );
    }
}
