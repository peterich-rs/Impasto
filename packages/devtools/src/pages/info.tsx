import { View as HostView } from "@impasto/solid-renderer/host";
/** Info page: platform, versions, memory. */
import { createRenderEffect, createSignal, For, Show } from "solid-js";
import { Text, useTheme } from "@impasto/solid-ui";
import { scene, readMetrics } from "@impasto/solid-renderer";
import * as raw from "@impasto/devtools/native";
type Row = [string, string];
function readIfAvailable<T>(read: () => T): T | null {
  try {
    return read();
  } catch {
    return null;
  }
}
export function InfoPage() {
  const theme = useTheme();
  const [rows, setRows] = createSignal<Row[]>([]);
  createRenderEffect(
    () => true,
    () => {
      const metrics = readMetrics();
      let memory = "-";
      try {
        const text = scene.debugMemory?.() ?? null;
        if (typeof text === "string") {
          const parsed = JSON.parse(text) as {
            mallocBytes?: number;
            objects?: number;
          };
          memory = `${((parsed.mallocBytes ?? 0) / 1024).toFixed(0)}KB / ${parsed.objects ?? 0} objects`;
        }
      } catch {
        /* scene without the debug binding keeps the dash */
      }
      const platform = readIfAvailable(() => raw.platform());
      const versions = readIfAvailable(() => raw.versions());
      const viewport = platform?.viewport;
      const out: Row[] = [
        [
          "os",
          (platform?.os ?? "") +
            (platform?.osVersion
              ? ` ${platform.osVersion}`
              : ""),
        ],
        [
          "viewport",
          viewport
            ? `${viewport.w ?? "-"}×${viewport.h ?? "-"} @${viewport.dpr ?? "-"}`
            : metrics
              ? `${metrics.width}×${metrics.height}`
              : "-",
        ],
        [
          "insets",
          metrics
            ? `t${metrics.viewInsets.top} b${metrics.viewInsets.bottom}`
            : "-",
        ],
        ["app", platform?.appState ?? ""],
        ["log", platform?.log || "info"],
        ["devtools", platform?.devtools === true ? "1" : platform ? "0" : "-"],
        ["dump", platform?.dump || "-"],
        ["shell", versions?.shell ?? ""],
        ["wgpu", versions?.wgpu ?? ""],
        ["Parley", versions?.parley ?? ""],
        ["quickjs", versions?.quickjsPin ?? ""],
        ["memory", memory],
        ["sourcemap", "见错误页堆栈"],
      ];
      setRows(out);
    },
  );
  return (
    <HostView
      style={{
        width: "100%",
        height: "100%",
        flex: 1,
        flexShrink: 1,
        minWidth: 0,
        minHeight: 0,
        gap: 2,
      }}
    >
      <For each={rows()}>
        {(row) => (
          <Text
            style={{
              fontSize: 10,
              color: theme().color.text,
            }}
          >
            {row[0]}: {row[1]}
          </Text>
        )}
      </For>
      <Show when={rows().length === 0}>
        <Text
          style={{
            fontSize: 10,
            color: theme().color.textMuted,
          }}
        >
          读取中…
        </Text>
      </Show>
    </HostView>
  );
}
