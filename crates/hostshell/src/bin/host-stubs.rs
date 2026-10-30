//! Write per-package `host.gen.ts` from the first-party HostModule table.

fn main() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if let Err(error) = hostshell::write_host_stubs(&root) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
