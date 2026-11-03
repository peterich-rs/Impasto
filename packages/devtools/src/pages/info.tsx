/** Info page: platform, versions, memory. */
import { createRenderEffect, createSignal, For, Show } from "solid-js";
import { Text, useTheme } from "@impasto/solid-ui";
import { host, readMetrics } from "@impasto/solid-renderer";
import * as raw from "../host.gen.ts";

type Row = [string, string];

function callRecord(read: () => unknown): Record<string, unknown> | null {
  try {
    const value = read();
    if (value && typeof value === "object") {
      return value as Record<string, unknown>;
    }
  } catch {
    /* host module not linked */
  }
  return null;
}

function textOf(value: unknown): string {
  return typeof value === "string" ? value : "";
}

export function InfoPage() {
  const theme = useTheme();
  const [rows, setRows] = createSignal<Row[]>([]);
  createRenderEffect(() => {
    const metrics = readMetrics();
    let memory = "-";
    try {
      const text = host.debugMemory?.() ?? null;
      if (typeof text === "string") {
        const parsed = JSON.parse(text) as { mallocBytes?: number; objects?: number };
        memory = `${((parsed.mallocBytes ?? 0) / 1024).toFixed(0)}KB / ${parsed.objects ?? 0} objects`;
      }
    } catch {
      /* host without the debug binding keeps the dash */
    }
    const platform = callRecord(() => raw.platform());
    const versions = callRecord(() => raw.versions());
    const viewport = platform?.viewport as { w?: number; h?: number; dpr?: number } | undefined;
    const out: Row[] = [
      ["os", textOf(platform?.os) + (textOf(platform?.osVersion) ? ` ${textOf(platform?.osVersion)}` : "")],
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
        metrics ? `t${metrics.viewInsets.top} b${metrics.viewInsets.bottom}` : "-",
      ],
      ["app", textOf(platform?.appState)],
      ["log", textOf(platform?.log) || "info"],
      ["devtools", platform?.devtools === true ? "1" : platform ? "0" : "-"],
      ["dump", textOf(platform?.dump) || "-"],
      ["hostshell", textOf(versions?.hostshell)],
      ["engine", textOf(versions?.enginePin)],
      ["quickjs", textOf(versions?.quickjsPin)],
      ["memory", memory],
      ["sourcemap", "见错误页堆栈"],
    ];
    setRows(out);
  });
  return (
    <column fill gap={2}>
      <For each={rows()}>
        {(row) => (
          <Text fontSize={10} color={theme().color.text}>
            {row[0]}: {row[1]}
          </Text>
        )}
      </For>
      <Show when={rows().length === 0}>
        <Text fontSize={10} color={theme().color.textMuted}>
          读取中…
        </Text>
      </Show>
    </column>
  );
}
