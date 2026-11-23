#!/usr/bin/env bash
set -euo pipefail

out=${1:?用法 tools/android-frame-replay.sh OUTPUT_DIR [RUNS] [SECONDS]}
runs=${2:-3}
seconds=${3:-6}
refresh=${FRAME_REFRESH:-}
suite=${FRAME_SUITE:-all}
reference=${FRAME_REFERENCE:-impasto}
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(cd -- "$script_dir/.." && pwd)
case "$reference" in
  impasto)
    app=dev.impasto.host.profile
    activity="$app/dev.impasto.host.ImpastoActivity"
    launch_args=()
    detail_x=120 detail_y=491 daily_x=483 daily_y=982 pause_y=235 crosshair_y=1350
    ;;
  flutter)
    app=dev.impasto.market_reference
    activity="$app/.MainActivity"
    launch_args=(--ez trace-systrace true)
    detail_x=120 detail_y=306 daily_x=780 daily_y=570 pause_y=132 crosshair_y=960
    ;;
  *) echo "FRAME_REFERENCE 必须是 impasto 或 flutter" >&2; exit 2 ;;
esac
case "$suite" in
  all|detail|daily|list) ;;
  *) echo "FRAME_SUITE 必须是 all、detail、daily 或 list" >&2; exit 2 ;;
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
  printf 'app=%s\nreference=%s\nsuite=%s\nruns=%s\nseconds=%s\nrequested_refresh_hz=%s\n' "$app" "$reference" "$suite" "$runs" "$seconds" "${refresh:-default}"
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
