import { createMemo, createSignal, onCleanup, For, Show } from "solid-js";
import { onFrame, scene, type CanvasRenderingContext2D, type NodeHandle } from "@impasto/solid-renderer";
import { Canvas, Row, type CanvasController } from "@impasto/solid-ui";
import { loc } from "../../copy.ts";
import { fmtPrice } from "../../format.ts";
import { marketOpen, minute, series, type MinuteBar, type Period } from "../../engine/analytics.ts";
import type { QuoteEngine } from "../../engine/engine.ts";
import type { Bar } from "../../engine/types.ts";
import { clock, colors, compact, Legend, Section, Segments, sign, tone } from "./ui.tsx";

const periods = ["intraday", "1m", "5m", "15m", "1D"] as const;
const averages = [[5, colors.lime], [10, colors.blue], [20, colors.amber]] as const;
const stockTime = (t: number) => clock(marketOpen * minute + t, false);

export function stroke(ctx: CanvasRenderingContext2D, color: string, x0: number, y0: number, x1: number, y1: number): void {
  ctx.strokeStyle = color;
  ctx.beginPath();
  ctx.moveTo(x0, y0);
  ctx.lineTo(x1, y1);
  ctx.stroke();
}

export function dot(ctx: CanvasRenderingContext2D, color: string, x: number, y: number, radius: number): void {
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.arc(x, y, radius, 0, Math.PI * 2);
  ctx.fill();
}

function path(ctx: CanvasRenderingContext2D, count: number, x: (index: number) => number, y: (index: number) => number): void {
  ctx.beginPath();
  ctx.moveTo(x(0), y(0));
  for (let index = 1; index < count; index++) ctx.lineTo(x(index), y(index));
}

function average(bars: readonly Bar[], index: number, length: number): number | undefined {
  if (index < length - 1) return undefined;
  let sum = 0;
  for (let offset = 0; offset < length; offset++) sum += bars[index - offset]!.c;
  return sum / length;
}

function shading(ctx: CanvasRenderingContext2D, width: number, from: number, top: number, height: number): void {
  if (from >= width) return;
  ctx.save();
  ctx.beginPath();
  ctx.rect(from, top, width - from, height);
  ctx.clip();
  ctx.strokeStyle = colors.line;
  ctx.beginPath();
  for (let x = from - height; x < width; x += 7) {
    ctx.moveTo(x, top + height);
    ctx.lineTo(x + height, top);
  }
  ctx.stroke();
  ctx.restore();
}

