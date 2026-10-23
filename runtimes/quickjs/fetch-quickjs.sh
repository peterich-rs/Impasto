#!/bin/bash
# Download the quickjs-ng release pinned below. Sources land in third_party/quickjs.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
tag="v0.17.0"
sha="559bc4c420475e55c7ab4510adbc562f55d7524d75e8e89d79ce4bb02f5687d9"
dest="$root/third_party/quickjs"
if [[ -f "$dest/quickjs.c" && -f "$dest/quickjs.h" ]]; then
  exit 0
fi
mkdir -p "$root/build/third_party"
tar="$root/build/third_party/quickjs-${tag}.tar.gz"
if [[ ! -f "$tar" ]]; then
  curl -fL --retry 3 -o "$tar" \
    "https://github.com/quickjs-ng/quickjs/archive/refs/tags/${tag}.tar.gz"
fi
echo "${sha}  ${tar}" | shasum -a 256 -c -
rm -rf "$dest"
mkdir -p "$dest"
tar -xzf "$tar" -C "$dest" --strip-components=1
