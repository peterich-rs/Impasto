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
    out="$src/dist/quotes.js"
    if [[ -f "$out" ]]; then
      newer=$(find "{{root}}/packages" "{{root}}/tools" "$src" -type f \( -name '*.ts' -o -name '*.tsx' -o -name '*.mjs' \) \
        -not -path '*/node_modules/*' -not -path '*/dist/*' -not -name '*.test.ts' \
        -newer "$out" -print -quit)
      [[ -z "$newer" ]] && exit 0
    fi
    pnpm -C "$src" install && pnpm -C "$src" run build

host: gallery-dist
    cargo run --manifest-path "{{root}}/Cargo.toml" -p macos -- --js "{{root}}/apps/gallery-solid/dist/quotes.js" "{{logo}}"

quickjs:
    "{{root}}/crates/quickjs/fetch-quickjs.sh"

host-js: host

# Debug APK. Gradle runs cargo build without --release.
# Needs the Android NDK, the arm64 Impeller SDK, and cargo on PATH.
android: gallery-dist
    "{{root}}/crates/android/apk/gradlew" -p "{{root}}/crates/android/apk" assembleDebug

# Release APK. Gradle runs cargo build --release. Signed with the debug keystore.
android-release: gallery-dist
    "{{root}}/crates/android/apk/gradlew" -p "{{root}}/crates/android/apk" assembleRelease
