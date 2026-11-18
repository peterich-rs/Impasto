import { Canvas, Row } from "@impasto/solid-ui";
import { loc } from "../../copy.ts";
import { fmtPrice } from "../../format.ts";
import { blockSize, marketOpen, minute, type Analytics } from "../../engine/analytics.ts";
import type { Bar, Quote, Tick } from "../../engine/types.ts";
import { clock, colors, compact, Label, Legend, Section } from "./ui.tsx";
import { dot, stroke } from "./chart.tsx";

export function Tape(props: { ticks: readonly Tick[]; dayStart: number }) {
  const totals = () => {
    const result = { buy: 0, sell: 0, blocks: 0 };
    for (const tick of props.ticks) {
      if (tick.side) result[tick.side] += tick.volume;
      if (tick.volume >= blockSize) result.blocks++;
    }
    return result;
  };
  return <Section title={loc("逐笔成交", "Time & sales")} note={loc("大单 ≥ 8,000", "blocks ≥ 8,000")}>
    <Row style={{ gap: 12, flexWrap: "wrap" }}>
      <Legend label={loc("外盘", "At ask")} value={compact(totals().buy)} color={colors.up} />
      <Legend label={loc("内盘", "At bid")} value={compact(totals().sell)} color={colors.down} />
      <Legend label={loc("大单", "Blocks")} value={String(totals().blocks)} color={colors.lime} />
    </Row>
    <Canvas style={{ width: "100%", height: 20 + 25 * Math.max(1, props.ticks.length) }}
      accessibilityLabel={loc("逐笔成交明细：时间、价格和数量", "Trades: time, price and size")}
      draw={(ctx, { width }) => {
        const time = (tick: Tick) => clock(marketOpen * minute + tick.t - props.dayStart);
        ctx.textBaseline = "middle";
        ctx.font = "10px Sans";
        ctx.fillStyle = colors.faint;
        ctx.fillText(loc("时间", "TIME"), 0, 9);
        ctx.fillText(loc("价格", "PRICE"), 80, 9);
        ctx.textAlign = "right";
        ctx.fillText(loc("数量", "SIZE"), width, 9);
        if (props.ticks.length === 0) {
          ctx.textAlign = "left";
          ctx.fillText(loc("暂无成交", "No trades yet"), 0, 32);
        }
        for (const [index, tick] of props.ticks.entries()) {
          const y = 32 + index * 25;
          const color = tick.side === "buy" ? colors.up : colors.down;
          ctx.textAlign = "left";
          ctx.fillStyle = colors.faint;
          ctx.font = "11px Sans";
          ctx.fillText(time(tick), 0, y);
          dot(ctx, color, 70, y, 2.5);
          ctx.font = "13px Sans";
          ctx.fillText(fmtPrice(tick.price), 80, y);
          ctx.fillStyle = tick.volume >= blockSize ? colors.lime : colors.ink;
          ctx.textAlign = "right";
          ctx.fillText(compact(tick.volume), width, y);
          stroke(ctx, colors.line, 0, y + 12, width, y + 12);
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
    <Canvas style={{ width: "100%", height: 154 }} accessibilityLabel={loc("5 日至 120 日价格区间", "5 to 120 day price ranges")}
      draw={(ctx, { width, height }) => {
        if (width < 100) return;
        const quote = props.quote;
        const ranges = horizons.map((days) => historicalRange(props.bars, quote, days));
        const [low, high] = ranges.at(-1)!;
        const x = (value: number) => 49 + (width - 81) * (value - low) / Math.max(high - low, 0.01);
        const end = height - 17;
        ctx.setLineDash([3, 3]);
        stroke(ctx, colors.faint, x(quote.prevClose), 0, x(quote.prevClose), end);
        ctx.setLineDash([]);
        ctx.font = "11px Sans";
        ctx.textBaseline = "middle";
        for (const [index, [min, max]] of ranges.entries()) {
          const y = 10 + 32 * index;
          ctx.lineWidth = 3;
          stroke(ctx, colors.line, x(min), y, x(max), y);
          dot(ctx, colors.down, x(min), y, 4);
          dot(ctx, colors.up, x(max), y, 4);
          ctx.fillStyle = colors.faint;
          ctx.textAlign = "left";
          ctx.fillText(loc(`${horizons[index]}日`, `${horizons[index]}d`), 0, y);
          ctx.textAlign = "center";
          ctx.fillText(fmtPrice(min), x(min), y + 13, 50);
          if (x(max) - x(min) > 35) ctx.fillText(fmtPrice(max), x(max), y + 13, 50);
        }
        ctx.lineWidth = 1.5;
        stroke(ctx, colors.lime, x(quote.last), 0, x(quote.last), end);
        ctx.fillStyle = colors.lime;
        ctx.textAlign = "center";
        ctx.fillText(`${loc("现价", "last")} ${fmtPrice(quote.last)}`, Math.max(42, Math.min(width - 42, x(quote.last))), height - 6, 84);
      }} />
  </Section>;
}
