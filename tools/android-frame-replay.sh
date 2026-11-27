#!/usr/bin/env bash
set -euo pipefail

out=${1:?用法 tools/android-frame-replay.sh OUTPUT_DIR [RUNS] [SECONDS]}
runs=${2:-3}
refresh=${FRAME_REFRESH:-}
reference=${FRAME_REFERENCE:-impasto}
default_suite=fixture
if [[ $reference == flutter ]]; then default_suite=all; fi
suite=${FRAME_SUITE:-$default_suite}
default_seconds=6
if [[ $suite == fixture ]]; then default_seconds=18; fi
seconds=${3:-$default_seconds}
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(cd -- "$script_dir/.." && pwd)
case "$reference" in
  impasto)
    app=${FRAME_APP_ID:?Specify FRAME_APP_ID for the installed app}
    activity=$(adb shell cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -p "$app" | tail -1)
    if [[ $activity != "$app/"* ]]; then
      echo "找不到 $app 的启动入口" >&2
      exit 1
    fi
    launch_args=()
    detail_x=120 detail_y=491 daily_x=483 daily_y=982 pause_y=235 crosshair_y=1350
    ;;
  flutter)
    app=${FRAME_APP_ID:-dev.impasto.market_reference}
    activity="$app/.MainActivity"
    launch_args=(--ez trace-systrace true)
    detail_x=120 detail_y=306 daily_x=780 daily_y=570 pause_y=132 crosshair_y=960
    ;;
  *) echo "FRAME_REFERENCE 必须是 impasto 或 flutter" >&2; exit 2 ;;
esac
case "$suite" in
  fixture|all|detail|daily|list) ;;
  *) echo "FRAME_SUITE 必须是 fixture、all、detail、daily 或 list" >&2; exit 2 ;;
esac
mkdir -p -- "$out"

# The touch script uses the same portrait viewport as the Pixel 9 capture.
# Scale physical points so changing Android's display resolution is harmless.
read -r width height < <(adb shell wm size | tail -1 | sed -E 's/.* ([0-9]+)x([0-9]+).*/\1 \2/')
x() { echo "$(($1 * width / 1080))"; }
y() { echo "$(($1 * height / 2424))"; }
tap() { adb shell input tap "$(x "$1")" "$(y "$2")"; }
swipe() { adb shell input swipe "$(x "$1")" "$(y "$2")" "$(x "$3")" "$(y "$4")" "$5"; }
start() { adb shell am start -W -n "$activity" ${launch_args[@]+"${launch_args[@]}"} >/dev/null; }
stop() { adb shell am force-stop "$app"; }

restore_refresh() {
  for key in min_refresh_rate peak_refresh_rate; do
    local value=$original_min
    [[ $key != peak_refresh_rate ]] || value=$original_peak
    if [[ $value == null ]]; then
      adb shell settings delete system "$key" >/dev/null
    else
      adb shell settings put system "$key" "$value"
    fi
  done
}

