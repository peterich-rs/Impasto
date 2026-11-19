#!/usr/bin/env bash
set -euo pipefail

output=${1:?用法 tools/android-frame-trace.sh OUTPUT [APP_ID] [SECONDS]}
app_id=${2:-dev.impasto.host.profile}
seconds=${3:-10}

if [[ ! $app_id =~ ^[a-zA-Z0-9_.]+$ || ! $seconds =~ ^[1-9][0-9]?$ ]]; then
  echo "APP_ID 必须是 Android 包名，SECONDS 必须在 1 到 99 之间" >&2
  exit 2
fi

remote="/data/misc/perfetto-traces/impasto-$(date +%s)-$$.perfetto-trace"
trap 'adb shell rm -f "$remote" >/dev/null 2>&1 || true' EXIT
adb shell perfetto --txt -c - -o "$remote" <<EOF
buffers { size_kb: 65536 fill_policy: RING_BUFFER }
duration_ms: $((seconds * 1000))
data_sources {
  config {
    name: "linux.ftrace"
    ftrace_config {
      ftrace_events: "sched/sched_switch"
      ftrace_events: "sched/sched_wakeup"
      atrace_apps: "$app_id"
      atrace_categories: "gfx"
      atrace_categories: "view"
      atrace_categories: "am"
      atrace_categories: "wm"
    }
  }
}
data_sources { config { name: "android.surfaceflinger.frametimeline" } }
EOF
adb pull "$remote" "$output"
