fn main() -> std::process::ExitCode {
    #[cfg(feature = "bundled-plugins")]
    if let Err(error) = embedder::install_bundled_plugins(impasto_bundled_plugins::register) {
        eprintln!("{error}");
        return std::process::ExitCode::FAILURE;
    }
    embedder_macos::main()
}
