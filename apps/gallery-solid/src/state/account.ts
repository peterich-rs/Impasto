import { createSignal } from "solid-js";
import type { QuoteEngine } from "../engine/engine.ts";
import type { Order, OrderSide, OrderType, Position } from "../engine/types.ts";
import { round2 } from "../engine/universe.ts";

const START_CASH = 1_000_000;
const FEE_RATE = 0.0003;
const MIN_FEE = 5;
/** Orders above this many shares fill in two clips. */
const SPLIT_QTY = 4000;
const KEY = "impasto.quotes.account.v1";

type Snapshot = { cash: number; positions: Position[]; orders: Order[] };

export type SubmitResult = { ok: true; order: Order } | { ok: false; reason: string };

/**
 * Mock brokerage. Market orders fill at the latest price on the next drain;
 * limit orders sit in the book until a tick crosses them.
 */
export function createAccount(engine: QuoteEngine) {
  const [cash, setCash] = createSignal(START_CASH);
  const [positions, setPositions] = createSignal<Position[]>([]);
  const [orders, setOrders] = createSignal<Order[]>([]);

  const feeOf = (amount: number): number => Math.max(MIN_FEE, round2(amount * FEE_RATE));

  function holding(symbol: string): number {
    return positions().find((p) => p.symbol === symbol)?.qty ?? 0;
  }

  function limitBand(symbol: string): { lo: number; hi: number } {
    const q = engine.quote(symbol);
    const ins = engine.ins(symbol);
    if (ins.market === "cn") {
      return { lo: round2(q.prevClose * 0.9), hi: round2(q.prevClose * 1.1) };
    }
    const span = q.prevClose * ins.sigma * 0.05;
    return { lo: round2(q.prevClose - span), hi: round2(q.prevClose + span) };
  }

  function submit(
    symbol: string,
    side: OrderSide,
    type: OrderType,
    price: number,
    qty: number,
  ): SubmitResult {
    if (!Number.isFinite(price) || price <= 0 || !Number.isInteger(qty) || qty <= 0) {
      return { ok: false, reason: "价格或数量无效" };
    }
    const band = limitBand(symbol);
    if (price < band.lo || price > band.hi) {
      return { ok: false, reason: `超出限制价区间 ${band.lo.toFixed(2)}–${band.hi.toFixed(2)}` };
    }
    const amount = price * qty;
    if (side === "buy" && amount + feeOf(amount) > cash()) {
      return { ok: false, reason: "资金不足" };
    }
    if (side === "sell" && holding(symbol) < qty) {
      return { ok: false, reason: "持仓不足" };
    }
    const order: Order = {
      id: engine.nextId("o"),
      symbol,
      side,
      type,
      price: round2(price),
      qty,
      filled: 0,
      avgFill: 0,
      status: "pending",
      createdAt: engine.simNow(),
    };
    setOrders((list) => [order, ...list]);
    if (type === "market") {
      fill(order, engine.quote(symbol).last);
    }
    void persist();
    return { ok: true, order };
  }

  function fill(order: Order, price: number): void {
    const live = orders().find((o) => o.id === order.id);
    if (live === undefined || live.status !== "pending") {
      return;
    }
    const remaining = live.qty - live.filled;
    const clip = remaining > SPLIT_QTY ? Math.floor(remaining / 2) : remaining;
    const amount = clip * price;
    const fee = feeOf(amount);
    setCash((c) => round2(c + (live.side === "buy" ? -amount - fee : amount - fee)));
    setPositions((list) => {
      const next = list.map((p) => ({ ...p }));
      let pos = next.find((p) => p.symbol === live.symbol);
      if (pos === undefined) {
        pos = { symbol: live.symbol, qty: 0, avgCost: 0 };
        next.push(pos);
      }
      if (live.side === "buy") {
        const total = pos.avgCost * pos.qty + price * clip;
        pos.qty += clip;
        pos.avgCost = round2(total / Math.max(1, pos.qty));
      } else {
        pos.qty -= clip;
        if (pos.qty <= 0) {
          return next.filter((p) => p.symbol !== live.symbol);
        }
      }
      return next;
    });
    setOrders((list) =>
      list.map((o) => {
        if (o.id !== live.id) {
          return o;
        }
        const filled = o.filled + clip;
        const avgFill = (o.avgFill * o.filled + price * clip) / Math.max(1, filled);
        return {
          ...o,
          filled,
          avgFill: round2(avgFill),
          status: filled >= o.qty ? "filled" : "pending",
        };
      }),
    );
    void persist();
  }

  // Matching runs inside the engine tick so a limit fill and its UI land together.
  engine.onTick((symbol, price) => {
    for (const o of orders()) {
      if (o.symbol !== symbol || o.status !== "pending" || o.type !== "limit") {
        continue;
      }
      const crossed = o.side === "buy" ? price <= o.price : price >= o.price;
      if (crossed) {
        fill(o, Math.min(price, o.side === "buy" ? o.price : price));
      }
    }
  });

  function cancel(id: string): void {
    setOrders((list) =>
      list.map((o) => (o.id === id && o.status === "pending" ? { ...o, status: "cancelled" } : o)),
    );
    void persist();
  }

  function reset(): void {
    setCash(START_CASH);
    setPositions([]);
    setOrders([]);
    void persist();
  }

  async function persist(): Promise<void> {
    try {
      const { AsyncStorage } = await import("@impasto/async-storage");
      const snap: Snapshot = { cash: cash(), positions: positions(), orders: orders() };
      await AsyncStorage.multiSet([[KEY, JSON.stringify(snap)]]);
    } catch {
      /* unlinked host */
    }
  }

  async function hydrate(): Promise<void> {
    try {
      const { AsyncStorage } = await import("@impasto/async-storage");
      const [[, raw]] = await AsyncStorage.multiGet([KEY]);
      if (!raw) {
        return;
      }
      const parsed = JSON.parse(raw) as Partial<Snapshot>;
      if (typeof parsed.cash === "number" && Number.isFinite(parsed.cash)) {
        setCash(parsed.cash);
      }
      if (Array.isArray(parsed.positions)) {
        setPositions(parsed.positions.filter(isPosition));
      }
      if (Array.isArray(parsed.orders)) {
        setOrders(parsed.orders.filter(isOrder));
      }
    } catch {
      /* unlinked host */
    }
  }

  void hydrate();

  function flatten(symbol: string): SubmitResult {
    const qty = holding(symbol);
    if (qty <= 0) {
      return { ok: false, reason: "没有持仓" };
    }
    return submit(symbol, "sell", "market", engine.quote(symbol).last, qty);
  }

  /** Liquidation value of the book at current prices. */
  function marketValue(): number {
    return round2(positions().reduce((sum, p) => sum + p.qty * engine.quote(p.symbol).last, 0));
  }

  return { cash, positions, orders, submit, cancel, flatten, reset, holding, feeOf, marketValue, limitBand };
}

function isPosition(value: unknown): value is Position {
  if (value === null || typeof value !== "object") {
    return false;
  }
  const row = value as Position;
  return typeof row.symbol === "string" && Number.isFinite(row.qty) && Number.isFinite(row.avgCost);
}

function isOrder(value: unknown): value is Order {
  if (value === null || typeof value !== "object") {
    return false;
  }
  const row = value as Order;
  return (
    typeof row.id === "string" &&
    typeof row.symbol === "string" &&
    (row.side === "buy" || row.side === "sell") &&
    (row.type === "market" || row.type === "limit") &&
    Number.isFinite(row.price) &&
    Number.isInteger(row.qty)
  );
}

export type Account = ReturnType<typeof createAccount>;
