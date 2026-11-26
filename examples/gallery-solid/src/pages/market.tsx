import { createRenderEffect, createMemo, createSignal } from "solid-js";
import { scene, insert, type NodeHandle } from "@impasto/solid-renderer";
import { View, Text } from "@impasto/solid-renderer/host";
import { VirtualList, createScrollController } from "@impasto/solid-ui";
import { colorBytes } from "../colors.ts";
import { useQuotes } from "../state/quotes-provider.tsx";
import type { Router } from "@impasto/solid-router";
import { QuoteText } from "../widgets/quote-text.tsx";
import { Monogram } from "../widgets/monogram.tsx";
import { IndexStrip } from "../widgets/index-strip.tsx";
import { loc } from "../copy.ts";
const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const PANEL = colorBytes("#FFFFFF");
const HAIR = colorBytes("#E8E8EC");
type Board = "gainers" | "losers" | "turnover";
type BoardItem = {
  symbol: string;
};
export function MarketPage(props: { router: Router }): NodeHandle {
  const { engine, binder } = useQuotes();
  const [tab, setTab] = createSignal<Board>("gainers");
  const [market, setMarket] = createSignal<"hk" | "us" | "cn">("hk");
  const controller = createScrollController();
  const page = scene.createElement("view", {
    style: {
      gap: 10,
    },
  });
  scene.patchStyle(page, {
    flex: 1,
  });
  const title = scene.createElement("text", {
    value: loc("市场", "Market"),
    style: {
      fontSize: 26,
      color: INK,
    },
  });
  scene.patchStyle(title, {
    fontWeight: 700,
  });
  const titlePad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(titlePad, {
    paddingTop: 12,
    paddingRight: 16,
    paddingBottom: 0,
    paddingLeft: 16,
  });
  scene.append(titlePad, title);
  scene.append(page, titlePad);
  const stripHost = IndexStrip({
    symbols: () =>
      engine
        .instruments()
        .filter((ins) => ins.index)
        .map((ins) => ins.symbol),
    engine: () => engine,
    binder: () => binder,
    onClick: (symbol) => props.router.push(`/stock/${symbol}`),
  });
  const stripPad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(stripPad, {
    paddingTop: 8,
    paddingRight: 16,
    paddingBottom: 0,
    paddingLeft: 16,
  });
  scene.append(stripPad, stripHost);
  scene.append(page, stripPad);
  const pills = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 6,
    },
  });
  const pillPad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(pillPad, {
    paddingTop: 12,
    paddingRight: 16,
    paddingBottom: 0,
    paddingLeft: 16,
  });
  scene.append(pillPad, pills);
  scene.append(page, pillPad);
  for (const m of ["hk", "us", "cn"] as const) {
    buildPill(m);
  }
  function buildPill(m: "hk" | "us" | "cn"): void {
    const cell = scene.createElement("view", {
      style: {},
    });
    scene.patchStyle(cell, {
      flex: 1,
    });
    scene.onClick(cell, () => setMarket(m));
    scene.append(pills, cell);
    const pill = scene.createElement("view", {
      style: {
        backgroundColor: PANEL,
        borderRadius: 999,
      },
    });
    scene.patchStyle(pill, {
      outlineWidth: 1,
      outlineColor: HAIR,
    });
    scene.patchStyle(pill, {
      minWidth: 0,
      minHeight: 32,
    });
    scene.patchStyle(pill, {
      flex: 1,
    });
    scene.append(cell, pill);
    const label = scene.createElement("text", {
      value: labelOf(m),
      style: {
        fontSize: 12,
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
    createRenderEffect(
      () => market() === m,
      (active) => {
        scene.patchStyle(pill, {
          backgroundColor: active ? INK : PANEL,
        });
        scene.patchStyle(label, {
          color: active ? colorBytes("#FFFFFF") : MUTED,
        });
      },
    );
  }
  const tabs = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 0,
    },
  });
  const tabPad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(tabPad, {
    paddingTop: 12,
    paddingRight: 16,
    paddingBottom: 0,
    paddingLeft: 16,
  });
  scene.append(tabPad, tabs);
  scene.append(page, tabPad);
  for (const b of ["gainers", "losers", "turnover"] as const) {
    buildTab(b);
  }
  function buildTab(b: Board): void {
    const cell = scene.createElement("view", {
      style: {},
    });
    scene.patchStyle(cell, {
      flex: 1,
    });
    scene.onClick(cell, () => setTab(b));
    scene.append(tabs, cell);
    const inner = scene.createElement("view", {
      style: {
        padding: 0,
      },
    });
    scene.patchStyle(inner, {
      paddingTop: 8,
      paddingRight: 0,
      paddingBottom: 8,
      paddingLeft: 0,
    });
    scene.patchStyle(inner, {
      flex: 1,
    });
    scene.append(cell, inner);
    const label = scene.createElement("text", {
      value: boardLabel(b),
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
    scene.append(inner, align);
    createRenderEffect(
      () => tab() === b,
      (active) => {
        scene.patchStyle(label, {
          color: active ? INK : MUTED,
        });
        scene.patchStyle(label, {
          fontWeight: active ? 700 : 400,
        });
      },
    );
  }
  const items = createMemo<BoardItem[]>(() => {
    const all = engine
      .instruments()
      .filter((ins) => ins.market === market() && !ins.index);
    const pct = (s: string) => {
      const q = engine.quote(s);
      return (q.last - q.prevClose) / q.prevClose;
    };
    if (tab() === "turnover") {
      return all
        .map((ins) => ({
          symbol: ins.symbol,
        }))
        .sort(
          (a, b) =>
            engine.quote(b.symbol).volume - engine.quote(a.symbol).volume,
        )
        .slice(0, 20);
    }
    return all
      .map((ins) => ({
        symbol: ins.symbol,
      }))
      .sort((a, b) =>
        tab() === "gainers"
          ? pct(b.symbol) - pct(a.symbol)
          : pct(a.symbol) - pct(b.symbol),
      )
      .slice(0, 20);
  });
  const panel = scene.createElement("view", {
    style: {
      backgroundColor: PANEL,
      borderRadius: 12,
    },
  });
  scene.patchStyle(panel, {
    outlineWidth: 1,
    outlineColor: HAIR,
  });
  scene.patchStyle(panel, {
    flex: 1,
  });
  scene.append(page, panel);
  const list = VirtualList({
    get items() {
      return items();
    },
    itemKey: (item: BoardItem) => item.symbol,
    estimatedHeight: 64,
    restoreKey: "market",
    controller,
    recycle: {
      typeOf: () => "row",
      render: (item, index) => buildBoardRow(item(), index()),
    },
  });
  insert(panel, list);
  function buildBoardRow(item: BoardItem, index: number): number {
    const ins = engine.ins(item.symbol);
    return <View onClick={() => props.router.push(`/stock/${item.symbol}`)}>
      <View style={{ paddingTop: 10, paddingBottom: 10, paddingLeft: 16, paddingRight: 16, flex: 1 }}>
        <View style={{ flexDirection: "row", gap: 8, minWidth: 0, minHeight: 44, alignItems: "center" }}>
          <Text value={String(index + 1)} style={{ fontSize: 12, color: "#8A8D91", minWidth: 18, minHeight: 0 }} />
          {Monogram({ ins: () => ins })}
          <View style={{ gap: 1, flex: 1 }}>
            <Text value={ins.name} style={{ fontSize: 14, color: "#1F2430", maxLines: 1 }} />
            <Text value={ins.symbol} style={{ fontSize: 10, color: "#8A8D91" }} />
          </View>
          <View style={{ gap: 1, minWidth: 88, minHeight: 0 }}>
            {QuoteText({ symbol: () => item.symbol, kind: "last", fontSize: 14, align: "right", binder: () => binder })}
            {QuoteText({ symbol: () => item.symbol, kind: "pct", fontSize: 12, align: "right", binder: () => binder })}
          </View>
        </View>
      </View>
    </View>;
  }
  return page;
}
function labelOf(m: "hk" | "us" | "cn"): string {
  return m === "hk"
    ? loc("港股", "HK")
    : m === "us"
      ? loc("美股", "US")
      : loc("A股", "CN");
}
function boardLabel(b: Board): string {
  return b === "gainers"
    ? loc("涨幅榜", "Gainers")
    : b === "losers"
      ? loc("跌幅榜", "Losers")
      : loc("成交额", "Turnover");
}
