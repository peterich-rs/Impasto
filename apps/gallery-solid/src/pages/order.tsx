import { createSignal, createTrackedEffect } from "solid-js";
import { scene, type NodeHandle } from "@impasto/solid-renderer";
import { Sheet } from "@impasto/solid-ui";
import { colorBytes } from "../colors.ts";
import { useQuotes } from "../state/quotes-provider.tsx";
import { accountRef } from "../state/account-ref.ts";
import { showToast } from "../state/toast.ts";
import { fmtPrice, fmtQty } from "../format.ts";
import { loc } from "../copy.ts";
import { takeOrderTicket } from "../state/order-ticket.ts";
import type { Router } from "@impasto/solid-router";
const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const PANEL = colorBytes("#FFFFFF");
const HAIR = colorBytes("#E8E8EC");
const UP = colorBytes("#EB3E3E");
const DOWN = colorBytes("#22B05F");

/** Order ticket sheet: side toggle, price, qty stepper, percent quick bars. */
export function OrderSheet(props: {
  symbol: string;
  query: Record<string, string>;
  onClose: () => void;
}): NodeHandle {
  const { engine } = useQuotes();
  const account = accountRef();
  const symbol = () => props.symbol;
  const q = () => engine.quote(symbol());
  const [side, setSide] = createSignal<"buy" | "sell">(
    props.query.side === "sell" ? "sell" : "buy",
  );
  const [type, setType] = createSignal<"market" | "limit">("limit");
  const [price, setPrice] = createSignal(
    props.query.price ? Number(props.query.price) : q().last,
  );
  const [qty, setQty] = createSignal(100);
  const [error, setError] = createSignal("");
  const ins = engine.ins(symbol());
  const pageInner = scene.createElement("view", {
    style: {
      gap: 8,
    },
  });
  const page = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(page, {
    paddingTop: 16,
    paddingRight: 16,
    paddingBottom: 16,
    paddingLeft: 16,
  });
  scene.append(page, pageInner);
  const title = scene.createElement("text", {
    value: `${ins.name} ${loc("下单", "Order")}`,
    style: {
      fontSize: 17,
      color: INK,
    },
  });
  scene.patchStyle(title, {
    fontWeight: 700,
  });
  scene.append(pageInner, title);
  const sideRow = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 0,
    },
  });
  scene.append(pageInner, sideRow);
  for (const s of ["buy", "sell"] as const) {
    const cell = scene.createElement("view", {
      style: {},
    });
    scene.patchStyle(cell, {
      flex: 1,
    });
    scene.onClick(cell, () => setSide(s));
    scene.append(sideRow, cell);
    const pill = scene.createElement("view", {
      style: {
        backgroundColor: PANEL,
        borderRadius: 8,
      },
    });
    scene.patchStyle(pill, {
      outlineWidth: 1,
      outlineColor: HAIR,
    });
    scene.patchStyle(pill, {
      minWidth: 0,
      minHeight: 36,
    });
    scene.patchStyle(pill, {
      flex: 1,
    });
    scene.append(cell, pill);
    const label = scene.createElement("text", {
      value: s === "buy" ? loc("买入", "Buy") : loc("卖出", "Sell"),
      style: {
        fontSize: 14,
        color: MUTED,
      },
    });
    const align = scene.createElement("view", {
      style: {
        alignItems: "center",
        justifyContent: "center",
      },
    });
    scene.patchStyle(align, {
      flex: 1,
    });
    scene.append(align, label);
    scene.append(pill, align);
    createTrackedEffect(() => {
      const active = side() === s;
      scene.patchStyle(pill, {
        backgroundColor: active ? (s === "buy" ? UP : DOWN) : PANEL,
      });
      scene.patchStyle(label, {
        color: active ? colorBytes("#FFFFFF") : MUTED,
      });
    });
  }
  const typeRow = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 0,
    },
  });
  scene.append(pageInner, typeRow);
  for (const t of ["market", "limit"] as const) {
    const cell = scene.createElement("view", {
      style: {},
    });
    scene.patchStyle(cell, {
      flex: 1,
    });
    scene.onClick(cell, () => setType(t));
    scene.append(typeRow, cell);
    const label = scene.createElement("text", {
      value: t === "market" ? loc("市价", "Market") : loc("限价", "Limit"),
      style: {
        fontSize: 13,
        color: MUTED,
      },
    });
    const align = scene.createElement("view", {
      style: {
        alignItems: "center",
        justifyContent: "center",
      },
    });
    scene.patchStyle(align, {
      flex: 1,
    });
    scene.append(align, label);
    scene.append(cell, align);
    createTrackedEffect(() => {
      const active = type() === t;
      scene.patchStyle(label, {
        color: active ? INK : MUTED,
      });
      scene.patchStyle(label, {
        fontWeight: active ? 600 : 400,
      });
    });
  }
  const priceBox = scene.createElement("view", {
    style: {
      gap: 4,
    },
  });
  scene.append(pageInner, priceBox);
  const priceLabel = scene.createElement("text", {
    value: loc("价格", "Price"),
    style: {
      fontSize: 12,
      color: MUTED,
    },
  });
  scene.append(priceBox, priceLabel);
  const priceRow = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 6,
    },
  });
  scene.append(priceBox, priceRow);
  const priceDown = stepperBtn("−", () =>
    setPrice((p) => Math.max(0.01, round2(p - tick()))),
  );
  scene.append(priceRow, priceDown);
  const priceValue = scene.createElement("text", {
    value: fmtPrice(price()),
    style: {
      fontSize: 16,
      color: INK,
    },
  });
  scene.patchStyle(priceValue, {
    fontWeight: 600,
  });
  scene.append(priceRow, priceValue);
  const priceGrow = scene.createElement("view", {
    style: {
      width: 0,
      height: 0,
    },
  });
  scene.patchStyle(priceGrow, {
    flex: 1,
  });
  scene.append(priceRow, priceGrow);
  const priceUp = stepperBtn("+", () => setPrice((p) => round2(p + tick())));
  scene.append(priceRow, priceUp);
  createTrackedEffect(() => {
    scene.setText(
      priceValue,
      type() === "market" ? loc("市价成交", "Market fill") : fmtPrice(price()),
    );
  });
  createTrackedEffect(() => {
    const hide = type() === "market";
    scene.patchStyle(priceDown, {
      opacity: (hide ? 80 : 255) / 255,
    });
    scene.patchStyle(priceUp, {
      opacity: (hide ? 80 : 255) / 255,
    });
  });
  function tick(): number {
    return ins.market === "cn" ? 0.01 : q().last > 100 ? 0.05 : 0.01;
  }
  const qtyBox = scene.createElement("view", {
    style: {
      gap: 4,
    },
  });
  scene.append(pageInner, qtyBox);
  const qtyLabel = scene.createElement("text", {
    value: loc(
      `数量（${ins.lot === 100 ? "按手，1手=100股" : "股数"}）`,
      `Qty (${ins.lot === 100 ? "lots of 100" : "shares"})`,
    ),
    style: {
      fontSize: 12,
      color: MUTED,
    },
  });
  scene.append(qtyBox, qtyLabel);
  const qtyRow = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 6,
    },
  });
  scene.append(qtyBox, qtyRow);
  const qtyDown = stepperBtn("−", () =>
    setQty((n) => Math.max(ins.lot, n - ins.lot)),
  );
  scene.append(qtyRow, qtyDown);
  const qtyValue = scene.createElement("text", {
    value: fmtQty(qty()),
    style: {
      fontSize: 16,
      color: INK,
    },
  });
  scene.patchStyle(qtyValue, {
    fontWeight: 600,
  });
  scene.append(qtyRow, qtyValue);
  const qtyGrow = scene.createElement("view", {
    style: {
      width: 0,
      height: 0,
    },
  });
  scene.patchStyle(qtyGrow, {
    flex: 1,
  });
  scene.append(qtyRow, qtyGrow);
  const qtyUp = stepperBtn("+", () => setQty((n) => n + ins.lot));
  scene.append(qtyRow, qtyUp);
  createTrackedEffect(() => {
    scene.setText(qtyValue, fmtQty(qty()));
  });
  const pctRow = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 6,
    },
  });
  scene.append(pageInner, pctRow);
  for (const pct of [25, 50, 75, 100]) {
    const cell = scene.createElement("view", {
      style: {},
    });
    scene.patchStyle(cell, {
      flex: 1,
    });
    scene.onClick(cell, () => {
      if (side() === "buy") {
        const spendable = account.cash() / (1 + 0.0003);
        const lots =
          Math.floor((spendable * pct) / 100 / (price() * ins.lot)) * ins.lot;
        setQty(Math.max(ins.lot, lots));
      } else {
        const held = account.holding(symbol());
        const lots = Math.floor((held * pct) / 100 / ins.lot) * ins.lot;
        setQty(Math.max(ins.lot, lots));
      }
    });
    scene.append(pctRow, cell);
    const pill = scene.createElement("view", {
      style: {
        backgroundColor: colorBytes("#F2F3F5"),
        borderRadius: 6,
      },
    });
    scene.patchStyle(pill, {
      outlineWidth: 1,
      outlineColor: HAIR,
    });
    scene.patchStyle(pill, {
      minWidth: 0,
      minHeight: 28,
    });
    scene.patchStyle(pill, {
      flex: 1,
    });
    scene.append(cell, pill);
    const label = scene.createElement("text", {
      value: `${pct}%`,
      style: {
        fontSize: 11,
        color: MUTED,
      },
    });
    const align = scene.createElement("view", {
      style: {
        alignItems: "center",
        justifyContent: "center",
      },
    });
    scene.patchStyle(align, {
      flex: 1,
    });
    scene.append(align, label);
    scene.append(pill, align);
  }
  const amount = scene.createElement("text", {
    value: "",
    style: {
      fontSize: 12,
      color: MUTED,
    },
  });
  scene.append(pageInner, amount);
  createTrackedEffect(() => {
    const eff = type() === "market" ? q().last : price();
    const total = eff * qty();
    scene.setText(
      amount,
      loc(
        `约 ${fmtPrice(total)} · 手续费 ${fmtPrice(account.feeOf(total))} · 可用 ${fmtPrice(account.cash())}`,
        `≈ ${fmtPrice(total)} · fee ${fmtPrice(account.feeOf(total))} · cash ${fmtPrice(account.cash())}`,
      ),
    );
  });
  const errText = scene.createElement("text", {
    value: "",
    style: {
      fontSize: 12,
      color: UP,
    },
  });
  scene.append(pageInner, errText);
  createTrackedEffect(() => {
    scene.setText(errText, error());
  });
  const submit = scene.createElement("view", {
    style: {},
  });
  scene.patchStyle(submit, {
    flex: 1,
  });
  scene.onClick(submit, () => {
    const eff = type() === "market" ? q().last : price();
    const result = account.submit(symbol(), side(), type(), eff, qty());
    if (!result.ok) {
      setError(result.reason);
      showToast(result.reason);
      return;
    }
    setError("");
    showToast(loc("已提交", "Submitted"));
    props.onClose();
  });
  const submitPill = scene.createElement("view", {
    style: {
      backgroundColor: UP,
      borderRadius: 8,
    },
  });
  scene.patchStyle(submitPill, {
    minWidth: 0,
    minHeight: 44,
  });
  scene.patchStyle(submitPill, {
    flex: 1,
  });
  scene.append(submit, submitPill);
  const submitLabel = scene.createElement("text", {
    value: loc("提交订单", "Submit"),
    style: {
      fontSize: 15,
      color: colorBytes("#FFFFFF"),
    },
  });
  scene.patchStyle(submitLabel, {
    fontWeight: 600,
  });
  const submitAlign = scene.createElement("view", {
    style: {
      alignItems: "center",
      justifyContent: "center",
    },
  });
  scene.patchStyle(submitAlign, {
    flex: 1,
  });
  scene.append(submitAlign, submitLabel);
  scene.append(submitPill, submitAlign);
  createTrackedEffect(() => {
    scene.patchStyle(submitPill, {
      backgroundColor: side() === "buy" ? UP : DOWN,
    });
  });
  scene.append(pageInner, submit);

  function stepperBtn(glyph: string, onClick: () => void): number {
    const cell = scene.createElement("view", {
      style: {
        backgroundColor: colorBytes("#F2F3F5"),
        borderRadius: 8,
        minWidth: 36,
        minHeight: 36,
        alignItems: "center",
        justifyContent: "center",
      },
    });
    scene.onClick(cell, onClick);
    scene.append(
      cell,
      scene.createElement("text", {
        value: glyph,
        style: { fontSize: 18, color: INK },
      }),
    );
    return cell;
  }
  return page;
}
function round2(value: number): number {
  return Math.round(value * 100) / 100;
}
export function OrderSheetHost(props: {
  router: Router;
  symbol: string;
}): NodeHandle {
  const query = takeOrderTicket();
  const [open, setOpen] = createSignal(true);
  const close = () => {
    setOpen(false);
    props.router.pop();
  };
  return (
    <Sheet open={open()} onClose={close}>
      {OrderSheet({
        symbol: props.symbol,
        query,
        onClose: close,
      })}
    </Sheet>
  );
}
