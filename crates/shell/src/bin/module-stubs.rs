//! Write per-package `capability declarations` from the first-party Module table.

fn main() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut args = std::env::args_os().skip(1);
    let result = match (args.next(), args.next()) {
        (None, None) => shell::write_module_stubs(&root),
        (Some(mode), None) if mode == "--check" => shell::check_module_stubs(&root),
        _ => Err("usage: module-stubs [--check]".into()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
