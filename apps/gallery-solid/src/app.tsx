import { createTrackedEffect } from "solid-js";
import { directionFor, scene, insert, type NodeHandle } from "@impasto/solid-renderer";
import {
  Column,
  ErrorPage as UiErrorPage,
  Toast,
  messageOf as uiMessageOf,
  stackOf as uiStackOf,
} from "@impasto/solid-ui";
import { settingsSig } from "./state/settings.ts";
import { RouterView, createRouter } from "@impasto/solid-router";
import { TabBar } from "./widgets/tab-bar.tsx";
import { WatchlistPage } from "./pages/watchlist.tsx";
import { MarketPage } from "./pages/market.tsx";
import { StockPage } from "./pages/stock.tsx";
import { StocksPage } from "./pages/stocks.tsx";
import { TradePage } from "./pages/trade.tsx";
import { ProfilePage } from "./pages/profile.tsx";
import { SearchPage } from "./pages/search.tsx";
import { OrderSheetHost } from "./pages/order.tsx";
import { CanvasPage } from "./pages/canvas.tsx";
import { closeToast, toastOpen, toastText } from "./state/toast.ts";
export function App(): NodeHandle {
  const router = createRouter(
    [
      {
        path: "/stocks",
        component: () => StocksPage({ router }),
        keepAlive: true,
      },
      {
        path: "/watchlist",
        component: () =>
          WatchlistPage({
            router,
          }),
        keepAlive: true,
      },
      {
        path: "/market",
        component: () =>
          MarketPage({
            router,
          }),
        keepAlive: true,
      },
      {
        path: "/trade",
        component: () =>
          TradePage({
            router,
          }),
        keepAlive: true,
      },
      {
        path: "/profile",
        component: () =>
          ProfilePage({
            router,
          }),
      },
      {
        path: "/search",
        component: () =>
          SearchPage({
            router,
          }),
      },
      {
        path: "/stock/:symbol",
        component: (p) =>
          StockPage({
            router,
            symbol: p.params.symbol ?? "",
            get transitioning() {
              return p.phase !== "live";
            },
          }),
      },
      {
        path: "/order/:symbol",
        component: (p) =>
          OrderSheetHost({
            router,
            symbol: p.params.symbol ?? "",
          }),
      },
      {
        path: "/canvas",
        component: () => CanvasPage(),
      },
    ],
    {
      initial: "/stocks",
    },
  );
  const shell = Column({
    style: {
      gap: 0,
      flex: 1,
      width: "100%",
      height: "100%",
      minWidth: 0,
      minHeight: 0,
      layoutDirection: directionFor(settingsSig().locale) === "rtl" ? "rtl" : "ltr",
    },
  });
  const content = scene.createElement("view", {
    style: {
      gap: 0,
    },
  });
  scene.patchStyle(content, {
    flex: 1,
  });
  scene.append(shell, content);
  const view = RouterView({
    router,
    background: "#F5F6F8",
    opaque: true,
    errorFallback: (err, reset) =>
      UiErrorPage({
        message: uiMessageOf(typeof err === "function" ? err() : err),
        stack: uiStackOf(typeof err === "function" ? err() : err),
        retry: () => reset(),
        onBack: () => router.pop(),
      }),
  });
  scene.append(content, view);
  const bar = TabBar({
    router,
  });
  const barWrap = scene.createElement("view", {
    style: {
      gap: 0,
    },
  });
  scene.append(barWrap, bar.node);
  scene.append(content, barWrap);
  createTrackedEffect(() => {
    // Keep the bar clamped until the exiting page unmounts. Unhiding it on the
    // first pop frame relayouts both stacked pages and hitchs the transition.
    const path = router.stack().at(-1)?.path ?? "";
    const hidden = router.depth() > 1 || path === "/stocks" || path.startsWith("/stock/") || path.startsWith("/order/");
    scene.patchStyle(barWrap, { display: hidden ? "none" : "flex" });
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
  insert(shell, toast);
  return shell;
}
