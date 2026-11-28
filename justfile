set shell := ["bash", "-euc"]

root := justfile_directory()
logo := root / "crates/embedder/macos/assets"

default:
    @just --list

# Same gate as the Linux clippy job in `.github/workflows/check.yml`.
clippy:
    cargo clippy --manifest-path "{{root}}/Cargo.toml" --workspace --exclude embedder-macos --all-targets -- -D warnings

check: style-check boundaries clippy test solid standard-check
    @echo "check ok"

bench:
    cargo test --locked --manifest-path "{{root}}/Cargo.toml" -p raster --lib --release -- bench_ --ignored --nocapture --test-threads=1

bench-wave:
    cargo test --locked --manifest-path "{{root}}/Cargo.toml" -p raster --lib --release -- tests::p0::wave::bench_ --ignored --nocapture --test-threads=1

test:
    cargo test --manifest-path "{{root}}/Cargo.toml" --workspace

# CRAP and duplicate-form reports for the native product crates, then a
# warning-free cargo check. crapper and dryer must be on PATH. CRAP reads
# whatever `just coverage` last wrote; it does not rerun tests.
quality:
    crapper --use-existing-coverage engine raster embedder
    dryer --threshold 0.8 crates/engine/src crates/raster/src crates/embedder
    RUSTFLAGS="-D warnings" cargo check --manifest-path "{{root}}/Cargo.toml" --workspace

coverage:
    cargo llvm-cov --manifest-path "{{root}}/Cargo.toml" --lcov --output-path "{{root}}/target/coverage/rust/lcov.info" --workspace

gen:
    cargo run -p engine --bin style-types -- --root "{{root}}"

# Fail when generated style types or the utility list drift.
style-check:
    cargo run -p engine --bin style-types -- --check --root "{{root}}"

solid:
    cargo run --locked --manifest-path "{{root}}/Cargo.toml" -p cli -- test --framework

boundaries:
    python3 "{{root}}/tools/check-javascript-boundaries.py"

standard-check:
    cargo test --locked --manifest-path "{{root}}/Cargo.toml" -p javascript --test standard_contract
    cargo run --locked --manifest-path "{{root}}/Cargo.toml" -p cli -- analyze --standard

solid-app: gallery-dist
    cargo run --locked --manifest-path "{{root}}/Cargo.toml" -p cli -- --project "{{root}}/examples/gallery-solid" test

gallery-tests: gallery-dist
    cargo test --locked --manifest-path "{{root}}/Cargo.toml" -p engine --features gallery-tests js::tests::gallery_tests
    cargo test --locked --manifest-path "{{root}}/Cargo.toml" -p cli --bin impasto gallery_patch -- --ignored

# Build the standalone Gallery ESM for the example workflow.
gallery-dist:
    cargo run --locked --manifest-path "{{root}}/Cargo.toml" -p cli -- pack "{{root}}/examples/gallery-solid/src/quotes.tsx" "{{root}}/examples/gallery-solid/dist/quotes.js"

host: gallery-dist
    cargo run --manifest-path "{{root}}/Cargo.toml" -p embedder-macos -- --title "Impasto Gallery" --js "{{root}}/examples/gallery-solid/dist/quotes.js" "{{logo}}"

host-js: host

ios-build-check:
    cargo run --locked --manifest-path "{{root}}/Cargo.toml" -p cli -- --project "{{root}}/examples/gallery-solid" build ios --ios-simulator --debug

[positional-arguments]
ios-sim-test *args:
    "{{root}}/tools/ios-sim-test.sh" "$@"

[positional-arguments]
ios-composition-test udid *args:
    "{{root}}/tools/ios-composition-test.sh" "$@"

# Debug APK. Gradle runs cargo build without --release.
# Needs the Android NDK and both Android Rust targets.
android:
    "{{root}}/crates/embedder/android/apk/gradlew" -p "{{root}}/crates/embedder/android/apk" :app:assembleDebug

android-build-check:
    "{{root}}/crates/embedder/android/apk/gradlew" -p "{{root}}/crates/embedder/android/apk" :app:assembleDebug :app:lintDebug
    ABI_LIST="${ORG_GRADLE_PROJECT_impastoAbis:-}" python3 "{{root}}/tools/check-android-package.py" "{{root}}/crates/embedder/android/apk/app/build/outputs/apk/debug/app-debug.apk"

# Local verification on one Android device or emulator with test package cleanup.
android-device-test:
    #!/usr/bin/env bash
    set -euo pipefail
    serial="$(adb get-serialno)"
    cleanup() {
        local status=$?
        local installed
        trap - EXIT
        if ! installed="$(adb -s "$serial" shell pm list packages)"; then
            echo "无法连接 ${serial}，未能清理 Android 测试包" >&2
            exit 1
        fi
        for package in dev.impasto.host.verification.test dev.impasto.host.verification; do
            if grep -Fxq "package:$package" <<< "$installed"; then
                adb -s "$serial" uninstall "$package" || status=1
            fi
        done
        exit "$status"
    }
    trap cleanup EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    ANDROID_SERIAL="$serial" "{{root}}/crates/embedder/android/apk/gradlew" -p "{{root}}/crates/embedder/android/apk" \
        -PimpastoVerification=true -Pandroid.testInstrumentationRunnerArguments.impastoTestVulkan="${IMPASTO_TEST_VULKAN:-true}" \
        :app:connectedDebugAndroidTest

# Release APK. Gradle runs cargo build --release. Signed with the debug keystore.
android-release:
    "{{root}}/crates/embedder/android/apk/gradlew" -p "{{root}}/crates/embedder/android/apk" -PimpastoLocalSigning=true :app:assembleRelease

# Relocatable CLI, framework sources and pinned native TypeScript.
cli-dist:
    cargo run --locked --release --manifest-path "{{root}}/Cargo.toml" -p cli -- sdk-dist "{{root}}/target/cli-dist"

# Identical gallery click and wheel scenarios before/after the native toolchain.
toolchain-baseline *args: gallery-dist
    python3 "{{root}}/tools/toolchain-baseline.py" {{args}}

# Check the standalone macOS DevTools client and its generated native bindings.
devtools-desktop:
    #!/usr/bin/env bash
    set -euo pipefail
    app="{{root}}/examples/devtools-desktop"
    cargo run --locked --manifest-path "{{root}}/Cargo.toml" -p cli -- --project "$app" sync --platform macos
    CARGO_TARGET_DIR="{{root}}/target/devtools-core" cargo clippy --manifest-path "$app/rust/Cargo.toml" --config "$app/.impasto/cargo/.cargo/config.toml" --config "$app/.impasto/cargo/Cargo.toml" --all-targets -- -D warnings
    CARGO_TARGET_DIR="{{root}}/target/devtools-core" cargo test --manifest-path "$app/rust/Cargo.toml" --config "$app/.impasto/cargo/.cargo/config.toml" --config "$app/.impasto/cargo/Cargo.toml"
    cargo run --locked --manifest-path "{{root}}/Cargo.toml" -p cli -- --project "$app" analyze --platform macos --deny-warnings
