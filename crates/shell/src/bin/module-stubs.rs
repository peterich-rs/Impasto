//! Write per-package `capability declarations` from the first-party Module table.

fn main() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if let Err(error) = shell::write_module_stubs(&root) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
