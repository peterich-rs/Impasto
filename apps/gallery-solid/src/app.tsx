import { createTrackedEffect } from "solid-js";
import { host, insert, type NodeHandle } from "@impasto/solid-renderer";
import { ErrorPage as UiErrorPage, Toast, messageOf as uiMessageOf, stackOf as uiStackOf } from "@impasto/solid-ui";
import { RouterView, createRouter } from "@impasto/solid-router";
import { TabBar } from "./widgets/tab-bar.tsx";
import { WatchlistPage } from "./pages/watchlist.tsx";
import { MarketPage } from "./pages/market.tsx";
import { StockPage } from "./pages/stock.tsx";
import { TradePage } from "./pages/trade.tsx";
import { ProfilePage } from "./pages/profile.tsx";
import { SearchPage } from "./pages/search.tsx";
import { OrderSheetHost } from "./pages/order.tsx";
import { closeToast, toastOpen, toastText } from "./state/toast.ts";
import type { Router } from "@impasto/solid-router";

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
      { path: "/search", component: () => SearchPage({ router }) },
      {
        path: "/stock/:symbol",
        component: (p) => StockPage({ router, symbol: p.params.symbol ?? "" }),
      },
      {
        path: "/order/:symbol",
        component: (p) => OrderSheetHost({ router, symbol: p.params.symbol ?? "" }),
      },
    ],
    { initial: "/watchlist" },
  );

  const shell = host.column(0);
  host.setGrow(shell, 1);

  const content = host.column(0);
  host.setGrow(content, 1);
  host.append(shell, content);

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

  const bar = TabBar({ router });
  // Non-decorated wrap: hide by clamping axis + min + max to 0. Never setAlpha on Decorated.
  const barWrap = host.column(0);
  host.append(barWrap, bar.id);
  host.append(content, barWrap);

  createTrackedEffect(() => {
    const hidden = liveDepth(router) > 1;
    host.setAxisSize(barWrap, "auto", hidden ? 0 : "auto");
    host.setMinSize(barWrap, 0, 0);
    host.setMaxSize(barWrap, 4096, hidden ? 0 : 4096);
    bar.sync();
  });

  const toast = Toast({
    get open() {
      return toastOpen();
    },
    get text() {
      return toastText();
    },
    onClose: closeToast,
  });
  insert({ id: shell } as NodeHandle, toast);

  return { id: shell };
}

/** Stack entries still exiting or covering do not count as a pushed page. */
function liveDepth(router: Router): number {
  return router.stack().filter((entry) => entry.phase !== "exit").length;
}

