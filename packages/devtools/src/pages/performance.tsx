/** Performance page: frame bars plus the selected frame's phase table. */
import { createSignal, For, Show } from "solid-js";
import { Text, useTheme } from "@impasto/solid-ui";
import { frames, type FrameSample } from "../store.ts";
const BAR_MAX = 96;

/** Focused frame facts; each phase row is one line of the table. */
function rowsOf(frame: FrameSample): [string, string][] {
  const phase = frame.phases;
  const rows: [string, string][] = [
    ["帧号", String(frame.number)],
    ["session", `${frame.sessionMs.toFixed(2)}ms`],
    ["gesture", `${frame.gestureMs.toFixed(2)}ms`],
    ["script", `${frame.scriptMs.toFixed(2)}ms`],
    ["layout", `${frame.layoutMs.toFixed(2)}ms`],
    ["record", `${frame.recordMs.toFixed(2)}ms`],
    ["submit", `${frame.submitMs.toFixed(2)}ms`],
    ["present", `${frame.presentMs.toFixed(2)}ms`],
    ["layers", String(frame.layers)],
    ["para/pic/tex", `${frame.paragraphs}/${frame.pictures}/${frame.textures}`],
    ["built/bridged", `${frame.shapesBuilt ?? 0}/${frame.shapesBridged ?? 0}`],
    ["paint hit/miss", `${frame.paintHit ?? 0}/${frame.paintMiss ?? 0}`],
    ["picture", `${frame.pictureRebuilt ?? 0}/${frame.pictureReused ?? 0}`],
    ["cause", frame.layoutCause ?? ""],
  ];
  if (phase) {
    rows.push(
      ["motion", `${phase.motionMs.toFixed(2)}ms`],
      ["dispatch", `${phase.dispatchMs.toFixed(2)}ms`],
      ["timer", `${phase.timerMs.toFixed(2)}ms`],
      ["raf", `${phase.rafMs.toFixed(2)}ms`],
      ["callback", `${phase.callbackMs.toFixed(2)}ms`],
      ["flush", `${phase.flushMs.toFixed(2)}ms`],
      ["layoutcb", `${phase.layoutCbMs.toFixed(2)}ms`],
      ["laywork", `${phase.layworkMs.toFixed(2)}ms x${phase.passes}`],
      ["reuse/shaped", `${phase.reuse}/${phase.shaped}`],
    );
  }
  return rows;
}
export function PerformancePage() {
  const theme = useTheme();
  const [selected, setSelected] = createSignal<number>(-1);
  const window = () => frames().slice(-120);
  const peak = () =>
    window().reduce(
      (max, frame) =>
        Math.max(max, frame.sessionMs + frame.submitMs + frame.presentMs),
      0.01,
    );
  const current = () => window().find((frame) => frame.seq === selected());
  const overCount = () => window().filter((frame) => frame.overBudget).length;
  return (
    <view
      style={{
        width: "100%",
        height: "100%",
        flex: 1,
        flexShrink: 1,
        minWidth: 0,
        minHeight: 0,
      }}
    >
      <Text
        style={{
          fontSize: 11,
          color: theme().color.textMuted,
        }}
      >
        {window().length} 帧 · 超预算 {overCount()} · 峰值 {peak().toFixed(1)}ms
      </Text>
      <view
        style={{
          flexDirection: "row",
          gap: 1,
          alignItems: "start",
        }}
      >
        <For each={window()}>
          {(frame) => {
            const total = frame.sessionMs + frame.submitMs + frame.presentMs;
            const height = Math.max(2, Math.round((total / peak()) * BAR_MAX));
            return (
              <view onClick={() => setSelected(frame.seq)} focusable={false}>
                <view
                  style={{
                    backgroundColor: frame.overBudget
                      ? theme().color.danger
                      : theme().color.success,
                    borderRadius: 1,
                  }}
                >
                  <view
                    style={{
                      width: 4,
                      height: height,
                    }}
                  />
                </view>
              </view>
            );
          }}
        </For>
      </view>
      <Show when={current()} keyed>
        {(frame) => (
          <view
            style={{
              gap: 1,
              flex: 1,
            }}
          >
            <For each={rowsOf(frame)}>
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
          </view>
        )}
      </Show>
    </view>
  );
}
