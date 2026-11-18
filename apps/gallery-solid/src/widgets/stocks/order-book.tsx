import { For } from "solid-js";
import { Canvas, Column, Pressable, Role, Row, View } from "@impasto/solid-ui";
import { loc } from "../../copy.ts";
import { fmtPrice } from "../../format.ts";
import { type book, type Level } from "../../engine/analytics.ts";
import { Action, colors, compact, Label, Legend, Rule, Section, sign, tone } from "./ui.tsx";
import { stroke } from "./chart.tsx";

type Book = ReturnType<typeof book>;
type Levels = 5 | 10;
const indices = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
const right = { textAlign: "right", width: 58 } as const;

export function OrderBook(props: { book: Book; count: Levels; previous: number; onToggle: () => void; onLevel: (side: "buy" | "sell", price: number) => void }) {
  const levels = () => indices.slice(0, props.count);
  const bids = () => props.book.bids[props.count - 1]!.total;
  const asks = () => props.book.asks[props.count - 1]!.total;
  const share = () => Math.round(bids() / (bids() + asks()) * 100);
  const levelRow = (read: () => Level, side: "bid" | "ask", number: number) => (
    <Pressable role={Role.Button} onClick={() => props.onLevel(side === "bid" ? "sell" : "buy", read().price)}
      style={{ height: 24, flexDirection: "row", alignItems: "center", gap: 8 }}>
      <View style={{ position: "absolute", top: 1, bottom: 1, right: 0,
        width: `${read().total / Math.max(bids(), asks()) * 57}%`,
        backgroundColor: `${side === "bid" ? colors.up : colors.down}17` }} />
      <Label style={{ width: 28 }} size={11} color={colors.faint}>{`${side === "bid" ? "B" : "A"}${number}`}</Label>
      <Label style={{ flex: 1 }} color={tone(read().price - props.previous)}>{fmtPrice(read().price)}</Label>
      <Label style={right}>{compact(read().size)}</Label>
      <Label style={right} size={11} color={colors.faint}>{compact(read().total)}</Label>
    </Pressable>
  );
  return <Section title={loc("盘口", "Order book")} action={<Action label={props.count === 5 ? "5 → 10" : "10 → 5"}
    title={props.count === 5 ? loc("显示十档", "Show ten levels") : loc("显示五档", "Show five levels")} onClick={props.onToggle} />}>
    <Column style={{ gap: 5 }}>
      <Row style={{ height: 6, backgroundColor: colors.down, borderRadius: 3, overflow: "hidden" }}>
        <View style={{ width: `${share()}%`, height: 6, backgroundColor: colors.up }} />
      </Row>
      <Row style={{ justifyContent: "space-between", gap: 4 }}>
        <Label size={11} color={colors.up}>{loc(`委买 ${share()}%`, `BID ${share()}%`)}</Label>
        <Label size={11} color={colors.faint}>{loc(`价差 ${fmtPrice(props.book.asks[0]!.price - props.book.bids[0]!.price)}`, `spread ${fmtPrice(props.book.asks[0]!.price - props.book.bids[0]!.price)}`)}</Label>
        <Label size={11} color={colors.down}>{loc(`${100 - share()}% 委卖`, `${100 - share()}% ASK`)}</Label>
      </Row>
    </Column>
    <Column style={{ gap: 0 }}>
      <Row style={{ height: 22, gap: 8 }}>
        <Label style={{ width: 28 }} size={10} color={colors.faint}>{loc("档位", "LVL")}</Label>
        <Label style={{ flex: 1 }} size={10} color={colors.faint}>{loc("价格", "PRICE")}</Label>
        <Label style={right} size={10} color={colors.faint}>{loc("数量", "SIZE")}</Label>
        <Label style={right} size={10} color={colors.faint}>{loc("累计", "TOTAL")}</Label>
      </Row>
      <For each={levels().reverse()}>{(index) => levelRow(() => props.book.asks[index]!, "ask", index + 1)}</For>
      <Rule />
      <For each={levels()}>{(index) => levelRow(() => props.book.bids[index]!, "bid", index + 1)}</For>
    </Column>
  </Section>;
}

export function Depth(props: { book: Book }) {
  const totalBid = () => props.book.bids.at(-1)!.total;
  const totalAsk = () => props.book.asks.at(-1)!.total;
  return <Section title={loc("深度", "Depth")} note={loc("±12 档 · 累计", "±12 ticks · cumulative")}>
    <Canvas style={{ width: "100%", height: 115 }} accessibilityLabel={loc("累计深度图", "Cumulative depth chart")}
      draw={(ctx, { width, height }) => {
        if (width < 40) return;
        const bottom = height - 17;
        const mid = width / 2;
        const gap = 6;
        const lane = mid - gap;
        const max = Math.max(1, totalBid(), totalAsk());
        for (let tick = 0; tick <= 3; tick++) stroke(ctx, colors.line, 0, bottom * tick / 3, width, bottom * tick / 3);
        ctx.setLineDash([2, 3]);
        stroke(ctx, colors.faint, mid, 0, mid, bottom);
        ctx.setLineDash([]);
        for (const [levels, direction, color] of [[props.book.bids, -1, colors.up], [props.book.asks, 1, colors.down]] as const) {
          ctx.beginPath();
          const origin = mid + gap * direction;
          ctx.moveTo(origin, bottom);
          for (let index = 0; index < levels.length; index++) {
            const at = origin + direction * lane * index / levels.length;
            const y = bottom - levels[index]!.total / max * (bottom - 5);
            ctx.lineTo(at, y);
            ctx.lineTo(origin + direction * lane * (index + 1) / levels.length, y);
          }
          ctx.lineTo(origin + direction * lane, bottom);
          ctx.closePath();
          ctx.fillStyle = `${color}25`;
          ctx.fill();
          ctx.strokeStyle = color;
          ctx.lineWidth = 1.4;
          ctx.stroke();
        }
        ctx.font = "11px Sans";
        ctx.textBaseline = "middle";
        ctx.fillStyle = colors.faint;
        ctx.textAlign = "left";
        ctx.fillText(fmtPrice(props.book.bids.at(-1)!.price), 0, height - 6);
        ctx.textAlign = "right";
        ctx.fillText(fmtPrice(props.book.asks.at(-1)!.price), width, height - 6);
        ctx.textAlign = "center";
        ctx.fillStyle = colors.ink;
        ctx.fillText(`${loc("中间价", "mid")} ${fmtPrice(props.book.mid)}`, mid, height - 6);
      }} />
    <Row style={{ gap: 12, flexWrap: "wrap" }}>
      <Legend label={loc("委买", "Bids")} value={compact(totalBid())} color={colors.up} />
      <Legend label={loc("委卖", "Asks")} value={compact(totalAsk())} color={colors.down} />
      <Legend label={loc("失衡", "Imbalance")} value={`${sign((totalBid() - totalAsk()) / (totalBid() + totalAsk()) * 100, 0)}%`} color={colors.line} />
    </Row>
  </Section>;
}
