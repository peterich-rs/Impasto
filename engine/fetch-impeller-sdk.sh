#!/bin/bash
# Download the Impeller standalone SDKs listed in engine/engine.pin.
# Each platform lands in third_party/impeller-sdk/<platform>/.
# Only headers, the dynamic library, and LICENSE.sdk.md are unpacked.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
pin="$root/engine/engine.pin"
dest_root="$root/third_party/impeller-sdk"
cache="$root/build/third_party"
base="https://storage.googleapis.com/flutter_infra_release/flutter"

if [[ -f "$dest_root/lib/libimpeller.dylib" && ! -f "$dest_root/darwin-arm64/lib/libimpeller.dylib" ]]; then
  mkdir -p "$dest_root/darwin-arm64"
  mv "$dest_root/include" "$dest_root/lib" "$dest_root/darwin-arm64/"
  if [[ -f "$dest_root/LICENSE.sdk.md" ]]; then
    mv "$dest_root/LICENSE.sdk.md" "$dest_root/darwin-arm64/"
  fi
fi

llvm_bin=""
if command -v llvm-strip >/dev/null 2>&1; then
  llvm_bin="$(dirname "$(command -v llvm-strip)")"
else
  ndk="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
  if [[ -z "$ndk" && -n "${ANDROID_HOME:-}" && -d "${ANDROID_HOME}/ndk" ]]; then
    ndk="$(find "${ANDROID_HOME}/ndk" -mindepth 1 -maxdepth 1 -type d | sort | tail -1)"
  fi
  if [[ -n "$ndk" ]]; then
    llvm_bin="$(find "$ndk/toolchains/llvm/prebuilt" -mindepth 2 -maxdepth 2 -type d -name bin | head -1)"
  fi
fi
if [[ -z "$llvm_bin" || ! -x "$llvm_bin/llvm-strip" ]]; then
  echo "llvm-strip was not found. Set ANDROID_NDK_HOME or put llvm-strip on PATH." >&2
  exit 1
fi

fix_android_names() {
  local so="$1"
  "$llvm_bin/llvm-readelf" -d "$so" >"$so.dyn"
  python3 - "$so" "$so.dyn" <<'PY'
import pathlib, sys
so = pathlib.Path(sys.argv[1])
text = pathlib.Path(sys.argv[2]).read_text()
blob = bytearray(so.read_bytes())
changed = False

def replace(old: str, new: str) -> None:
    global changed
    old_b = old.encode() + b"\0"
    new_b = new.encode() + b"\0"
    if len(new_b) > len(old_b):
        raise SystemExit(f"{old} cannot be rewritten to {new}: the new name is longer")
    start = 0
    while True:
        at = blob.find(old_b, start)
        if at < 0:
            break
        blob[at:at + len(old_b)] = new_b + b"\0" * (len(old_b) - len(new_b))
        changed = True
        start = at + len(old_b)

soname = ""
needed = []
for line in text.splitlines():
    if "SONAME" in line and "[" in line:
        soname = line.split("[", 1)[1].split("]", 1)[0]
    if "NEEDED" in line and "[" in line:
        needed.append(line.split("[", 1)[1].split("]", 1)[0])
if soname and soname != "libimpeller.so":
    replace(soname, "libimpeller.so")
for name in needed:
    base = name.rsplit("/", 1)[-1]
    if name != base:
        replace(name, base)
if changed:
    so.write_bytes(blob)
PY
  rm -f "$so.dyn"
  local soname needed
  soname="$("$llvm_bin/llvm-readelf" -d "$so" | sed -n 's/.*SONAME.*\[\(.*\)\].*/\1/p')"
  if [[ "$soname" != "libimpeller.so" ]]; then
    echo "SONAME of $so is '$soname', expected libimpeller.so" >&2
    exit 1
  fi
  needed="$("$llvm_bin/llvm-readelf" -d "$so" | sed -n 's/.*NEEDED.*\[\(.*\)\].*/\1/p')"
  if grep -q '/' <<<"$needed"; then
    echo "NEEDED entries of $so still contain a path:" >&2
    echo "$needed" >&2
    exit 1
  fi
}

