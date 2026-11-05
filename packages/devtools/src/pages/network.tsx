/** Network page: request list with status, timing, and redacted headers. */
import { createSignal, For, Show } from "solid-js";
import { Text, useTheme } from "@impasto/solid-ui";
import { network, type NetEntry } from "../store.ts";

function statusColor(entry: NetEntry, danger: string, success: string, muted: string): string {
  if (entry.error !== null) {
    return danger;
  }
  if (entry.status === null) {
    return muted;
  }
  return entry.status >= 400 ? danger : entry.status >= 200 && entry.status < 300 ? success : muted;
}

export function NetworkPage() {
  const theme = useTheme();
  const [selected, setSelected] = createSignal<number>(-1);
  const [onlyFailures, setOnlyFailures] = createSignal(false);
  const [onlyStream, setOnlyStream] = createSignal(false);
  const current = () => network().find((entry) => entry.seq === selected());
  const visible = () =>
    network().filter((entry) => {
      if (onlyFailures() && entry.error === null && (entry.status ?? 0) < 400) {
        return false;
      }
      if (onlyStream() && entry.kind === "http") {
        return false;
      }
      return true;
    });
  return (
    <column fill>
      <row gap={8}>
        <pointer onClick={() => setOnlyFailures((prev) => !prev)}>
          <Text fontSize={11} color={onlyFailures() ? theme().color.danger : theme().color.textMuted}>
            {onlyFailures() ? "只看失败" : "全部"}
          </Text>
        </pointer>
        <pointer onClick={() => setOnlyStream((prev) => !prev)}>
          <Text fontSize={11} color={onlyStream() ? theme().color.primary : theme().color.textMuted}>
            {onlyStream() ? "流/套接字" : "含 HTTP"}
          </Text>
        </pointer>
        <Text fontSize={11} color={theme().color.textMuted}>
          {network().length} 条
        </Text>
      </row>
      <column gap={2} grow>
        <For each={visible().slice(-50)}>
          {(entry) => (
            <pointer onClick={() => setSelected(entry.seq)}>
              <decorated color={theme().color.surface} radius={4}>
                <padding edges={4}>
                  <Text fontSize={10} color={statusColor(entry, theme().color.danger, theme().color.success, theme().color.textMuted)} maxLines={1} ellipsis="…">
                    [{entry.kind}] {entry.method || "GET"} {entry.url}
                    {entry.status !== null ? ` → ${entry.status}` : ""}
                  </Text>
                </padding>
              </decorated>
            </pointer>
          )}
        </For>
      </column>
      <Show when={current()} keyed>
        {(entry) => (
          <column gap={2}>
            <Text fontSize={11} color={theme().color.text}>
              {entry.method || "GET"} {entry.url}
            </Text>
            <Text fontSize={10} color={theme().color.textMuted}>
              状态 {entry.status ?? "-"} · {entry.doneAt !== null ? `${(entry.doneAt - entry.at).toFixed(0)}ms` : "…"}
              {entry.responseBytes !== null ? ` · ${entry.responseBytes}B` : ""}
            </Text>
            <For each={entry.requestHeaders.slice(0, 8)}>
              {(header) => (
                <Text fontSize={9} color={theme().color.textMuted}>
                  {header[0]}: {header[1]}
                </Text>
              )}
            </For>
            <Show when={entry.requestBody !== null}>
              <Text fontSize={9} color={theme().color.textMuted} maxLines={3} ellipsis="…">
                → {entry.requestBody}
              </Text>
            </Show>
            <For each={entry.responseHeaders.slice(0, 8)}>
              {(header) => (
                <Text fontSize={9} color={theme().color.textMuted}>
                  {header[0]}: {header[1]}
                </Text>
              )}
            </For>
            <Show when={entry.responsePreview !== null}>
              <Text fontSize={9} color={theme().color.textMuted} maxLines={6} ellipsis="…">
                ← {entry.responsePreview}
              </Text>
            </Show>
            <Show when={entry.error !== null}>
              <Text fontSize={9} color={theme().color.danger}>
                {entry.error}
              </Text>
            </Show>
          </column>
        )}
      </Show>
    </column>
  );
}
