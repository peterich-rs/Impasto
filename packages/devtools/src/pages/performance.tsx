import { View as HostView } from "@impasto/solid-renderer/host";
/** Performance page: frame bars plus the selected frame's phase table. */
import { createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import { Button, Text, useTheme } from "@impasto/solid-ui";
import { frameSnapshot, frames, store, traceResult, type FrameSample } from "../store.ts";
import { effectiveBudget, frameStageFlags, frameStageMs, summarizeFrames } from "../metrics.ts";
const BAR_MAX = 96;

/** Focused frame facts; each phase row is one line of the table. */
function rowsOf(frame: FrameSample, fallback: number): [string, string][] {
  const phase = frame.phases;
  const budget = effectiveBudget(frame, fallback);
  const flags = frameStageFlags(frame, fallback);
  const duration = (ms: number | null | undefined) => ms == null ? "—" : `${ms.toFixed(2)} ms`;
  const over = [flags.ui && "UI", flags.raster && "Raster", flags.gpu && "GPU"].filter(Boolean).join(" / ");
  const rows: [string, string][] = [
    ["帧号", String(frame.number)],
    ["触发源", frame.trigger],
    ["阶段瓶颈", duration(frameStageMs(frame))],
    ["UI / Raster / GPU", `${duration(frame.uiMs)} / ${duration(frame.rasterMs)} / ${duration(frame.gpuSpanMs)}`],
    ["端到端 / 帧预算", `${duration(frame.e2eMs)} / ${budget > 0 ? duration(budget) : "—"}`],
    ["目标截止 / 超支", `${duration(frame.targetLeadMs)} / ${duration(frame.deadlineOverrunMs)}`],
    ["阶段超预算", over || "—"],
    ["呈现", frame.presented ? "成功" : frame.skipReason ?? "失败"],
    ["排队", duration(frame.queueMs)],
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
    rows.splice(1, 0, ["合成线程", `Raster 托管${budget > 0 ? ` · ${Math.round(1000 / budget)} Hz` : ""}`]);
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
  const fallbackBudget = createMemo(() => {
    const hz = frameSnapshot()?.refreshRate;
    return hz && Number.isFinite(hz) && hz > 0 ? 1000 / hz : 0;
  });
  const stats = createMemo(() => {
    const snapshot = frameSnapshot();
    return summarizeFrames(snapshot?.entries ?? [], fallbackBudget(), snapshot);
  });
  const chartPeak = createMemo(() => frames().reduce((peak, frame) => Math.max(peak, frameStageMs(frame)), 0.01));
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
        Hitch {stats().hitchMsPerSec.toFixed(1)} ms/s · 掉帧 {stats().droppedFrames} · {stats().presentRateHz.toFixed(1)}/s · {stats().count} 帧 · 峰值 {stats().peakMs.toFixed(1)} ms
      </Text>
      <HostView
        style={{
          flexDirection: "row",
          gap: 1,
          alignItems: "end",
        }}
      >
        <For each={frames()}>
          {(frame) => {
            const height = () => Math.max(2, Math.round((frameStageMs(frame) / chartPeak()) * BAR_MAX));
            return (
              <HostView onClick={() => { setSelected(frame.seq); refreshPending(); }} focusable={false}>
                <HostView
                  style={{
                    backgroundColor: frameStageFlags(frame, fallbackBudget()).any || !frame.presented
                      ? theme().color.danger
                      : theme().color.success,
                    borderRadius: 1,
                  }}
                >
                  <HostView
                    style={{
                      width: 4,
                      height: height(),
                    }}
                  />
                </HostView>
              </HostView>
            );
          }}
        </For>
      </HostView>
      <Show when={current()} keyed>
        {(frame) => (
          <HostView
            style={{
              gap: 1,
              flex: 1,
            }}
          >
            <Show when={gpu()} keyed>
              {(timing) => (
                <HostView>
                  <Text>GPU 区间 {timing.frameSpanUs.toFixed(1)} μs · Pass 合计 {timing.passesSumUs.toFixed(1)} μs</Text>
                  <For each={timing.passes}>
                    {(pass) => (
                      <Text style={{ fontSize: 10, color: theme().color.text }}>
                        {pass.kind}{pass.sliceIndex === null ? "" : ` · 切片 ${pass.sliceIndex}`} · {pass.draws} Draw · {pass.durationUs.toFixed(1)} μs
                      </Text>
                    )}
                  </For>
                </HostView>
              )}
            </Show>
            <For each={rowsOf(frame, fallbackBudget())}>
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
          </HostView>
        )}
      </Show>
    </HostView>
  );
}
