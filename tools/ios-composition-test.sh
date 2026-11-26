#!/usr/bin/env bash
set -euo pipefail

usage() {
    printf '%s\n' '用法: tools/ios-composition-test.sh <模拟器 UDID> [截图输出目录]'
}

if [[ ${1:-} == --help || ${1:-} == -h ]]; then
    usage
    exit 0
fi
if [[ $# -lt 1 || $# -gt 2 || ! "$1" =~ ^[a-fA-F0-9-]{36}$ ]]; then
    usage >&2
    exit 2
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$root/tools/ios-composition-fixture"
udid="$1"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/impasto-ios-composition.XXXXXX")"
bundle="dev.impasto.ios-composition.$(uuidgen | tr '[:upper:]' '[:lower:]')"
installed=false
launcher=""
cleanup() {
    local status=$?
    trap - EXIT
    if [[ "$installed" == true ]]; then
        xcrun simctl uninstall "$udid" "$bundle" >/dev/null 2>&1 || {
            printf '无法卸载本次验收应用 %s\n' "$bundle" >&2
            status=1
        }
    fi
    if [[ -n "$launcher" ]]; then
        kill "$launcher" >/dev/null 2>&1 || true
        wait "$launcher" 2>/dev/null || true
    fi
    rm -r -- "$scratch"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

xcrun simctl bootstatus "$udid" -b
if xcrun simctl get_app_container "$udid" "$bundle" app >/dev/null 2>&1; then
    printf '已存在同 bundle ID 的应用 %s\n' "$bundle" >&2
    exit 1
fi

app="$scratch/app"
mkdir -p "$app/src" "$app/assets"
cp "$fixture/app.toml" "$app/app.toml"
cp "$fixture/src/main.tsx" "$app/src/main.tsx"
printf '\n[ios]\nbundle_id = "%s"\n' "$bundle" >> "$app/app.toml"
(
    cd "$app"
    cargo run --locked --manifest-path "$root/Cargo.toml" -p cli -- build ios --ios-simulator --debug
)

project="$app/build/ios/iphonesimulator/debug"
product="$project/xcode/Build/Products/Debug-iphonesimulator/Impasto.app"
architecture="$(xcrun lipo -archs "$product/Impasto")"
cp "$fixture/App.swift" "$app/ios/Runner/App.swift"
xcodebuild -project "$app/ios/Runner.xcodeproj" -scheme Runner -configuration Debug \
    -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' -derivedDataPath "$project/xcode" \
    "ARCHS=$architecture" CODE_SIGNING_ALLOWED=NO -quiet build

xcrun simctl install "$udid" "$product"
installed=true
container="$(xcrun simctl get_app_container "$udid" "$bundle" data)"
xcrun simctl launch --console "$udid" "$bundle" > "$scratch/console.log" 2>&1 &
launcher=$!
report="$container/Documents/report.txt"
deadline=$((SECONDS + 30))
while [[ ! -f "$report" ]] && (( SECONDS < deadline )) && kill -0 "$launcher" 2>/dev/null; do
    sleep 0.25
done

if [[ ! -f "$report" ]]; then
    printf '验收应用未生成报告。\n' >&2
    cat "$scratch/console.log" >&2
    exit 1
fi
if [[ "$(cat "$report")" != ok ]]; then
    cat "$report" >&2
    printf '\n' >&2
    exit 1
fi

if [[ -n ${2:-} ]]; then
    mkdir -p -- "$2"
    artifacts="$(mktemp -d "$2/impasto-ios-composition.XXXXXX")"
    cp "$container/Documents/mixed.png" "$artifacts/"
    cp "$scratch/console.log" "$artifacts/"
    printf '截图与日志位于 %s\n' "$artifacts"
fi
printf 'UIKit/Metal 混排、裁剪与命中、BGRA/NV12、截图范围和无障碍阅读顺序检查通过。\n'
