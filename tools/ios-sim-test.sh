#!/usr/bin/env bash
set -euo pipefail

usage() {
    printf '%s\n' '用法: tools/ios-sim-test.sh [--debug|--release] [--udid UDID] [--app PATH]'
}

mode=debug
app=""
udid=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --debug) mode=debug; shift ;;
        --release) mode=release; shift ;;
        --app|--udid)
            if [[ $# -lt 2 || "$2" == --* ]]; then
                usage >&2
                exit 2
            fi
            if [[ "$1" == --app ]]; then
                app="$2"
            else
                udid="$2"
            fi
            shift 2
            ;;
        --help|-h) usage; exit 0 ;;
        *)
            usage >&2
            exit 2
            ;;
    esac
done

scratch="$(mktemp -d "${TMPDIR:-/tmp}/impasto-ios-smoke.XXXXXX")"
fixture_id=""
installed=false
cleanup() {
    local status=$?
    if [[ "$installed" == true ]]; then
        xcrun simctl uninstall "$udid" "$fixture_id" >/dev/null 2>&1 || {
            printf '无法清理本次测试副本 %s\n' "$fixture_id" >&2
            status=1
        }
    fi
    rm -r -- "$scratch"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

cat > "$scratch/probe.swift" <<'SWIFT'
import CoreGraphics
import Darwin
import Foundation
import ImageIO

struct Device: Decodable {
    let udid: String
    let state: String
    let isAvailable: Bool?
}

struct Devices: Decodable {
    let devices: [String: [Device]]
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("\(message)\n".utf8))
    exit(1)
}

func selectDevice(_ json: URL, requested: String) throws {
    let devices = try JSONDecoder().decode(Devices.self, from: Data(contentsOf: json))
    let booted = devices.devices
        .filter { $0.key.hasPrefix("com.apple.CoreSimulator.SimRuntime.iOS-") }
        .values.flatMap { $0 }
        .filter { $0.state == "Booted" && $0.isAvailable != false }
    if requested.isEmpty {
        guard booted.count == 1, let device = booted.first else {
            fail("需要一个已启动的 iOS 模拟器。启动多个时请用 --udid 指定。")
        }
        print(device.udid)
    } else {
        guard let device = booted.first(where: { $0.udid.caseInsensitiveCompare(requested) == .orderedSame }) else {
            fail("--udid 指定的 iOS 模拟器当前未启动。")
        }
        print(device.udid)
    }
}

func painted(_ file: URL) -> Bool {
    guard let source = CGImageSourceCreateWithURL(file as CFURL, nil),
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil),
          let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) else { return false }

    // Use the middle of the display so the status bar, home indicator,
    // rounded corners and device shell do not count as app content.
    let width = 48
    let height = 72
    let insetX = CGFloat(image.width) * 0.08
    let insetY = CGFloat(image.height) * 0.15
    let ratioX = CGFloat(width) / (CGFloat(image.width) - insetX * 2)
    let ratioY = CGFloat(height) / (CGFloat(image.height) - insetY * 2)
    var bytes = [UInt8](repeating: 0, count: width * height * 4)
    let decoded = bytes.withUnsafeMutableBytes { data -> Bool in
        guard let context = CGContext(
            data: data.baseAddress, width: width, height: height, bitsPerComponent: 8,
            bytesPerRow: width * 4, space: colorSpace,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else { return false }
        context.draw(image, in: CGRect(
            x: -insetX * ratioX, y: -insetY * ratioY,
            width: CGFloat(image.width) * ratioX, height: CGFloat(image.height) * ratioY
        ))
        return true
    }
    guard decoded else { return false }

    var tonalRegions = [Int: Int]()
    var nonWhite = 0
    var chromatic = 0
    var sampleCount = 0
    for pixel in stride(from: 0, to: bytes.count, by: 4) {
        let row = pixel / (width * 4)
        // Use the outer bands to avoid counting a centered system alert.
        if row >= height / 4 && row < height * 3 / 4 { continue }
        sampleCount += 1
        let r = Int(bytes[pixel])
        let g = Int(bytes[pixel + 1])
        let b = Int(bytes[pixel + 2])
        let low = min(r, g, b)
        let high = max(r, g, b)
        if low < 230 { nonWhite += 1 }
        if high - low >= 36 { chromatic += 1 }
        let bucket = ((r / 12) << 10) | ((g / 12) << 5) | (b / 12)
        tonalRegions[bucket, default: 0] += 1
    }

    let dominant = tonalRegions.values.max() ?? sampleCount
    // A blank launch screen can be white or dark. A single chromatic fill
    // also counts, allowing a simple colored verification app.
    return nonWhite * 100 >= sampleCount * 7
        && (chromatic * 100 >= sampleCount * 5 || dominant * 100 <= sampleCount * 97)
}

