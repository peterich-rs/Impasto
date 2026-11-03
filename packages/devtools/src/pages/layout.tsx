/** Layout page: overflow list with parent/child highlight. */
import { createSignal, For, Show } from "solid-js";
import { Text, useTheme } from "@impasto/solid-ui";
import { host } from "@impasto/solid-renderer";
import { setHighlights } from "../store.ts";

type LayoutIssueLine = {
  parent: number;
  axis: string;
  parentSize: number;
  used: number;
  excess: number;
  worstChild: number;
};

function parseIssues(text: string | null): LayoutIssueLine[] {
  if (typeof text !== "string") {
    return [];
  }
  try {
    const parsed = JSON.parse(text) as { issues?: LayoutIssueLine[] };
    return Array.isArray(parsed.issues) ? parsed.issues : [];
  } catch {
    return [];
  }
}

export function LayoutPage() {
  const theme = useTheme();
  const [issues, setIssues] = createSignal<LayoutIssueLine[]>([]);
  const [selected, setSelected] = createSignal<LayoutIssueLine | null>(null);
  const [note, setNote] = createSignal("");
  const refresh = () => {
    setIssues(parseIssues(host.debugLayoutIssues?.() ?? null));
  };
  const focus = (issue: LayoutIssueLine) => {
    setSelected(issue);
    const boxes: { x: number; y: number; w: number; h: number; color: string }[] = [];
    const notes: string[] = [];
    for (const [label, raw, color] of [
      ["父", issue.parent, "#ff3b30"],
      ["子", issue.worstChild, "#ffcc00"],
    ] as [string, number, string][]) {
      try {
        const rect = host.rect(raw);
        if (rect) {
          boxes.push({ x: rect.x, y: rect.y, w: rect.w, h: rect.h, color });
          notes.push(
            `${label} (${rect.x.toFixed(0)}, ${rect.y.toFixed(0)}, ${rect.w.toFixed(0)}×${rect.h.toFixed(0)})`,
          );
        }
      } catch {
        /* retired node: skip its box */
      }
    }
    setHighlights(boxes);
    setNote(notes.join("  "));
  };
  return (
    <column fill>
      <row gap={8}>
        <pointer onClick={refresh} focusable>
          <decorated color={theme().color.primary} radius={6}>
            <padding edges={6}>
              <Text fontSize={11} color={theme().color.onPrimary}>
                刷新
              </Text>
            </padding>
          </decorated>
        </pointer>
        <Text fontSize={11} color={theme().color.textMuted}>
          {issues().length} 处溢出
        </Text>
      </row>
      <column gap={2} grow>
        <For each={issues()}>
          {(issue) => (
            <pointer onClick={() => focus(issue)}>
              <decorated color={selected() === issue ? theme().color.primary : theme().color.surface} radius={4}>
                <padding edges={4}>
                  <Text fontSize={10} color={theme().color.text}>
                    {issue.axis === "horizontal" ? "row" : "column"} 溢出 {issue.excess.toFixed(1)}pt（内容 {issue.used.toFixed(0)} / 容器 {issue.parentSize.toFixed(0)}）
                  </Text>
                </padding>
              </decorated>
            </pointer>
          )}
        </For>
      </column>
      <Show when={note() !== ""}>
        <Text fontSize={9} color={theme().color.textMuted}>
          {note()}
        </Text>
      </Show>
      <Text fontSize={9} color={theme().color.textMuted}>
        Scroll 内容、Visible 文本、Clip 子树的溢出按设计跳过。
      </Text>
    </column>
  );
}
