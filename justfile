set shell := ["bash", "-euc"]

root := justfile_directory()
logo := root / "hosts/macos/assets"

default:
    @just --list

check: test kotlin solid pixel
    @echo "uifw check ok"

pixel:
    @if [[ ! -d "{{root}}/third_party/impeller-sdk/darwin-arm64" || ! -d "{{root}}/third_party/quickjs" ]]; then echo "pixel skipped"; exit 0; fi
    cargo test --manifest-path "{{root}}/Cargo.toml" -p macos --test pixels

bench:
    cargo test --manifest-path "{{root}}/Cargo.toml" --workspace --release -- --ignored

test:
    cargo test --manifest-path "{{root}}/Cargo.toml" --workspace

kotlin:
    "{{root}}/frameworks/kotlin-ui/gradlew" -p "{{root}}/frameworks/kotlin-ui" :library:jvmTest

solid:
    node --conditions=browser --experimental-strip-types "{{root}}/frameworks/solid-renderer/src/tree.test.ts"
    node --experimental-strip-types "{{root}}/frameworks/solid-renderer/runtime/frame.test.ts"
    node --conditions=browser --experimental-strip-types "{{root}}/frameworks/solid-renderer/runtime/slot.test.ts"
    node --experimental-strip-types "{{root}}/frameworks/solid-renderer/runtime/list-binding.test.ts"
    node --experimental-strip-types "{{root}}/frameworks/solid-renderer/runtime/motion.test.ts"
    node --experimental-strip-types "{{root}}/frameworks/solid-renderer/src/direction.test.ts"
    node --experimental-strip-types "{{root}}/frameworks/net/src/index.test.ts"
    node --experimental-strip-types "{{root}}/frameworks/i18n/src/index.test.ts"
    cd "{{root}}/frameworks/solid-router" && pnpm install --frozen-lockfile && node --experimental-strip-types src/path.test.ts && node --experimental-strip-types src/transition.test.ts && node --conditions=browser --experimental-strip-types --import "{{root}}/frameworks/register.mjs" src/router.test.ts
    cd "{{root}}/frameworks/form" && pnpm install --frozen-lockfile && node --conditions=browser --experimental-strip-types src/index.test.ts
    cd "{{root}}/frameworks/solid-ui" && pnpm install --frozen-lockfile && for test in src/*.test.ts src/field/*.test.ts; do node --conditions=browser --experimental-strip-types --import "{{root}}/frameworks/register.mjs" "$test"; done

solid-app:
    cd "{{root}}/apps/gallery-solid" && pnpm install && node build.mjs && node --experimental-strip-types counter.node.test.ts && node --experimental-strip-types counter.esm.test.ts

host:
    cargo run --manifest-path "{{root}}/Cargo.toml" -p macos -- --js "{{root}}/apps/gallery-solid/dist/counter.js" "{{logo}}"

quickjs:
    "{{root}}/runtimes/quickjs/fetch-quickjs.sh"

host-js: host

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
