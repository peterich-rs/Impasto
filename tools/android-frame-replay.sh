#!/usr/bin/env bash
set -euo pipefail

out=${1:?用法 tools/android-frame-replay.sh OUTPUT_DIR [RUNS] [SECONDS]}
runs=${2:-3}
seconds=${3:-6}
app=dev.impasto.host.profile
activity="$app/dev.impasto.host.HostActivity"
refresh=${FRAME_REFRESH:-}
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
mkdir -p -- "$out"

# The touch script uses the same portrait viewport as the Pixel 9 capture.
# Scale physical points so changing Android's display resolution is harmless.
read -r width height < <(adb shell wm size | tail -1 | sed -E 's/.* ([0-9]+)x([0-9]+).*/\1 \2/')
x() { echo "$(($1 * width / 1080))"; }
y() { echo "$(($1 * height / 2424))"; }
tap() { adb shell input tap "$(x "$1")" "$(y "$2")"; }
swipe() { adb shell input swipe "$(x "$1")" "$(y "$2")" "$(x "$3")" "$(y "$4")" "$5"; }
start() { adb shell am start -W -n "$activity" >/dev/null; }
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

presentation_times() {
  local pid=$1
  local layer
  while kill -0 "$pid" 2>/dev/null; do
    if [[ -z ${layer:-} ]]; then
      layer=$(adb shell dumpsys SurfaceFlinger --list | awk '
        /SurfaceView.*dev[.]impasto[.]host[.]profile.*[(]BLAST[)]/ && !found {
          sub(/^RequestedLayerState[{]/, ""); sub(/ parentId=.*/, ""); print; found=1
        }')
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
detail() { tap 120 491; }
daily() { tap 483 982; }
pause_market() { tap 1002 235; }

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
    swipe 212 1350 797 1350 650
    swipe 797 1350 212 1350 650
  done
}
scroll_list() {
  for _ in 1 2 3; do
    swipe 541 1920 541 780 520
    swipe 541 780 541 1920 520
  done
}

adb shell input keyevent KEYCODE_WAKEUP
adb shell wm dismiss-keyguard
if [[ -n $refresh ]]; then
  case "$refresh" in 60|120) ;; *) echo "FRAME_REFRESH 必须是 60 或 120" >&2; exit 2 ;; esac
  original_min=$(adb shell settings get system min_refresh_rate)
  original_peak=$(adb shell settings get system peak_refresh_rate)
  trap restore_refresh EXIT
  adb shell settings put system min_refresh_rate "$refresh"
  adb shell settings put system peak_refresh_rate "$refresh"
fi
for ((run = 1; run <= runs; run++)); do
  stop
  capture "startup-$run" start
  capture "detail-entry-$run" detail
  capture "intraday-$run" idle
  daily
  sleep 1
  capture "daily-$run" idle
  capture "crosshair-live-$run" crosshair
  pause_market
  sleep 1
  capture "crosshair-paused-$run" crosshair
  stop
  start
  sleep 1
  capture "home-$run" idle
  capture "list-scroll-$run" scroll_list
done
