import { View as HostView } from "@impasto/solid-renderer/host";
/** Layout page: overflow list with parent/child highlight. */
import { createSignal, For, Show } from "solid-js";
import { Text, useTheme } from "@impasto/solid-ui";
import { scene } from "@impasto/solid-renderer";
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
    const parsed = JSON.parse(text) as {
      issues?: LayoutIssueLine[];
    };
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
    setIssues(parseIssues(scene.debugLayoutIssues?.() ?? null));
  };
  const focus = (issue: LayoutIssueLine) => {
    setSelected(issue);
    const boxes: {
      x: number;
      y: number;
      w: number;
      h: number;
      color: string;
    }[] = [];
    const notes: string[] = [];
    for (const [label, raw, color] of [
      ["父", issue.parent, "#ff3b30"],
      ["子", issue.worstChild, "#ffcc00"],
    ] as [string, number, string][]) {
      try {
        const rect = scene.rect(raw);
        if (rect) {
          boxes.push({
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
            color,
          });
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
    <HostView
      style={{
        width: "100%",
        height: "100%",
        flex: 1,
        flexShrink: 1,
        minWidth: 0,
        minHeight: 0,
      }}
    >
      <HostView
        style={{
          flexDirection: "row",
          gap: 8,
        }}
      >
        <HostView onClick={refresh} focusable>
          <HostView
            style={{
              backgroundColor: theme().color.primary,
              borderRadius: 6,
            }}
          >
            <HostView
              style={{
                padding: 6,
              }}
            >
              <Text
                style={{
                  fontSize: 11,
                  color: theme().color.onPrimary,
                }}
              >
                刷新
              </Text>
            </HostView>
          </HostView>
        </HostView>
        <Text
          style={{
            fontSize: 11,
            color: theme().color.textMuted,
          }}
        >
          {issues().length} 处溢出
        </Text>
      </HostView>
      <HostView
        style={{
          gap: 2,
          flex: 1,
        }}
      >
        <For each={issues()}>
          {(issue) => (
            <HostView onClick={() => focus(issue)}>
              <HostView
                style={{
                  backgroundColor:
                    selected() === issue
                      ? theme().color.primary
                      : theme().color.surface,
                  borderRadius: 4,
                }}
              >
                <HostView
                  style={{
                    padding: 4,
                  }}
                >
                  <Text
                    style={{
                      fontSize: 10,
                      color: theme().color.text,
                    }}
                  >
                    {issue.axis === "horizontal" ? "row" : "column"} 溢出{" "}
                    {issue.excess.toFixed(1)}pt（内容 {issue.used.toFixed(0)} /
                    容器 {issue.parentSize.toFixed(0)}）
                  </Text>
                </HostView>
              </HostView>
            </HostView>
          )}
        </For>
      </HostView>
      <Show when={note() !== ""}>
        <Text
          style={{
            fontSize: 9,
            color: theme().color.textMuted,
          }}
        >
          {note()}
        </Text>
      </Show>
      <Text
        style={{
          fontSize: 9,
          color: theme().color.textMuted,
        }}
      >
        Scroll 内容、Visible 文本、Clip 子树的溢出按设计跳过。
      </Text>
    </HostView>
  );
}
