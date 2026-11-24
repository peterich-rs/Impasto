/** Performance page: frame bars plus the selected frame's phase table. */
import { createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import { Button, Text, useTheme } from "@impasto/solid-ui";
import { frames, store, traceResult, type FrameSample } from "../store.ts";
const BAR_MAX = 96;

/** Focused frame facts; each phase row is one line of the table. */
function rowsOf(frame: FrameSample): [string, string][] {
  const phase = frame.phases;
  const rows: [string, string][] = [
    ["帧号", String(frame.number)],
    ["端到端 / 帧预算", `${frame.e2eMs.toFixed(2)} / ${frame.budgetMs.toFixed(2)} ms`],
    ["UI / 排队 / Raster", `${frame.uiMs.toFixed(2)} / ${frame.queueMs.toFixed(2)} / ${frame.rasterMs.toFixed(2)} ms`],
    ["Draw / Pass", `${frame.draws} / ${frame.passes}`],
    ["GPU", frame.gpuStatus],
    ["session", `${frame.sessionMs.toFixed(2)}ms`],
    ["gesture", `${frame.gestureMs.toFixed(2)}ms`],
    ["script", `${frame.scriptMs.toFixed(2)}ms`],
    ["layout", `${frame.layoutMs.toFixed(2)}ms`],
    ["record", `${frame.recordMs.toFixed(2)}ms`],
    ["submit", `${frame.submitMs.toFixed(2)}ms`],
    ["present", `${frame.presentMs.toFixed(2)}ms`],
    ["layers", String(frame.layers)],
    ["glyphs/pictures/textures", `${frame.atlasEntries}/${frame.pictures}/${frame.textures}`],
    ["shapes built", String(frame.shapesBuilt ?? 0)],
    ["atlas hits", String(frame.atlasHits ?? 0)],
    ["paint hit/miss", `${frame.paintHit ?? 0}/${frame.paintMiss ?? 0}`],
    ["picture", `${frame.pictureRebuilt ?? 0}/${frame.pictureReused ?? 0}`],
    ["cause", frame.layoutCause ?? ""],
  ];
  if (frame.compositorOnly) {
    rows.splice(1, 0, ["合成线程", `Raster 托管 · ${Math.round(1000 / frame.budgetMs)} Hz`]);
  } else if (frame.compositorBlock) {
    const node = frame.compositorBlockNode ? ` · 节点 #${frame.compositorBlockNode}` : "";
    const hint = frame.compositorBlock === "scroll_callback"
      ? " · 可使用 useScrollThreshold 或 bindScrollStyle" : "";
    rows.splice(1, 0, ["合成回退", frame.compositorBlock + node + hint]);
  }
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
      ["canvas", `${phase.canvasRecordMs.toFixed(2)}/${phase.canvasPrepareMs.toFixed(2)}ms`],
      ["ops/seg", `${phase.canvasOps}/${phase.canvasSegments}`],
      ["redraw/reused", `${phase.canvasRedraws}/${phase.canvasReused}`],
    );
  }
  return rows;
}
export function PerformancePage() {
  const theme = useTheme();
  const [selected, setSelected] = createSignal<number>(-1);
  store.sampleFrames();
  const peak = createMemo(() =>
    frames().reduce(
      (max, frame) =>
        Math.max(max, frame.e2eMs),
      0.01,
    ),
  );
  const current = () => frames().find((frame) => frame.seq === selected());
  const gpu = createMemo(() => {
    const frame = current();
    return frame?.gpuStatus === "ready" ? store.gpuTiming(frame.number) : null;
  });
  let poll: ReturnType<typeof setTimeout> | undefined;
  const refreshPending = () => {
    clearTimeout(poll);
    if (current()?.gpuStatus !== "pending") return;
    poll = setTimeout(() => {
      store.sampleFrames();
      refreshPending();
    }, 200);
  };
  onCleanup(() => clearTimeout(poll));
  const overCount = () => frames().filter((frame) => frame.overBudget).length;
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
      <Button label="刷新快照" onClick={store.sampleFrames} />
      <Button label="录制 5 秒并导出 Trace" onClick={() => { void store.recordTrace(); }} />
      <Show when={traceResult()}>
        {(result) => <Text style={{ fontSize: 11, color: theme().color.textMuted }}>{result()}</Text>}
      </Show>
      <Text
        style={{
          fontSize: 11,
          color: theme().color.textMuted,
        }}
      >
        {frames().length} 帧 · 超预算 {overCount()} · 峰值 {peak().toFixed(1)}ms
      </Text>
      <view
        style={{
          flexDirection: "row",
          gap: 1,
          alignItems: "start",
        }}
      >
        <For each={frames()}>
          {(frame) => {
            const height = Math.max(2, Math.round((frame.e2eMs / peak()) * BAR_MAX));
            return (
              <view onClick={() => { setSelected(frame.seq); refreshPending(); }} focusable={false}>
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
            <Show when={gpu()} keyed>
              {(timing) => (
                <view>
                  <Text>GPU 区间 {timing.frameSpanUs.toFixed(1)} μs · Pass 合计 {timing.passesSumUs.toFixed(1)} μs</Text>
                  <For each={timing.passes}>
                    {(pass) => (
                      <Text style={{ fontSize: 10, color: theme().color.text }}>
                        {pass.kind}{pass.sliceIndex === null ? "" : ` · 切片 ${pass.sliceIndex}`} · {pass.draws} Draw · {pass.durationUs.toFixed(1)} μs
                      </Text>
                    )}
                  </For>
                </view>
              )}
            </Show>
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
