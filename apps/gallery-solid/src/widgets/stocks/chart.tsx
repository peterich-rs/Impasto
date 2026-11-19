import { createMemo, createSignal, createTrackedEffect, onCleanup, For, Show } from "solid-js";
import { onLayout, scene, type CanvasRenderingContext2D, type NodeHandle } from "@impasto/solid-renderer";
import { Canvas, Column, Row, View, type CanvasController } from "@impasto/solid-ui";
import { loc } from "../../copy.ts";
import { fmtPrice } from "../../format.ts";
import { series, type MinuteBar, type Period } from "../../engine/analytics.ts";
import type { StockSnapshot } from "../../engine/stock-snapshot.ts";
import { Action, colors, compact, Legend, Label, Section, Segments, tone, useLegendNumberStyle } from "./ui.tsx";
import { barLabel, buildChartGeometry, paintChart, resolveSelection } from "./chart-model.ts";

export { dot, stroke } from "./chart-model.ts";

const periods = ["intraday", "1m", "5m", "15m", "1D"] as const;
const averages = [[5, colors.lime], [10, colors.blue], [20, colors.amber]] as const;

function average(bars: readonly { c: number }[], index: number, length: number): number | undefined {
  if (index < length - 1) return undefined;
  let sum = 0;
  for (let offset = 0; offset < length; offset++) sum += bars[index - offset]!.c;
  return sum / length;
}

