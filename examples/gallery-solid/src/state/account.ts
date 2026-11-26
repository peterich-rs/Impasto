import { createSignal } from "solid-js";
import type { QuoteEngine } from "../engine/engine.ts";
import type { Order, OrderSide, OrderType, Position } from "../engine/types.ts";
import { round2 } from "../engine/universe.ts";
const START_CASH = 1_000_000;
const FEE_RATE = 0.0003;
const MIN_FEE = 5;
const SPLIT_QTY = 4000;
const KEY = "impasto.quotes.account.v1";
type Snapshot = {
  cash: number;
  positions: Position[];
  orders: Order[];
};
export type SubmitResult =
  | {
      ok: true;
      order: Order;
    }
  | {
      ok: false;
      reason: string;
    };

/**
 * Mock brokerage. Market orders fill at the latest price;
 * limit orders sit in the book until a tick crosses them.
 */
export function createAccount(engine: QuoteEngine) {
  const [cash, setCash] = createSignal(START_CASH);
  const [positions, setPositions] = createSignal<Position[]>([]);
  const [orders, setOrders] = createSignal<Order[]>([]);
  const feeOf = (amount: number): number =>
    Math.max(MIN_FEE, round2(amount * FEE_RATE));
  function holding(symbol: string): number {
    return positions().find((p) => p.symbol === symbol)?.qty ?? 0;
  }
  function clipSize(remaining: number, lot: number): number {
    return remaining > SPLIT_QTY ? Math.max(lot, Math.floor(remaining / (2 * lot)) * lot) : remaining;
  }
  function reservedCost(price: number, qty: number, lot: number): number {
    let total = price * qty;
    for (let remaining = qty; remaining > 0;) {
      const clip = clipSize(remaining, lot);
      total += feeOf(clip * price);
      remaining -= clip;
    }
    return round2(total);
  }
  function availableCash(): number {
    return Math.max(0, round2(cash() - orders().reduce((total, order) =>
      order.status === "pending" && order.side === "buy"
        ? total + reservedCost(order.price, order.qty - order.filled, engine.ins(order.symbol).lot)
        : total, 0)));
  }
  function availableHolding(symbol: string): number {
    return Math.max(0, holding(symbol) - orders().reduce((total, order) =>
      order.status === "pending" && order.side === "sell" && order.symbol === symbol
        ? total + order.qty - order.filled : total, 0));
  }
  function limitBand(symbol: string): {
    lo: number;
    hi: number;
  } {
    const q = engine.quote(symbol);
    const ins = engine.ins(symbol);
    if (ins.market === "cn") {
      return {
        lo: round2(q.prevClose * 0.9),
        hi: round2(q.prevClose * 1.1),
      };
    }
    const span = q.prevClose * ins.sigma * 0.05;
    return {
      lo: round2(q.prevClose - span),
      hi: round2(q.prevClose + span),
    };
  }
  function submit(
    symbol: string,
    side: OrderSide,
    type: OrderType,
    price: number,
    qty: number,
  ): SubmitResult {
    const lot = engine.ins(symbol).lot;
    price = type === "market" ? engine.quote(symbol).last : round2(price);
    if (
      !Number.isFinite(price) ||
      price <= 0 ||
      !Number.isSafeInteger(qty) ||
      qty <= 0 || qty % lot !== 0
    ) {
      return {
        ok: false,
        reason: "价格或数量无效",
      };
    }
    const band = limitBand(symbol);
    if (price < band.lo || price > band.hi) {
      return {
        ok: false,
        reason: `超出限制价区间 ${band.lo.toFixed(2)}–${band.hi.toFixed(2)}`,
      };
    }
    const amount = price * qty;
    if (side === "buy" && (type === "market" ? amount + feeOf(amount) : reservedCost(price, qty, lot)) > availableCash()) {
      return {
        ok: false,
        reason: "资金不足",
      };
    }
    if (side === "sell" && availableHolding(symbol) < qty) {
      return {
        ok: false,
        reason: "持仓不足",
      };
    }
    let id = engine.nextId("o");
    while (orders().some((order) => order.id === id)) id = engine.nextId("o");
    const order: Order = {
      id,
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
    const submitted = type === "market" ? applyFill(order, price, qty) : order;
    setOrders((list) => [submitted, ...list]);
    void persist();
    return {
      ok: true,
      order: submitted,
    };
  }
  function applyFill(live: Order, price: number, clip: number): Order {
    const amount = clip * price;
    const fee = feeOf(amount);
    setCash((c) =>
      round2(c + (live.side === "buy" ? -amount - fee : amount - fee)),
    );
    setPositions((list) => {
      const next = list.map((p) => ({
        ...p,
      }));
      let pos = next.find((p) => p.symbol === live.symbol);
      if (pos === undefined) {
        pos = {
          symbol: live.symbol,
          qty: 0,
          avgCost: 0,
        };
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
    const filled = live.filled + clip;
    return {
      ...live,
      filled,
      avgFill: round2((live.avgFill * live.filled + price * clip) / filled),
      status: filled >= live.qty ? "filled" : "pending",
    };
  }
  function fill(order: Order, price: number): void {
    const clip = clipSize(order.qty - order.filled, engine.ins(order.symbol).lot);
    const filled = applyFill(order, price, clip);
    setOrders((list) => list.map((item) => item.id === order.id ? filled : item));
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
        fill(o, price);
      }
    }
  });
  function cancel(id: string): void {
    setOrders((list) =>
      list.map((o) =>
        o.id === id && o.status === "pending"
          ? {
              ...o,
              status: "cancelled",
            }
          : o,
      ),
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
      const snap: Snapshot = {
        cash: cash(),
        positions: positions(),
        orders: orders(),
      };
      await AsyncStorage.multiSet([[KEY, JSON.stringify(snap)]]);
    } catch {
      /* unlinked scene */
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
      /* unlinked scene */
    }
  }
  void hydrate();
  function flatten(symbol: string): SubmitResult {
    const qty = availableHolding(symbol);
    if (qty <= 0) {
      return {
        ok: false,
        reason: "没有持仓",
      };
    }
    return submit(symbol, "sell", "market", engine.quote(symbol).last, qty);
  }

  /** Liquidation value of the book at current prices. */
  function marketValue(): number {
    return round2(
      positions().reduce(
        (sum, p) => sum + p.qty * engine.quote(p.symbol).last,
        0,
      ),
    );
  }
  return {
    cash,
    positions,
    orders,
    submit,
    cancel,
    flatten,
    reset,
    holding,
    availableCash,
    availableHolding,
    feeOf,
    marketValue,
    limitBand,
  };
}
function isPosition(value: unknown): value is Position {
  if (value === null || typeof value !== "object") {
    return false;
  }
  const row = value as Position;
  return (
    typeof row.symbol === "string" &&
    Number.isFinite(row.qty) &&
    Number.isFinite(row.avgCost)
  );
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