export function StockChart(props: { engine: QuoteEngine; symbol: string; revision: number; height: number }) {
  const [period, setPeriod] = createSignal<Period>("intraday");
  const [cursor, setCursor] = createSignal<number | null>(null);
  const [startOffset, setStartOffset] = createSignal(0);
  const [slot, setSlot] = createSignal(6);
  let controller: CanvasController | null = null;
  let node: NodeHandle | null = null;
  let hovered = false;
  let dragging = false;
  const intraday = () => period() === "intraday";
  const data = createMemo(() => {
    props.revision;
    return series(period(), props.engine.analytics(props.symbol).minutes, props.engine.bars(props.symbol), props.engine.quote(props.symbol));
  });
  const focusedIndex = () => Math.min(data().length - 1, cursor() ?? data().length - 1);
  const focus = () => data()[focusedIndex()];
  const ref = () => {
    props.revision;
    return props.engine.quote(props.symbol).prevClose;
  };
  let hit: ((x: number) => number | null) | undefined;
  const pick = () => {
    const point = controller?.toLocal(scene.pointerPosition());
    if (point) setCursor(hit?.(point.x) ?? null);
  };
  onCleanup(onFrame(() => { if (hovered || dragging) pick(); }));
  const select = (next: Period) => {
    setCursor(null);
    setStartOffset(0);
    setPeriod(next);
  };

  const draw = (ctx: CanvasRenderingContext2D, { width, height }: { width: number; height: number }) => {
    const bars = data();
    if (width < 100 || height < 90 || bars.length === 0) return;
    const intradayChart = intraday();
    const plot = width - 52;
    const top = 8;
    const bottom = height - 57;
    const volumeTop = bottom + 10;
    const volumeBottom = height - 18;
    const count = Math.max(1, Math.floor(plot / slot()));
    const end = intradayChart ? bars.length : Math.max(Math.min(bars.length, count), bars.length - startOffset());
    const first = intradayChart ? 0 : Math.max(0, end - count);
    const visible = bars.slice(first, end);
    const span = Math.max(60 * minute, (bars.at(-1)!.t - bars[0]!.t) * 1.06);
    const x = (index: number) => intradayChart ? (visible[index]!.t - bars[0]!.t) / span * plot : (index + 0.5) * plot / visible.length;
    hit = (point) => {
      if (point < 0 || point > plot) return null;
      if (!intradayChart) return first + Math.min(visible.length - 1, Math.floor(point / plot * visible.length));
      let nearest = 0;
      while (nearest + 1 < visible.length && (x(nearest) + x(nearest + 1)) / 2 <= point) nearest++;
      return nearest;
    };
    const previous = ref();
    const values = visible.map((bar) => bar.c);
    const low = intradayChart ? Math.min(previous, ...values, ...visible.map((bar) => (bar as MinuteBar).avg)) : Math.min(...visible.map((bar) => bar.l));
    const high = intradayChart ? Math.max(previous, ...values, ...visible.map((bar) => (bar as MinuteBar).avg)) : Math.max(...visible.map((bar) => bar.h));
    const margin = Math.max(previous * 0.0005, (high - low) * 0.08, 0.01);
    const range = high - low + 2 * margin;
    const y = (price: number) => top + (high + margin - price) / range * (bottom - top);
    ctx.font = "10px Sans";
    ctx.textBaseline = "middle";
    ctx.textAlign = "right";
    for (let line = 0; line < 5; line++) {
      const axisY = top + line * (bottom - top) / 4;
      stroke(ctx, colors.line, 0, axisY, plot, axisY);
      const price = high + margin - line * range / 4;
      ctx.fillStyle = colors.faint;
      ctx.fillText(intradayChart ? `${sign((price / previous - 1) * 100)}%` : fmtPrice(price), width, axisY, 49);
    }
    const last = visible.at(-1)!;
    const lastIndex = visible.length - 1;
    const lastTone = tone(last.c - previous);
    if (intradayChart) {
      shading(ctx, plot, x(lastIndex) + 4, top, bottom - top);
      ctx.setLineDash([3, 4]);
      stroke(ctx, colors.faint, 0, y(previous), plot, y(previous));
      ctx.setLineDash([]);
      path(ctx, visible.length, x, (index) => y(visible[index]!.c));
      ctx.lineTo(x(lastIndex), bottom);
      ctx.lineTo(x(0), bottom);
      ctx.closePath();
      const gradient = ctx.createLinearGradient(0, top, 0, bottom);
      gradient.addColorStop(0, `${lastTone}43`);
      gradient.addColorStop(1, `${lastTone}00`);
      ctx.fillStyle = gradient;
      ctx.fill();
      path(ctx, visible.length, x, (index) => y(visible[index]!.c));
      ctx.strokeStyle = lastTone;
      ctx.lineWidth = 1.5;
      ctx.lineJoin = "round";
      ctx.stroke();
      path(ctx, visible.length, x, (index) => y((visible[index] as MinuteBar).avg));
      ctx.strokeStyle = colors.amber;
      ctx.lineWidth = 1;
      ctx.stroke();
      dot(ctx, `${lastTone}30`, x(lastIndex), y(last.c), 8);
      dot(ctx, lastTone, x(lastIndex), y(last.c), 3);
    } else {
      const body = Math.max(1, plot / visible.length * 0.63);
      visible.forEach((bar, index) => {
        const px = x(index);
        const color = tone(bar.c - bar.o);
        const bodyY = Math.min(y(bar.o), y(bar.c));
        const bodyHeight = Math.max(1, Math.abs(y(bar.o) - y(bar.c)));
        stroke(ctx, color, px, y(bar.h), px, y(bar.l));
        if (bar.c > bar.o && body >= 3 && bodyHeight >= 3) {
          ctx.fillStyle = colors.bg;
          ctx.fillRect(px - body / 2, bodyY, body, bodyHeight);
          ctx.strokeStyle = color;
          ctx.strokeRect(px - body / 2, bodyY, body, bodyHeight);
        } else {
          ctx.fillStyle = color;
          ctx.fillRect(px - body / 2, bodyY, body, bodyHeight);
        }
      });
      ctx.save();
      ctx.beginPath();
      ctx.rect(0, top, plot, bottom - top);
      ctx.clip();
      for (const [length, color] of averages) {
        const from = Math.max(0, length - 1 - first);
        if (from >= visible.length) continue;
        path(ctx, visible.length - from, (index) => x(index + from), (index) => y(average(bars, first + from + index, length)!));
        ctx.strokeStyle = color;
        ctx.stroke();
      }
      ctx.restore();
      ctx.setLineDash([3, 3]);
      stroke(ctx, `${lastTone}90`, 0, y(last.c), plot, y(last.c));
      ctx.setLineDash([]);
    }
    const volumeScale = Math.max(1, ...visible.map((bar) => bar.v));
    const volumeWidth = Math.max(1, plot / (intradayChart ? span / minute : visible.length) * 0.73);
    visible.forEach((bar, index) => {
      const barHeight = bar.v / volumeScale * (volumeBottom - volumeTop);
      ctx.fillStyle = `${tone(bar.c - (index > 0 ? visible[index - 1]!.c : bar.o))}88`;
      ctx.fillRect(Math.max(0, x(index) - volumeWidth / 2), volumeBottom - barHeight, volumeWidth, barHeight);
    });
    stroke(ctx, colors.line, 0, volumeBottom, plot, volumeBottom);
    const latest = bars.at(-1)!;
    const label = (bar: Bar) => period() !== "1D" ? stockTime(bar.t) : bar === latest ? loc("今日", "Today")
      : loc(`${bars.length - 1 - bars.indexOf(bar)}天前`, `${bars.length - 1 - bars.indexOf(bar)}d ago`);
    const axisTimes = intradayChart ? [stockTime(bars[0]!.t), stockTime(bars[0]!.t + span / 2), stockTime(bars[0]!.t + span)]
      : [label(visible[0]!), label(visible[Math.floor(lastIndex / 2)]!), label(last)];
    ctx.fillStyle = colors.faint;
    for (const [index, align] of ["left", "center", "right"].entries()) {
      ctx.textAlign = align as "left" | "center" | "right";
      ctx.fillText(axisTimes[index]!, index * plot / 2, height - 7, plot / 3);
    }
    const priceY = Math.max(top + 8, Math.min(bottom - 8, y(last.c)));
    ctx.fillStyle = lastTone;
    ctx.beginPath();
    ctx.roundRect(plot + 3, priceY - 8, 49, 16, 5);
    ctx.fill();
    ctx.fillStyle = colors.bg;
    ctx.textAlign = "center";
    ctx.fillText(fmtPrice(last.c), plot + 27, priceY, 45);

    const focusIndex = cursor();
    if (focusIndex === null || focusIndex < first || focusIndex >= end) return;
    const bar = bars[focusIndex]!;
    const px = x(focusIndex - first);
    const py = y(bar.c);
    ctx.setLineDash([3, 3]);
    stroke(ctx, colors.dim, px, top, px, volumeBottom);
    stroke(ctx, colors.dim, 0, py, plot, py);
    ctx.setLineDash([]);
    dot(ctx, lastTone, px, py, 3);
    const tooltipWidth = Math.min(132, plot - 4);
    const tooltipX = px > plot / 2 ? Math.max(1, px - tooltipWidth - 8) : Math.min(plot - tooltipWidth, px + 8);
    ctx.fillStyle = colors.raised;
    ctx.beginPath();
    ctx.roundRect(tooltipX, top + 4, tooltipWidth, intradayChart ? 43 : 69, 6);
    ctx.fill();
    ctx.fillStyle = colors.ink;
    ctx.textAlign = "left";
    ctx.fillText(`${label(bar)} · ${fmtPrice(bar.c)}`, tooltipX + 8, top + 16, tooltipWidth - 16);
    if (!intradayChart) {
      ctx.fillText(`${loc("开", "O")} ${fmtPrice(bar.o)}  ${loc("高", "H")} ${fmtPrice(bar.h)}`, tooltipX + 8, top + 31, tooltipWidth - 16);
      ctx.fillText(`${loc("低", "L")} ${fmtPrice(bar.l)}  ${loc("收", "C")} ${fmtPrice(bar.c)}`, tooltipX + 8, top + 45, tooltipWidth - 16);
    }
    ctx.fillStyle = colors.dim;
    ctx.fillText(`${loc("成交量", "Volume")} ${compact(bar.v)}`, tooltipX + 8, top + (intradayChart ? 32 : 60), tooltipWidth - 16);
  };

  return <Section>
    <Segments options={periods} selected={period()} label={(item) => item === "intraday" ? loc("分时", "Intraday") : item} onSelect={select} />
    <Row style={{ flexWrap: "wrap", gap: 12, minHeight: 16 }}>
      <Show when={intraday()} fallback={<For each={averages}>{([length, color]) => <Legend color={color} label={`MA${length}`}
        value={fmtPrice(average(data(), focusedIndex(), length) ?? focus()?.c ?? 0)} />}</For>}>
        <Legend label={loc("价格", "Price")} value={fmtPrice(focus()?.c ?? 0)} color={tone((focus()?.c ?? ref()) - ref())} />
        <Legend label={loc("均价", "Avg")} value={fmtPrice((focus() as MinuteBar | undefined)?.avg ?? 0)} color={colors.amber} />
        <Legend label={loc("量", "Vol")} value={compact(focus()?.v ?? 0)} color={colors.faint} />
      </Show>
    </Row>
    <Canvas style={{ height: props.height, width: "100%" }} accessibilityLabel={loc(`${props.symbol} 走势图`, `${props.symbol} chart`)}
      ref={(next) => { node = next; if (node) scene.setDragClaim(node, "horizontal"); }} controller={(value) => { controller = value; }}
      onHover={() => { hovered = true; pick(); }} onLeave={() => { hovered = false; if (!dragging) setCursor(null); }}
      onPress={pick} onRelease={() => { if (!hovered && !dragging) setCursor(null); }}
      onDragBegin={() => { dragging = true; pick(); }} onDragBy={pick}
      onDragEnd={() => { dragging = false; if (!hovered) setCursor(null); }}
      onWheel={(dx) => {
        if (intraday() || Math.abs(dx) < 1) return;
        setCursor(null);
        setStartOffset((value) => Math.max(0, Math.min(data().length - 1, value + Math.round(-dx / slot()))));
      }} onZoom={(scale) => { if (!intraday()) setSlot((value) => Math.max(3, Math.min(18, value * scale))); }}
      draw={draw} />
  </Section>;
}
