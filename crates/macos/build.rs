fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_FEATURE_IMPELLER").is_some() {
        println!("cargo::rustc-link-arg-bin=macos=-Wl,-rpath,@executable_path/../Frameworks");
    }
}
