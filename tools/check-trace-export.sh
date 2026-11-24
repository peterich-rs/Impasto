#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
processor="${IMPASTO_TRACE_PROCESSOR:-trace_processor}"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
trace="$scratch/impasto.pftrace"
IMPASTO_TEST_TRACE="$trace" cargo test --locked --manifest-path "$root/Cargo.toml" -p embedder --lib \
  diagnostics::trace_export::tests::encodes_nested_spans_flows_counters_and_frame_relative_gpu -- --exact

query="
WITH expected(source, target) AS (
  VALUES ('Vsync received', 'UI started'), ('UI submitted', 'Submission queued'),
    ('Submission queued', 'Raster started')
), edges AS (
  SELECT s.name AS source, t.name AS target FROM flow
  JOIN slice s ON s.id = slice_out JOIN slice t ON t.id = slice_in
)
SELECT CASE WHEN
    (SELECT COUNT(*) FROM slice s JOIN slice p ON p.id = s.parent_id
      WHERE s.name = 'layout' AND s.category = 'impasto::layout' AND p.name = 'frame') = 3
    AND EXISTS (SELECT 1 FROM slice s JOIN slice p ON p.id = s.parent_id
      WHERE s.name = 'taffy_compute' AND p.name = 'layout')
    AND NOT EXISTS (SELECT * FROM expected EXCEPT SELECT * FROM edges)
    AND (SELECT COUNT(*) FROM edges) = 5
    AND (SELECT COUNT(*) FROM edges WHERE source = 'Submission queued' AND target = 'Raster started') = 2
    AND (SELECT COUNT(*) FROM slice WHERE name = 'UI frame') = 1
    AND (SELECT COUNT(*) FROM counter) = 48
    AND EXISTS (SELECT 1 FROM slice WHERE name = 'Main' AND dur = 0
      AND EXTRACT_ARG(arg_set_id, 'debug.offset_us') = 100.0)
    AND NOT EXISTS (SELECT 1 FROM stats WHERE value > 0 AND severity IN ('error', 'data_loss'))
  THEN 'trace-ok' ELSE 'trace-invalid' END AS result;
"
result="$("$processor" query "$trace" "$query")"
if [[ "$result" != *trace-ok* ]]; then
  printf '%s\n' "$result" >&2
  exit 1
fi