finish_library() {
  local platform="$1"
  local dest="$dest_root/$platform"
  local lib=""
  if [[ -f "$dest/lib/libimpeller.dylib" ]]; then
    lib="$dest/lib/libimpeller.dylib"
    install_name_tool -id @rpath/libimpeller.dylib "$lib"
    # NDK llvm-strip does not strip Mach-O. `strip -x` drops local symbols and keeps exports.
    strip -x "$lib"
    if ! "$llvm_bin/llvm-nm" -gU "$lib" | grep -q 'ImpellerContextCreateVulkanNew'; then
      echo "$lib does not export ImpellerContextCreateVulkanNew after strip" >&2
      exit 1
    fi
  elif [[ -f "$dest/lib/libimpeller.so" ]]; then
    lib="$dest/lib/libimpeller.so"
    "$llvm_bin/llvm-strip" --strip-unneeded "$lib"
    fix_android_names "$lib"
    if ! "$llvm_bin/llvm-nm" -D --defined-only "$lib" | grep -q 'ImpellerContextCreateVulkanNew'; then
      echo "$lib does not export ImpellerContextCreateVulkanNew after strip" >&2
      exit 1
    fi
    echo "stripped $(wc -c <"$lib" | tr -d ' ') bytes $lib"
  else
    echo "no dynamic library in $dest" >&2
    exit 1
  fi
}

unpack_platform() {
  local platform="$1"
  local commit="$2"
  local sha="$3"
  local dest="$dest_root/$platform"
  if [[ -f "$dest/include/impeller.h" && ( -f "$dest/lib/libimpeller.dylib" || -f "$dest/lib/libimpeller.so" ) ]]; then
    finish_library "$platform"
    return
  fi
  mkdir -p "$cache" "$dest"
  local zip="$cache/impeller_sdk-${platform}.zip"
  if [[ ! -f "$zip" ]]; then
    curl -fL --retry 3 -o "$zip" "$base/${commit}/${platform}/impeller_sdk.zip"
  fi
  local actual
  actual="$(shasum -a 256 "$zip" | awk '{print $1}')"
  if [[ "$actual" != "$sha" ]]; then
    echo "$platform zip sha256 is $actual, pin says $sha" >&2
    exit 1
  fi
  local stage
  stage="$(mktemp -d)"
  local members=()
  while IFS= read -r name; do
    members+=("$name")
  done < <(unzip -Z1 "$zip" | awk '
    $0 ~ /^include\// || $0 == "LICENSE.sdk.md" || $0 ~ /(^|\/)libimpeller\.(so|dylib)$/ { print }
  ')
  if [[ ${#members[@]} -eq 0 ]]; then
    echo "$platform zip has no headers or dynamic library" >&2
    exit 1
  fi
  unzip -o "$zip" "${members[@]}" -d "$stage" >/dev/null
  mkdir -p "$dest/include" "$dest/lib"
  if [[ -d "$stage/include" ]]; then
    cp -R "$stage/include/." "$dest/include/"
  fi
  if [[ -f "$stage/LICENSE.sdk.md" ]]; then
    cp "$stage/LICENSE.sdk.md" "$dest/LICENSE.sdk.md"
  fi
  local found
  found="$(find "$stage" -type f \( -name 'libimpeller.dylib' -o -name 'libimpeller.so' \) | head -1)"
  if [[ -z "$found" ]]; then
    echo "$platform zip unpacked no dynamic library" >&2
    exit 1
  fi
  cp "$found" "$dest/lib/$(basename "$found")"
  rm -rf "$stage"
  if [[ ! -f "$dest/include/impeller.h" ]]; then
    echo "$platform zip has no include/impeller.h" >&2
    exit 1
  fi
  finish_library "$platform"
}

found=0
while read -r kind platform hash_kind commit sha; do
  [[ "$kind" == "platform" ]] || continue
  found=1
  unpack_platform "$platform" "$commit" "$sha"
done <"$pin"
if [[ "$found" -eq 0 ]]; then
  echo "engine.pin lists no platforms" >&2
  exit 1
fi
