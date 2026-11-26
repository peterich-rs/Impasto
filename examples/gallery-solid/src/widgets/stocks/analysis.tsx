import { createMemo } from "solid-js";
import { Canvas, Column, Row } from "@impasto/solid-ui";
import { loc } from "../../copy.ts";
import { fmtPrice } from "../../format.ts";
import { blockSize, marketOpen, minute, type Analytics } from "../../engine/analytics.ts";
import type { Bar, Quote, Tick } from "../../engine/types.ts";
import { clock, colors, compact, Label, Legend, Section, useLegendNumberStyle } from "./ui.tsx";
import { dot, stroke } from "./chart.tsx";

export function Tape(props: { ticks: readonly Tick[]; rows: number; dayStart: number }) {
  const numberStyle = useLegendNumberStyle();
  const totals = createMemo(() => {
    const result = { buy: 0, sell: 0, blocks: 0 };
    for (const tick of props.ticks) {
      if (tick.side) result[tick.side] += tick.volume;
      if (tick.volume >= blockSize) result.blocks++;
    }
    return result;
  });
  return <Section title={loc("逐笔成交", "Time & sales")} note={loc("大单 ≥ 8,000", "blocks ≥ 8,000")}>
    <Row style={{ gap: 12, alignItems: "baseline", flexWrap: "wrap" }}>
      <Legend label={loc("外盘", "At ask")} value={compact(totals().buy)} color={colors.up} valueStyle={numberStyle()} />
      <Legend label={loc("内盘", "At bid")} value={compact(totals().sell)} color={colors.down} valueStyle={numberStyle()} />
      <Legend label={loc("大单", "Blocks")} value={String(totals().blocks)} color={colors.lime} valueStyle={numberStyle()} />
    </Row>
    <Canvas style={{ width: "100%", height: 20 + 25 * Math.max(1, props.rows) }}
      accessibilityLabel={loc("逐笔成交明细：时间、价格和数量", "Trades: time, price and size")}
      draw={(ctx, { width }) => {
        const ticks = props.ticks;
        for (const [index, tick] of ticks.entries()) {
          const y = 32 + index * 25;
          dot(ctx, tick.side === "buy" ? colors.up : colors.down, 70, y, 2.5);
          stroke(ctx, colors.line, 0, y + 12, width, y + 12);
        }
        const time = (tick: Tick) => clock(marketOpen * minute + tick.t - props.dayStart);
        ctx.textBaseline = "middle";
        ctx.font = "10px Sans";
        ctx.fillStyle = colors.faint;
        ctx.fillText(loc("时间", "TIME"), 0, 9);
        ctx.fillText(loc("价格", "PRICE"), 80, 9);
        ctx.textAlign = "right";
        ctx.fillText(loc("数量", "SIZE"), width, 9);
        if (ticks.length === 0) {
          ctx.textAlign = "left";
          ctx.fillText(loc("暂无成交", "No trades yet"), 0, 32);
        }
        for (const [index, tick] of ticks.entries()) {
          const y = 32 + index * 25;
          ctx.textAlign = "left";
          ctx.fillStyle = colors.faint;
          ctx.font = "11px Sans";
          ctx.fillText(time(tick), 0, y);
          ctx.font = "13px Sans";
          ctx.fillText(fmtPrice(tick.price), 80, y);
          ctx.fillStyle = tick.volume >= blockSize ? colors.lime : colors.ink;
          ctx.textAlign = "right";
          ctx.fillText(compact(tick.volume), width, y);
        }
      }} />
  </Section>;
}

const buckets = [0, 1, 2, 3];
const sum = (values: readonly number[]) => values.reduce((total, value) => total + value, 0);
const signedQty = (value: number) => `${value >= 0 ? "+" : ""}${compact(value)}`;

