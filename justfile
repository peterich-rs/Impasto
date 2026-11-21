set shell := ["bash", "-euc"]

root := justfile_directory()
logo := root / "crates/embedder/macos/assets"

default:
    @just --list

# Same gate as the Linux clippy job in `.github/workflows/check.yml`.
clippy:
    cargo clippy --manifest-path "{{root}}/Cargo.toml" --workspace --exclude embedder-macos --all-targets -- -D warnings

check: style-check clippy test solid standard-check
    @echo "check ok"

bench:
    cargo test --manifest-path "{{root}}/Cargo.toml" --workspace --release -- --ignored

test: gallery-dist
    cargo test --manifest-path "{{root}}/Cargo.toml" --workspace

# CRAP and duplicate-form reports for the native product crates, then a
# warning-free cargo check. crapper and dryer must be on PATH. CRAP reads
# whatever `just coverage` last wrote; it does not rerun tests.
quality:
    crapper --use-existing-coverage protocol uicore engine session embedder
    dryer --threshold 0.8 crates/protocol/src crates/uicore/src crates/engine/src crates/session/src
    RUSTFLAGS="-D warnings" cargo check --manifest-path "{{root}}/Cargo.toml" --workspace

coverage:
    cargo llvm-cov --manifest-path "{{root}}/Cargo.toml" --lcov --output-path "{{root}}/target/coverage/rust/lcov.info" --workspace

gen:
    cargo run -p embedder --bin module-stubs
    cargo run -p uicore --bin style-types -- --root "{{root}}"

# Fail when generated style types or the utility list drift.
style-check:
    cargo run -p uicore --bin style-types -- --check --root "{{root}}"

solid:
    pnpm -C "{{root}}" install --frozen-lockfile
    pnpm -C "{{root}}" -r test

standard-check:
    node "{{root}}/tools/check-standard-library.mjs"

solid-app:
    pnpm -C "{{root}}" --filter @impasto/gallery-solid run build
    pnpm -C "{{root}}" --filter @impasto/gallery-solid run test:node
    pnpm -C "{{root}}" --filter @impasto/gallery-solid run test:esm

# Build the gallery ESM bundle when its sources or the lockfile change. The
# bundle is not committed; JavaScript UI tests fail with instructions when it is
# absent (`just test` builds it first).
gallery-dist:
    #!/usr/bin/env bash
    set -euo pipefail
    src="{{root}}/apps/gallery-solid"
    out="$src/dist/quotes.js"
    if [[ -f "$out" && ! "{{root}}/pnpm-lock.yaml" -nt "$out" ]]; then
      newer=$(find "{{root}}/packages" "{{root}}/tools" "$src" -type f \( -name '*.ts' -o -name '*.tsx' -o -name '*.mjs' \) \
        -not -path '*/node_modules/*' -not -path '*/dist/*' -not -name '*.test.ts' \
        -newer "$out" -print -quit)
      [[ -z "$newer" ]] && exit 0
    fi
    pnpm -C "$src" install && pnpm -C "$src" run build

[positional-arguments]
impeller *target:
    python3 "{{root}}/tools/impeller_sdk.py" prepare "$@"

[positional-arguments]
impeller-source *target:
    python3 "{{root}}/tools/impeller_sdk.py" build "$@"

host: gallery-dist
    #!/usr/bin/env bash
    set -euo pipefail
    sdk=$(python3 "{{root}}/tools/impeller_sdk.py" prepare)
    IMPASTO_IMPELLER_SDK_DIR="$sdk" cargo run --manifest-path "{{root}}/Cargo.toml" -p embedder-macos --features impeller -- --js "{{root}}/apps/gallery-solid/dist/quotes.js" "{{logo}}"

host-js: host

ios-build-check:
    cargo run --locked --manifest-path "{{root}}/Cargo.toml" -p cli -- build ios --ios-simulator --debug

[positional-arguments]
ios-sim-test *args:
    "{{root}}/tools/ios-sim-test.sh" "$@"

[positional-arguments]
ios-composition-test udid *args:
    "{{root}}/tools/ios-composition-test.sh" "$@"

# Debug APK. Gradle runs cargo build without --release.
# Needs the Android NDK and both Android Rust targets.
android: gallery-dist
    "{{root}}/crates/embedder/android/apk/gradlew" -p "{{root}}/crates/embedder/android/apk" :app:assembleDebug

android-build-check: gallery-dist
    "{{root}}/crates/embedder/android/apk/gradlew" -p "{{root}}/crates/embedder/android/apk" :app:assembleDebug :app:assembleDebugAndroidTest :app:lintDebug
    python3 "{{root}}/tools/check-android-package.py" "{{root}}/crates/embedder/android/apk/app/build/outputs/apk/debug/app-debug.apk"

# Run on one Android device and remove its temporary test packages on exit.
android-device-test: gallery-dist
    #!/usr/bin/env bash
    set -euo pipefail
    serial="$(adb get-serialno)"
    cleanup() {
        local status=$?
        local installed
        trap - EXIT
        if ! installed="$(adb -s "$serial" shell pm list packages)"; then
            echo "无法连接 $serial，未能清理 Android 测试包" >&2
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
android-release: gallery-dist
    "{{root}}/crates/embedder/android/apk/gradlew" -p "{{root}}/crates/embedder/android/apk" -PimpastoLocalSigning=true :app:assembleRelease

# Relocatable CLI distribution, including the pinned Node/compiler toolchain.
cli-dist:
    node "{{root}}/tools/distribute.mjs"

# Identical gallery click and wheel scenarios before/after the native toolchain.
toolchain-baseline: gallery-dist
    python3 "{{root}}/tools/toolchain-baseline.py"