let arguments = CommandLine.arguments
guard arguments.count >= 3 else { exit(2) }
let input = URL(fileURLWithPath: arguments[2])
switch arguments[1] {
case "device":
    do {
        try selectDevice(input, requested: arguments.count == 4 ? arguments[3] : "")
    } catch {
        fail("读取模拟器列表失败。\(error)")
    }
case "frame":
    exit(painted(input) ? 0 : 1)
default:
    exit(2)
}
SWIFT
xcrun swiftc -O "$scratch/probe.swift" -o "$scratch/probe"
xcrun simctl list devices booted --json > "$scratch/devices.json"
udid="$("$scratch/probe" device "$scratch/devices.json" "$udid")"

if [[ -z "$app" ]]; then
    root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
    (
        cd "$root"
        cargo run --quiet --locked -p cli -- build ios --ios-simulator "--$mode"
    ) | tee "$scratch/build.log"
    app="$(tail -1 "$scratch/build.log")"
fi

if [[ ! -f "$app/Info.plist" ]]; then
    printf '找不到模拟器应用 %s\n' "$app" >&2
    exit 1
fi
executable="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app/Info.plist")"
if [[ -z "$executable" || "$executable" == */* || ! -f "$app/$executable" ]]; then
    printf '应用缺少有效的 CFBundleExecutable。\n' >&2
    exit 1
fi
if ! xcrun vtool -show-build "$app/$executable" | grep -q 'platform IOSSIMULATOR'; then
    printf '应用二进制没有 iOS 模拟器切片。\n' >&2
    exit 1
fi
if [[ "$mode" == debug ]]; then
    [[ -f "$app/app.js" && ! -e "$app/app.bin" ]] || {
        printf 'Debug 应用需要 app.js。\n' >&2
        exit 1
    }
else
    [[ -f "$app/app.bin" && ! -e "$app/app.js" ]] || {
        printf 'Release 应用需要 app.bin。\n' >&2
        exit 1
    }
fi

# Install a copy under a new bundle identifier and leave the user's app alone.
staged="$scratch/ImpastoSmoke.app"
ditto "$app" "$staged"
fixture_id="dev.impasto.ios-smoke.$(uuidgen | tr '[:upper:]' '[:lower:]')"
plutil -replace CFBundleIdentifier -string "$fixture_id" "$staged/Info.plist"
if plutil -extract CFBundleURLTypes xml1 -o - "$staged/Info.plist" >/dev/null 2>&1; then
    plutil -remove CFBundleURLTypes "$staged/Info.plist"
fi
codesign --force --deep --sign - "$staged" >/dev/null 2>&1
xcrun simctl install "$udid" "$staged"
installed=true

launched="$(xcrun simctl launch "$udid" "$fixture_id")"
pid="${launched##*: }"
if [[ ! "$pid" =~ ^[0-9]+$ ]]; then
    printf '无法读取模拟器应用进程 ID。%s\n' "$launched" >&2
    exit 1
fi

running() {
    xcrun simctl spawn "$udid" launchctl list | awk -v pid="$pid" -v label="UIKitApplication:$fixture_id[" '
        $1 == pid && index($3, label) == 1 { found = 1 }
        END { exit (found ? 0 : 1) }
    '
}

# Check twice so a transient launch animation does not count as the app frame.
deadline=$((SECONDS + 15))
consecutive=0
while (( SECONDS < deadline )); do
    sleep 0.4
    if ! running; then
        printf 'iOS %s 应用启动后提前退出。\n' "$mode" >&2
        exit 1
    fi
    if xcrun simctl io "$udid" screenshot "$scratch/frame.png" >/dev/null 2>&1         && "$scratch/probe" frame "$scratch/frame.png"; then
        consecutive=$((consecutive + 1))
        if (( consecutive == 2 )); then
            xcrun simctl terminate "$udid" "$fixture_id"
            printf 'iOS %s 可见首帧与进程检查通过。\n' "$mode"
            exit 0
        fi
    else
        consecutive=0
    fi
done
printf 'iOS %s 应用在等待时间内没有产生可见画面。\n' "$mode" >&2
exit 1