profile_layer() {
  adb shell dumpsys SurfaceFlinger --list | awk -v app="$app" '
    /SurfaceView/ && index($0, "SurfaceView[" app "/") && /[(]BLAST[)]/ && !found {
      sub(/^RequestedLayerState[{]/, ""); sub(/ parentId=.*/, ""); print; found=1
    }'
}

presentation_times() {
  local pid=$1
  local layer
  while kill -0 "$pid" 2>/dev/null; do
    if [[ -z ${layer:-} ]]; then
      layer=$(profile_layer)
    fi
    if [[ -n $layer ]]; then
      local sample
      sample=$(adb shell "dumpsys SurfaceFlinger --latency '$layer'")
      if [[ $sample == *$'\t'* ]]; then
        echo "$sample"
      else
        layer=
      fi
    fi
    sleep 0.7
  done
}
detail() { tap "$detail_x" "$detail_y"; }
daily() { tap "$daily_x" "$daily_y"; }
pause_market() { tap 1002 "$pause_y"; }

capture() {
  local name=$1
  shift
  local trace="$out/$name.perfetto-trace"
  "$script_dir/android-frame-trace.sh" "$trace" "$app" "$seconds" >"$out/$name.log" 2>&1 &
  local pid=$!
  presentation_times "$pid" >"$out/$name.presentation" &
  local sampler=$!
  sleep 0.7
  local result=0
  "$@" || result=$?
  local trace_status=0
  wait "$pid" || trace_status=$?
  wait "$sampler"
  ((trace_status == 0)) || return "$trace_status"
  return "$result"
}

idle() { :; }
crosshair() {
  for _ in 1 2 3; do
    swipe 212 "$crosshair_y" 797 "$crosshair_y" 650
    swipe 797 "$crosshair_y" 212 "$crosshair_y" 650
  done
}
scroll_list() {
  for _ in 1 2 3; do
    swipe 541 1920 541 780 520
    swipe 541 780 541 1920 520
  done
}

replay_fixture_detail() {
  # Match text labels through Android's semantic tree, including density and insets.
  tap_label 'Open detail' || return
  tap_label 'Show dialog' || return
  tap_label 'Close dialog' || return
  tap_label 'Replay editor' || return
  adb shell input text replay || return
  adb shell input keyevent KEYCODE_BACK || return
  tap_label 'Back'
}

tap_label() {
  local label=$1 point tap_x tap_y file=/data/local/tmp/impasto-replay-$$.xml
  adb shell uiautomator dump "$file" >/dev/null || return
  point=$(adb shell cat "$file" | python3 -c '
import re, sys
from xml.etree import ElementTree

for node in ElementTree.parse(sys.stdin).iter("node"):
    if sys.argv[1] in (node.get("text"), node.get("content-desc")):
        left, top, right, bottom = map(int, re.findall(r"-?\d+", node.attrib["bounds"]))
        print((left + right) // 2, (top + bottom) // 2)
        break
else:
    sys.exit(f"missing visible label {sys.argv[1]!r}")
' "$label") || { adb shell rm -f "$file"; return 1; }
  adb shell rm -f "$file"
  read -r tap_x tap_y <<< "$point"
  adb shell input tap "$tap_x" "$tap_y"
}

scroll_detail() {
  for _ in 1 2 3; do swipe 541 1770 541 580 570; done
  for _ in 1 2 3; do swipe 541 580 541 1770 570; done
}

open_daily_detail() {
  stop
  start
  sleep 1
  if [[ -z $(profile_layer) ]]; then
    echo "找不到 $app 的 SurfaceView，检查设备是否停在解锁或启动界面" >&2
    return 1
  fi
  detail
  sleep 1
  daily
  sleep 1
}

adb shell input keyevent KEYCODE_WAKEUP
adb shell wm dismiss-keyguard
if [[ -n $refresh ]]; then
  case "$refresh" in 60|90|120) ;; *) echo "FRAME_REFRESH 必须是 60、90 或 120" >&2; exit 2 ;; esac
  original_min=$(adb shell settings get system min_refresh_rate)
  original_peak=$(adb shell settings get system peak_refresh_rate)
  trap restore_refresh EXIT
  adb shell settings put system min_refresh_rate "$refresh"
  adb shell settings put system peak_refresh_rate "$refresh"
fi
{
  printf 'app=%s\nreference=%s\nsuite=%s\nruns=%s\nseconds=%s\nrequested_refresh_hz=%s\nbuild_profile=%s\n' "$app" "$reference" "$suite" "$runs" "$seconds" "${refresh:-default}" "${FRAME_BUILD_PROFILE:-unspecified}"
  printf 'impasto_commit=%s\n' "$(git -C "$repo_dir" rev-parse HEAD)"
  if [[ -n $(git -C "$repo_dir" status --porcelain) ]]; then
    printf 'impasto_checkout_dirty=true\n'
  else
    printf 'impasto_checkout_dirty=false\n'
  fi
  if [[ $reference == impasto ]]; then
    cargo tree --locked --manifest-path "$repo_dir/Cargo.toml" -p raster --depth 1 --prefix none | sed -n '/^wgpu v/p; /^parley v/p'
  else
    version="$repo_dir/target/android-flutter-reference/flutter-version.json"
    if [[ -f $version ]]; then cat "$version"; fi
  fi
  adb shell getprop ro.build.fingerprint
  adb shell getprop ro.product.model
  adb shell getprop ro.product.cpu.abilist
  adb shell getconf PAGE_SIZE
  adb shell wm size
  adb shell wm density
  adb shell pm path "$app"
} >"$out/environment.txt"
for ((run = 1; run <= runs; run++)); do
  if [[ $suite == fixture ]]; then
    stop
    capture "fixture-startup-$run" start
    capture "fixture-list-$run" scroll_list
    capture "fixture-detail-$run" replay_fixture_detail
    continue
  fi
  if [[ $suite == list ]]; then
    stop
    start
    sleep 1
    capture "list-scroll-$run" scroll_list
    continue
  fi
  if [[ $suite == all ]]; then
    stop
    capture "startup-$run" start
    capture "detail-entry-$run" detail
    capture "intraday-$run" idle
    daily
    sleep 1
  else
    open_daily_detail
  fi
  capture "daily-$run" idle
  if [[ $suite == daily ]]; then
    continue
  fi
  if [[ $suite == all ]]; then
    capture "crosshair-live-$run" crosshair
  fi
  pause_market
  sleep 1
  capture "crosshair-paused-$run" crosshair
  capture "detail-scroll-paused-$run" scroll_detail
  open_daily_detail
  capture "detail-scroll-live-$run" scroll_detail
  if [[ $suite == all ]]; then
    stop
    start
    sleep 1
    capture "home-$run" idle
    capture "list-scroll-$run" scroll_list
  fi
done
pid=$(adb shell pidof "$app" || true)
if [[ $reference == impasto && -n $pid ]]; then
  adb logcat -d --pid="$pid" | awk '/Android surface opened/' >"$out/backends.log"
fi
