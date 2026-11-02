set shell := ["bash", "-euc"]

root := justfile_directory()
logo := root / "crates/macos/assets"

default:
    @just --list

# Same gate as the Linux clippy job in `.github/workflows/check.yml`.
clippy:
    cargo clippy --manifest-path "{{root}}/Cargo.toml" --workspace --exclude macos --all-targets -- -D warnings

check: clippy test kotlin solid
    @echo "check ok"

bench:
    cargo test --manifest-path "{{root}}/Cargo.toml" --workspace --release -- --ignored

test: gallery-dist
    cargo test --manifest-path "{{root}}/Cargo.toml" --workspace

# CRAP and duplicate-form reports for the native product crates, then a
# warning-free cargo check. crapper and dryer must be on PATH. CRAP reads
# whatever `just coverage` last wrote; it does not rerun tests.
quality:
    crapper --use-existing-coverage protocol uicore engine session hostshell
    dryer --threshold 0.8 crates/protocol/src crates/uicore/src crates/engine/src crates/session/src
    RUSTFLAGS="-D warnings" cargo check --manifest-path "{{root}}/Cargo.toml" --workspace

coverage:
    cargo llvm-cov --manifest-path "{{root}}/Cargo.toml" --lcov --output-path "{{root}}/target/coverage/rust/lcov.info" --workspace

gen:
    cargo run -p hostshell --bin host-stubs

kotlin:
    "{{root}}/kotlin/gradlew" -p "{{root}}/kotlin" :library:jvmTest

solid:
    pnpm -C "{{root}}" install --frozen-lockfile
    pnpm -C "{{root}}" -r test

solid-app:
    pnpm -C "{{root}}" --filter @impasto/gallery-solid run build
    pnpm -C "{{root}}" --filter @impasto/gallery-solid run test:node
    pnpm -C "{{root}}" --filter @impasto/gallery-solid run test:esm

# Build the gallery ESM bundle from the TSX sources when sources changed. The
# bundle is not committed; quickjs tests fail with instructions when it is
# absent (`just test` builds it first).
gallery-dist:
    #!/usr/bin/env bash
    set -euo pipefail
    src="{{root}}/apps/gallery-solid"
    out="$src/dist/counter.js"
    if [[ -f "$out" ]]; then
      newer=$(find "{{root}}/packages" "{{root}}/tools" "$src" -type f \( -name '*.ts' -o -name '*.tsx' -o -name '*.mjs' \) \
        -not -path '*/node_modules/*' -not -path '*/dist/*' -not -name '*.test.ts' \
        -newer "$out" -print -quit)
      [[ -z "$newer" ]] && exit 0
    fi
    pnpm -C "$src" install && pnpm -C "$src" run build

host: gallery-dist
    cargo run --manifest-path "{{root}}/Cargo.toml" -p macos -- --js "{{root}}/apps/gallery-solid/dist/counter.js" "{{logo}}"

quickjs:
    "{{root}}/crates/quickjs/fetch-quickjs.sh"

host-js: host

# Cargo cdylib, then the APK. Needs the Android NDK and the arm64 Impeller SDK.
android: gallery-dist
    #!/usr/bin/env bash
    set -euo pipefail
    ndk="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
    if [[ -z "$ndk" && -n "${ANDROID_HOME:-}" && -d "${ANDROID_HOME}/ndk" ]]; then
      ndk="$(find "${ANDROID_HOME}/ndk" -mindepth 1 -maxdepth 1 -type d | sort | tail -1)"
    fi
    if [[ -z "$ndk" ]]; then
      echo "ANDROID_NDK_HOME is not set" >&2
      exit 1
    fi
    export ANDROID_NDK_HOME="$ndk"
    prebuilt="$(find "$ndk/toolchains/llvm/prebuilt" -mindepth 2 -maxdepth 2 -type d -name bin | head -1)"
    export PATH="$prebuilt:$PATH"
    export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$prebuilt/aarch64-linux-android24-clang"
    cargo build --manifest-path "{{root}}/Cargo.toml" --release --target aarch64-linux-android -p android
    assets="{{root}}/crates/android/apk/app/src/main/assets"
    jni="{{root}}/crates/android/apk/app/src/main/jniLibs/arm64-v8a"
    rm -rf "$assets" "$jni"
    mkdir -p "$assets/fonts" "$assets/images" "$jni"
    cp "{{root}}/crates/android/assets/fonts/Sans.ttf" "$assets/fonts/Sans.ttf"
    cp "{{root}}/crates/macos/assets/logo.png" "$assets/images/logo.png"
    cp "{{root}}/apps/gallery-solid/dist/counter.js" "$assets/counter.js"
    cp "{{root}}/target/aarch64-linux-android/release/libhost.so" "$jni/libhost.so"
    cp "{{root}}/third_party/impeller-sdk/android-arm64/lib/libimpeller.so" "$jni/libimpeller.so"
    "{{root}}/crates/android/apk/gradlew" -p "{{root}}/crates/android/apk" assembleDebug