export function Flow(props: { data: Pick<Analytics, "buy" | "sell"> }) {
  const net = () => sum(props.data.buy) - sum(props.data.sell);
  const big = () => props.data.buy[2]! + props.data.buy[3]! - props.data.sell[2]! - props.data.sell[3]!;
  const captions = () => [loc("小单 ≤500", "Small ≤500"), loc("中单 ≤2K", "Medium ≤2K"), loc("大单 <8K", "Large <8K"), loc("超大单 ≥8K", "Block ≥8K")];
  return <Section title={loc("成交方向", "Trade flow")} note={loc("卖出 ← · → 买入", "sold ← · → bought")}>
    <Row style={{ alignItems: "baseline", gap: 6, flexWrap: "wrap" }}>
      <Label size={23} weight={600}>{signedQty(net())}</Label>
      <Label size={12} color={big() >= 0 ? colors.up : colors.down}>{loc(`净量 · ${signedQty(big())} 大单`, `net · ${signedQty(big())} large orders`)}</Label>
    </Row>
    <Canvas style={{ width: "100%", height: 168 }} accessibilityLabel={loc("按成交量分档的买卖对比", "Buy and sell volume by trade size")}
      draw={(ctx, { width }) => {
        const { buy, sell } = props.data;
        const half = width / 2;
        const lane = Math.max(0, half - 3);
        const max = Math.max(1, ...buy, ...sell);
        for (const bucket of buckets) {
          const y = 42 * bucket;
          stroke(ctx, colors.line, half, y + 17, half, y + 33);
          for (const [size, color, left] of [[sell[bucket]!, colors.down, true], [buy[bucket]!, colors.up, false]] as const) {
            const bar = size / max * lane;
            if (bar === 0) continue;
            ctx.beginPath();
            ctx.roundRect(left ? half - 3 - bar : half + 3, y + 20, bar, 10, 3);
            ctx.fillStyle = color;
            ctx.fill();
          }
        }
        const labels = captions();
        ctx.font = "12px Sans";
        ctx.textBaseline = "middle";
        for (const bucket of buckets) {
          const y = 42 * bucket;
          ctx.textAlign = "left";
          ctx.fillStyle = colors.ink;
          ctx.fillText(labels[bucket]!, 0, y + 8, half);
          ctx.textAlign = "right";
          ctx.fillStyle = colors.down;
          ctx.fillText(compact(sell[bucket]!), width - 72, y + 8);
          ctx.fillStyle = colors.faint;
          ctx.fillText("/", width - 61, y + 8);
          ctx.fillStyle = colors.up;
          ctx.fillText(compact(buy[bucket]!), width, y + 8);
        }
      }} />
  </Section>;
}

const horizons = [5, 20, 60, 120];
export function historicalRange(bars: readonly Bar[], quote: Quote, days: number): [number, number] {
  let low = quote.low;
  let high = quote.high;
  for (let i = Math.max(0, bars.length - days + 1); i < bars.length; i++) {
    const bar = bars[i]!;
    low = Math.min(low, bar.l);
    high = Math.max(high, bar.h);
  }
  return [low, high];
}

export function Ranges(props: { bars: readonly Bar[]; quote: Quote }) {
  return <Section title={loc("价格区间", "Ranges")} note={loc("最低 → 最高", "low → high")}>
    <Row style={{ gap: 8, alignItems: "stretch" }}>
      <Column style={{ width: 44, flexShrink: 0, paddingTop: 3, gap: 32, height: 154 }}>
        {horizons.map((days) => <Label size={11} color={colors.faint} style={{ height: 14 }}>{loc(`${days}日`, `${days}d`)}</Label>)}
      </Column>
      <Canvas style={{ flex: 1, minWidth: 0, height: 154 }} accessibilityLabel={loc("5 日至 120 日价格区间", "5 to 120 day price ranges")}
        draw={(ctx, { width, height }) => {
          if (width < 100) return;
          const quote = props.quote;
          const ranges = horizons.map((days) => historicalRange(props.bars, quote, days));
          const [low, high] = ranges.at(-1)!;
          // Symmetric side padding keeps the plot on the section's center line.
          const pad = 36;
          const plot = Math.max(1, width - pad * 2);
          const x = (value: number) => pad + plot * (value - low) / Math.max(high - low, 0.01);
          const end = height - 17;
          ctx.setLineDash([3, 3]);
          stroke(ctx, colors.faint, x(quote.prevClose), 0, x(quote.prevClose), end);
          ctx.setLineDash([]);
          for (const [index, [min, max]] of ranges.entries()) {
            const y = 10 + 32 * index;
            ctx.lineWidth = 3;
            stroke(ctx, colors.line, x(min), y, x(max), y);
            dot(ctx, colors.down, x(min), y, 4);
            dot(ctx, colors.up, x(max), y, 4);
          }
          ctx.lineWidth = 1.5;
          stroke(ctx, colors.lime, x(quote.last), 0, x(quote.last), end);
          ctx.font = "11px Sans";
          ctx.textBaseline = "middle";
          ctx.textAlign = "center";
          ctx.fillStyle = colors.faint;
          for (const [index, [min, max]] of ranges.entries()) {
            const y = 23 + 32 * index;
            ctx.fillText(fmtPrice(min), x(min), y, 64);
            if (x(max) - x(min) > 35) ctx.fillText(fmtPrice(max), x(max), y, 64);
          }
          ctx.fillStyle = colors.lime;
          ctx.fillText(`${loc("现价", "last")} ${fmtPrice(quote.last)}`, Math.max(pad, Math.min(width - pad, x(quote.last))), height - 6, 84);
        }} />
    </Row>
  </Section>;
}
