fn main() {
    println!("cargo:rerun-if-changed=src/diagnostics/signpost.c");
    if std::env::var("CARGO_CFG_TARGET_VENDOR").as_deref() == Ok("apple") {
        cc::Build::new()
            .file("src/diagnostics/signpost.c")
            .compile("impasto_signpost");
    }
}
