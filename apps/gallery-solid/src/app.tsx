import { createSignal, createTrackedEffect, onCleanup } from "solid-js";
import { host, type NodeHandle } from "@impasto/solid-renderer";
import {
  Button,
  Dialog,
  ErrorPage as UiErrorPage,
  OverlayRoot,
  messageOf as uiMessageOf,
  stackOf as uiStackOf,
} from "@impasto/solid-ui";
import { RouterView, createRouter } from "@impasto/solid-router";
import { QuotesProvider, useBinder } from "./state/quotes-provider.tsx";
import { TabBar } from "./widgets/tab-bar.tsx";
import { WatchlistPage } from "./pages/watchlist.tsx";
import { MarketPage } from "./pages/market.tsx";
import { StockPage } from "./pages/stock.tsx";
import { TradePage } from "./pages/trade.tsx";
import { ProfilePage } from "./pages/profile.tsx";
import { OrderSheetHost } from "./pages/order.tsx";
import { colorBytes } from "./colors.ts";
import type { Router } from "@impasto/solid-router";

const BG = colorBytes("#F5F6F8");

export function App(): NodeHandle {
  const router = createRouter(
    [
      {
        path: "/watchlist",
        component: () => WatchlistPage({ router }),
        keepAlive: true,
      },
      { path: "/market", component: () => MarketPage({ router }), keepAlive: true },
      { path: "/trade", component: () => TradePage({ router }), keepAlive: true },
      { path: "/profile", component: () => ProfilePage({ router }) },
      { path: "/search", component: (p) => SearchPage({ router, params: p.params }) },
      {
        path: "/stock/:symbol",
        component: (p) => StockPage({ router, symbol: p.params.symbol ?? "" }),
      },
      {
        path: "/order/:symbol",
        component: (p) => {
          const [symbol, query] = splitOrderPath(p.params.symbol ?? "");
          return OrderSheetHost({ router, symbol, query });
        },
      },
    ],
    { initial: "/watchlist" },
  );

  const shell = host.column(0);
  host.setGrow(shell, 1);

  const content = host.column(0);
  host.setGrow(content, 1);
  host.append(shell, content);

  const bar = TabBar({ router });
  createTrackedEffect(() => {
    router.stack();
    bar.sync();
  });

  const view = RouterView({
    router,
    background: "#F5F6F8",
    errorFallback: (err, reset) =>
      UiErrorPage({
        message: uiMessageOf(typeof err === "function" ? err() : err),
        stack: uiStackOf(typeof err === "function" ? err() : err),
        retry: () => reset(),
        onBack: () => router.pop(),
      }),
  });
  host.append(content, view.id);
  host.append(content, bar.id);

  return { id: shell };
}

function splitOrderPath(raw: string): [string, Record<string, string>] {
  const [symbol, search = ""] = raw.split("?");
  const query: Record<string, string> = {};
  for (const pair of search.split("&")) {
    if (!pair) {
      continue;
    }
    const [k, v = ""] = pair.split("=");
    query[decodeURIComponent(k)] = decodeURIComponent(v);
  }
  return [symbol ?? "", query];
}

function SearchPage(props: { router: Router; params: Record<string, string> }): NodeHandle {
  const { engine, binder } = useBinder();
  const [term, setTerm] = createSignal(props.params.q ?? "");

  const page = host.column(8);
  host.setGrow(page, 1);
  const title = host.text("搜索", 26, colorBytes("#1F2430"));
  host.setFontWeight(title, 700);
  host.append(page, title);

  const hint = host.text(term() ? `关键词：${term()}` : "输入代码或名称（从自选页快捷进入）", 13, colorBytes("#8A8D91"));
  host.append(page, hint);

  const results = host.column(0);
  host.setGrow(results, 1);
  host.append(page, results);

  createTrackedEffect(() => {
    const stops: Array<() => void> = [];
    onCleanup(() => {
      for (const stop of stops) {
        stop();
      }
    });
    const t = term().trim().toLowerCase();
    clearChildren(results);
    const hits = engine
      .instruments()
      .filter(
        (ins) =>
          t === "" ||
          ins.symbol.toLowerCase().includes(t) ||
          ins.name.toLowerCase().includes(t),
      )
      .slice(0, 12);
    if (t !== "" && hits.length === 0) {
      const empty = host.text("没有匹配的标的", 13, colorBytes("#8A8D91"));
      host.append(results, empty);
      return;
    }
    for (const ins of hits) {
      const row = host.pointerRegion();
      host.onClick(row, () => props.router.push(`/stock/${ins.symbol}`));
      const pad = host.padding(0);
      host.setPaddingEdges(pad, 8, 16, 8, 16);
      host.append(row, pad);
      const line = host.row(8);
      host.setMinSize(line, 0, 36);
      host.append(pad, line);
      const name = host.text(ins.name, 14, colorBytes("#1F2430"));
      host.append(line, name);
      const sym = host.text(ins.symbol, 11, colorBytes("#8A8D91"));
      host.append(line, sym);
      const spacer = host.sized(0, 0);
      host.setGrow(spacer, 1);
      host.append(line, spacer);
      const last = host.text("—", 13, colorBytes("#1F2430"));
      host.append(line, last);
      stops.push(binder.bind(ins.symbol, { last, flashNode: last }));
    }
  });

  void setTerm;
  void Button;
  void Dialog;
  void OverlayRoot;
  return { id: page };
}