export function StockChart(props: { snapshot: StockSnapshot; height: number }) {
  const numberStyle = useLegendNumberStyle();
  const [period, setPeriod] = createSignal<Period>("intraday");
  const [selectedTime, setSelectedTime] = createSignal<number | null>(null);
  const [startOffset, setStartOffset] = createSignal(0);
  const [slot, setSlot] = createSignal(6);
  const [size, setSize] = createSignal<{ width: number; height: number } | null>(null);
  const symbol = createMemo(() => props.snapshot.quote.symbol);
  let controller: CanvasController | null = null;
  let canvasNode: NodeHandle | null = null;
  let hovered = false;
  let dragging = false;
  const intraday = () => period() === "intraday";
  const data = createMemo(() => {
    const { minutes, history, quote } = props.snapshot;
    return series(period(), minutes, history, quote);
  });
  const ref = createMemo(() => props.snapshot.quote.prevClose);
  const geometry = createMemo(() => {
    const box = size();
    if (!box) return null;
    return buildChartGeometry({
      bars: data(),
      period: period(),
      startOffset: startOffset(),
      slot: slot(),
      width: box.width,
      height: box.height,
      previous: ref(),
    });
  });
  const overlayBox = createMemo((previous?: { top: number; plot: number; volumeBottom: number }) => {
    const geo = geometry();
    const top = geo?.top ?? 0;
    const plot = geo?.plot ?? 0;
    const volumeBottom = geo?.volumeBottom ?? 0;
    return previous?.top === top && previous.plot === plot && previous.volumeBottom === volumeBottom
      ? previous : { top, plot, volumeBottom };
  });
  createTrackedEffect(() => {
    const time = selectedTime();
    if (time === null) return;
    const geo = geometry();
    const bars = data();
    if (!geo || resolveSelection(time, bars, geo.first, geo.end) === null) setSelectedTime(null);
  });
  const focusedIndex = () => {
    const bars = data();
    const time = selectedTime();
    if (time === null) return bars.length - 1;
    const geo = geometry();
    const selected = geo ? resolveSelection(time, bars, geo.first, geo.end) : null;
    return Math.min(bars.length - 1, selected ?? bars.length - 1);
  };
  const focus = () => data()[focusedIndex()];
  const focusPrice = createMemo(() => fmtPrice(focus()?.c ?? 0));
  const focusColor = createMemo(() => tone((focus()?.c ?? ref()) - ref()));
  const focusAverage = createMemo(() => fmtPrice((focus() as MinuteBar | undefined)?.avg ?? 0));
  const focusVolume = createMemo(() => compact(focus()?.v ?? 0));
  const stepSelection = (direction: -1 | 1) => {
    const geo = geometry();
    if (!geo) return;
    const current = resolveSelection(selectedTime(), data(), geo.first, geo.end);
    const next = current === null ? geo.end - 1 : Math.max(geo.first, Math.min(geo.end - 1, current + direction));
    setSelectedTime(data()[next]?.t ?? null);
  };
  const readout = createMemo(() => {
    const time = selectedTime();
    if (time === null) return null;
    const geo = geometry();
    const bars = data();
    const index = geo ? resolveSelection(time, bars, geo.first, geo.end) : null;
    if (!geo || index === null) return null;
    const bar = bars[index]!;
    const x = geo.xAt(index);
    const y = geo.yAt(bar.c);
    const tooltipWidth = Math.min(132, geo.plot - 4);
    return {
      x,
      y,
      boxX: x > geo.plot / 2 ? Math.max(1, x - tooltipWidth - 8) : Math.min(geo.plot - tooltipWidth, x + 8),
      title: `${barLabel(period(), bars, bar)} · ${fmtPrice(bar.c)}`,
      ohlc: geo.intraday ? [] : [
        `${loc("开", "O")} ${fmtPrice(bar.o)}  ${loc("高", "H")} ${fmtPrice(bar.h)}`,
        `${loc("低", "L")} ${fmtPrice(bar.l)}  ${loc("收", "C")} ${fmtPrice(bar.c)}`,
      ],
      volume: `${loc("成交量", "Volume")} ${compact(bar.v)}`,
      dot: tone(bar.c - geo.previous),
    };
  });
  const lastReadout = createMemo<ReturnType<typeof readout>>((previous) => readout() ?? previous ?? null, null);
  const tooltipLine = { width: "100%", height: 12, lineHeight: 1.2 } as const;
  const pick = () => {
    const point = controller?.toLocal(scene.pointerPosition());
    const geo = geometry();
    if (!point || !geo) return;
    const index = geo.hit(point.x);
    const bar = index === null ? undefined : data()[index];
    setSelectedTime(bar ? bar.t : null);
  };
  const insideChart = () => {
    const point = controller?.toLocal(scene.pointerPosition());
    const geo = geometry();
    if (!point || !geo) return false;
    return point.x >= 0 && point.y >= 0 && point.x <= geo.width && point.y <= geo.height;
  };
  const select = (next: Period) => {
    setStartOffset(0);
    setPeriod(next);
  };
  onCleanup(onLayout(() => {
    if (canvasNode === null) return;
    const box = scene.rect(canvasNode);
    setSize((previous) => previous && previous.width === box.w && previous.height === box.h ? previous : { width: box.w, height: box.h });
  }));
  // 只读几何。选中点在覆盖层上，不进入这次绘制。
  const draw = (ctx: CanvasRenderingContext2D) => {
    const geo = geometry();
    if (geo) paintChart(ctx, geo);
  };

  const plot = <Canvas deferWhenOffscreen style={{ width: "100%", height: props.height }}
    ref={(next) => { canvasNode = next; }}
    controller={(value) => { controller = value; }}
    draw={draw} />;
  return <Section>
    <Row style={{ alignItems: "center", justifyContent: "space-between", flexWrap: "wrap", gap: 4 }}>
      <Segments options={periods} selected={period()} label={(item) => item === "intraday" ? loc("分时", "Intraday") : item} onSelect={select} />
      <Row style={{ gap: 4 }}>
        <Action label="‹" title={loc("上一个数据点", "Previous chart point")} onClick={() => stepSelection(-1)} />
        <Action label="›" title={loc("下一个数据点", "Next chart point")} onClick={() => stepSelection(1)} />
      </Row>
    </Row>
    <Row style={{ alignItems: "baseline", flexWrap: "wrap", gap: 12, minHeight: 16 }}>
      <Show when={intraday()} fallback={<For each={averages}>{([length, color]) => {
        const value = createMemo(() => fmtPrice(average(data(), focusedIndex(), length) ?? focus()?.c ?? 0));
        return <Legend color={color} label={`MA${length}`} valueStyle={numberStyle()} value={value()} />;
      }}</For>}>
        <Legend label={loc("价格", "Price")} value={focusPrice()} color={focusColor()} valueStyle={numberStyle()} />
        <Legend label={loc("均价", "Avg")} value={focusAverage()} color={colors.amber} valueStyle={numberStyle()} />
        <Legend label={loc("量", "Vol")} value={focusVolume()} color={colors.faint} valueStyle={numberStyle()} />
      </Show>
    </Row>
    <View style={{ height: props.height, width: "100%" }}
        focusable
        accessibilityLabel={loc(`${symbol()} 走势图`, `${symbol()} chart`)}
        ref={(next) => { if (next) scene.setDragClaim(next, "horizontal"); }}
        onKey={(key) => {
          if (key === "Left") stepSelection(-1);
          if (key === "Right") stepSelection(1);
          if (key === "Escape") setSelectedTime(null);
        }}
        onHover={() => { hovered = true; pick(); }}
        onLeave={() => { hovered = false; if (!dragging && !insideChart()) setSelectedTime(null); }}
        onPress={pick}
        onRelease={() => { if (!hovered && !dragging) setSelectedTime(null); }}
        onDragBegin={() => { dragging = true; pick(); }}
        onDragBy={pick}
        onDragEnd={() => { dragging = false; if (!hovered) setSelectedTime(null); }}
        onWheel={(dx) => {
          if (intraday() || Math.abs(dx) < 1) return;
          setStartOffset((value) => Math.max(0, Math.min(data().length - 1, value + Math.round(-dx / slot()))));
        }}
        onZoom={(scale) => { if (!intraday()) setSlot((value) => Math.max(3, Math.min(18, value * scale))); }}>
      {plot}
      <View style={{ position: "absolute", top: 0, left: 0, width: "100%", height: props.height, opacity: readout() ? 1 : 0 }}>
        <View style={{ position: "absolute", left: 0, top: overlayBox().top,
          width: 1, height: overlayBox().volumeBottom - overlayBox().top,
          backgroundColor: colors.dim, translateX: lastReadout()?.x ?? 0 }} />
        <View style={{ position: "absolute", left: 0, top: 0, width: overlayBox().plot, height: 1,
          backgroundColor: colors.dim, translateY: lastReadout()?.y ?? 0 }} />
        <View style={{ position: "absolute", left: 0, top: 0, width: 6, height: 6, borderRadius: 3,
          backgroundColor: lastReadout()?.dot ?? colors.dim,
          translateX: (lastReadout()?.x ?? 3) - 3, translateY: (lastReadout()?.y ?? 3) - 3 }} />
        <Column style={{ position: "absolute", left: 0, top: 0,
          translateX: lastReadout()?.boxX ?? 0, translateY: overlayBox().top + 4,
          width: Math.max(0, Math.min(132, overlayBox().plot - 4)),
          height: intraday() ? 43 : 73, padding: 8, gap: 3, borderRadius: 6, backgroundColor: colors.raised }}>
          <Label size={10} style={tooltipLine}>{lastReadout()?.title ?? ""}</Label>
          <Show when={!intraday()}>
            <Label size={10} style={tooltipLine}>{lastReadout()?.ohlc[0] ?? ""}</Label>
            <Label size={10} style={tooltipLine}>{lastReadout()?.ohlc[1] ?? ""}</Label>
          </Show>
          <Label size={10} style={tooltipLine} color={colors.dim}>{lastReadout()?.volume ?? ""}</Label>
        </Column>
      </View>
    </View>
  </Section>;
}
