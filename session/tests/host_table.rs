//! The generated host-method table in `frameworks/modules` must match the
//! static contract snapshot in `host-table`.
//! `cargo run -p session --bin host-table` rewrites it.

use std::path::PathBuf;

#[test]
fn host_table_matches_generated_file() {
    let generated: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../frameworks/modules/src/host-table.gen.ts");
    let current = std::fs::read_to_string(&generated)
        .unwrap_or_else(|error| panic!("read {}: {error}", generated.display()));
    let mut fresh = std::process::Command::new(env!("CARGO_BIN_EXE_host-table"))
        .output()
        .expect("run host-table");
    fresh.stdout.extend_from_slice(b"\n");
    let fresh = String::from_utf8(fresh.stdout).expect("host-table prints UTF-8");
    let current = format!("{}\n", current.trim_end());
    let fresh = fresh.trim_end().to_string() + "\n";
    assert_eq!(
        current, fresh,
        "host-table.gen.ts is stale; run `cargo run -p session --bin host-table > frameworks/modules/src/host-table.gen.ts`"
    );
}
