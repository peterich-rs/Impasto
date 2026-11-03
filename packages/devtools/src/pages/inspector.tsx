/** Inspector page: tree snapshot plus pointer probe with breadcrumbs. */
import { createSignal, For, Show } from "solid-js";
import { Text, useTheme } from "@impasto/solid-ui";
import { host, onCleanup, onFrame, requestFrame } from "@impasto/solid-renderer";
import { setHighlights } from "../store.ts";

type SnapshotNode = {
  id: number;
  kind: string;
  x: number;
  y: number;
  w: number;
  h: number;
  focusable: boolean;
  text?: string;
  debugOwned: boolean;
  children: (SnapshotNode | null)[];
};

type FlatRow = { depth: number; node: SnapshotNode };

function flatten(node: SnapshotNode | null, depth: number, out: FlatRow[], limit: number): void {
  if (node === null || out.length >= limit) {
    return;
  }
  out.push({ depth, node });
  for (const child of node.children) {
    flatten(child, depth + 1, out, limit);
  }
}

export function InspectorPage() {
  const theme = useTheme();
  const [rows, setRows] = createSignal<FlatRow[]>([]);
  const [selected, setSelected] = createSignal<number | null>(null);
  const [probing, setProbing] = createSignal(false);
  const [chain, setChain] = createSignal<string>("");
  const stopProbe = () => {
    if (probing()) {
      host.debugSetPointerTrace?.(false, false);
      setProbing(false);
    }
  };
  onCleanup(stopProbe);
  const refresh = () => {
    const text = host.debugSnapshot?.() ?? "null";
    try {
      const parsed = JSON.parse(text) as { root: SnapshotNode | null };
      const out: FlatRow[] = [];
      flatten(parsed.root ?? null, 0, out, 300);
      setRows(out);
    } catch {
      setRows([]);
    }
  };
  const boxOf = (id: number, color: string) => {
    try {
      const rect = host.rect(id);
      if (!rect) {
        return;
      }
      setHighlights([{ x: rect.x, y: rect.y, w: rect.w, h: rect.h, color }]);
    } catch {
      setHighlights([]);
    }
  };
  const startProbe = () => {
    host.debugSetPointerTrace?.(true, true);
    setProbing(true);
    setChain("");
    const stop = onFrame(() => {
      const last = host.debugLastPointer?.() ?? "null";
      if (typeof last !== "string") {
        return;
      }
      try {
        const parsed = JSON.parse(last) as { x: number; y: number; phase: string };
        if (parsed.phase === "up") {
          const probe = host.debugProbe?.(parsed.x, parsed.y) ?? "{}";
          const walked = JSON.parse(probe) as { chain?: { id: number; kind: string }[] };
          const chainNodes = walked.chain ?? [];
          setChain(chainNodes.map((item) => item.kind).join(" → "));
          const leaf = chainNodes[chainNodes.length - 1];
          if (leaf) {
            boxOf(leaf.id, "#ff3b30");
          }
          stopProbe();
        }
      } catch {
        /* keep probing until a clean tap */
      }
      requestFrame();
    });
    onCleanup(stop);
  };
  const current = () => rows().find((row) => row.node.id === selected()) ?? null;
  return (
    <column fill>
      <row gap={8}>
        <pointer onClick={refresh} focusable>
          <decorated color={theme().color.primary} radius={6}>
            <padding edges={6}>
              <Text fontSize={11} color={theme().color.onPrimary}>
                树
              </Text>
            </padding>
          </decorated>
        </pointer>
        <pointer onClick={() => (probing() ? stopProbe() : startProbe())} focusable>
          <decorated color={probing() ? theme().color.danger : theme().color.surface} radius={6}>
            <padding edges={6}>
              <Text fontSize={11} color={probing() ? theme().color.onPrimary : theme().color.text}>
                {probing() ? "点屏幕…" : "选点"}
              </Text>
            </padding>
          </decorated>
        </pointer>
      </row>
      <Show when={chain() !== ""}>
        <Text fontSize={10} color={theme().color.text}>
          {chain()}
        </Text>
      </Show>
      <column gap={1} grow>
        <For each={rows().slice(0, 200)}>
          {(row) => (
            <pointer onClick={() => {
              setSelected(row.node.id);
              boxOf(row.node.id, "#ff3b30");
            }}>
              <decorated
                color={selected() === row.node.id ? theme().color.primary : theme().color.surface}
                radius={4}
              >
                <padding edges={4}>
                  <Text fontSize={10} color={theme().color.text} maxLines={1} ellipsis="…">
                    {"  ".repeat(row.depth)}
                    {row.node.kind} {row.node.w.toFixed(0)}×{row.node.h.toFixed(0)}
                    {row.node.text !== undefined ? ` "${row.node.text}"` : ""}
                  </Text>
                </padding>
              </decorated>
            </pointer>
          )}
        </For>
      </column>
      <Show when={current() !== null}>
        <Text fontSize={9} color={theme().color.textMuted}>
          id {current()!.node.id} · ({current()!.node.x.toFixed(0)}, {current()!.node.y.toFixed(0)}) ·{" "}
          {current()!.node.focusable ? "focusable" : "not focusable"}
        </Text>
      </Show>
    </column>
  );
}
