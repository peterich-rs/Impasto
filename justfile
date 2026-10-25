set shell := ["bash", "-euc"]

root := justfile_directory()
counter := root / "protocol/tests/fixtures/counter.bin"
logo := root / "hosts/macos/assets"

default:
    @just --list

check: test kotlin solid
    @echo "uifw check ok"

test:
    cargo test --manifest-path "{{root}}/Cargo.toml" --workspace

kotlin:
    "{{root}}/frameworks/kotlin-ui/gradlew" -p "{{root}}/frameworks/kotlin-ui" :library:jvmTest :gallery:runGallery

solid:
    UIFW_COUNTER_BYTES="{{counter}}" node --experimental-strip-types "{{root}}/frameworks/solid-renderer/src/encode.test.ts"
    node --conditions=browser --experimental-strip-types "{{root}}/frameworks/solid-renderer/src/tree.test.ts"
    node --experimental-strip-types "{{root}}/frameworks/solid-renderer/runtime/frame.test.ts"
    node --experimental-strip-types "{{root}}/apps/gallery-solid/gallery.ts"

solid-app:
    cd "{{root}}/apps/gallery-solid" && pnpm install && node build.mjs && node --experimental-strip-types counter.node.test.ts

host:
    cargo run --manifest-path "{{root}}/Cargo.toml" -p macos -- "{{counter}}" "{{logo}}"

quickjs:
    "{{root}}/runtimes/quickjs/fetch-quickjs.sh"

host-js:
    cargo run --manifest-path "{{root}}/Cargo.toml" -p macos -- --js "{{root}}/apps/gallery-solid/dist/counter.js" "{{logo}}"

# Local pixel compare. Needs the Impeller SDK. Not part of `just check`.
pixel-test:
    UIFW_DUMP=/tmp/uifw-counter-frame.png cargo run --manifest-path "{{root}}/Cargo.toml" -p macos -- "{{counter}}" "{{logo}}"
    UIFW_FRAME=/tmp/uifw-counter-frame.png cargo test --manifest-path "{{root}}/Cargo.toml" -p macos -- --ignored pixels::tests::counter_frame_matches_the_reference --exact

# Cargo cdylib, then the APK. Needs the Android NDK and the arm64 Impeller SDK.
android:
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
    assets="{{root}}/hosts/android/apk/app/src/main/assets"
    jni="{{root}}/hosts/android/apk/app/src/main/jniLibs/arm64-v8a"
    rm -rf "$assets" "$jni"
    mkdir -p "$assets/fonts" "$assets/images" "$jni"
    cp "{{root}}/hosts/android/assets/fonts/UIFWSans.ttf" "$assets/fonts/UIFWSans.ttf"
    cp "{{root}}/hosts/macos/assets/logo.png" "$assets/images/logo.png"
    cp "{{root}}/apps/gallery-solid/dist/counter.js" "$assets/counter.js"
    cp "{{root}}/target/aarch64-linux-android/release/libuifw.so" "$jni/libuifw.so"
    cp "{{root}}/third_party/impeller-sdk/android-arm64/lib/libimpeller.so" "$jni/libimpeller.so"
    "{{root}}/hosts/android/apk/gradlew" -p "{{root}}/hosts/android/apk" assembleDebug
