import { directionFor, scene, type NodeHandle } from "@impasto/solid-renderer";
import {
  Column,
  ErrorPage as UiErrorPage,
  Toast,
  messageOf as uiMessageOf,
  stackOf as uiStackOf,
} from "@impasto/solid-ui";
import { settingsSig } from "./state/settings.ts";
import { RouterView, createRouter } from "@impasto/solid-router";
import { TabPill } from "./widgets/tab-pill.tsx";
import { MarketsPage, WatchlistPage } from "./pages/stocks.tsx";
import { closeToast, toastOpen, toastText } from "./state/toast.ts";
import { colors } from "./widgets/stocks/theme.ts";
import { loc } from "./copy.ts";

const TAB_PATHS = ["/watchlist", "/account", "/settings"];

/** Deep links open on the matching route; anything else lands on the watchlist. */
export function initialPath(url: string | null): string {
  const path = url?.replace(/^impasto:\/\//, "/") ?? "";
  return path.startsWith("/stock/") || path.startsWith("/search") || TAB_PATHS.includes(path) ? path : "/watchlist";
}

export function App(props: { initial?: string }): NodeHandle {
  const router = createRouter(
    [
      { path: "/watchlist", component: () => WatchlistPage({
        router, title: loc("自选", "Watchlist"), empty: loc("还没有自选，去搜索添加", "Nothing here yet. Search to add symbols"),
        emptyAction: loc("去搜索", "Search"), onEmpty: () => router.push("/search"),
      }) },
      { path: "/markets", component: () => MarketsPage({ router, title: loc("行情", "Markets"), empty: loc("暂无证券", "No symbols") }) },
      {
        path: "/account",
        lazy: () => import("./pages/account.tsx").then(({ AccountPage }) => ({ default: () => AccountPage({ router }) })),
      },
      {
        path: "/settings",
        lazy: () => import("./pages/settings.tsx").then(({ SettingsPage }) => ({ default: () => SettingsPage({ router }) })),
      },
      {
        path: "/search",
        lazy: () => import("./pages/search.tsx").then(({ SearchPage }) => ({ default: () => SearchPage({ router }) })),
      },
      {
        path: "/stock/:symbol",
        lazy: () => import("./pages/stock.tsx").then(({ StockPage }) => ({
          default: (p) => StockPage({
            router,
            symbol: p.params.symbol ?? "",
            get transitioning() { return p.phase !== "live"; },
          }),
        })),
      },
      {
        path: "/order/:symbol",
        lazy: () => import("./pages/order.tsx").then(({ OrderSheetHost }) => ({
          default: (p) => OrderSheetHost({ router, symbol: p.params.symbol ?? "" }),
        })),
      },
    ],
    {
      initial: props.initial ?? "/watchlist",
      tabs: {
        initial: "watchlist",
        branches: { watchlist: "/watchlist", account: "/account", settings: "/settings" },
        maxRetained: 2,
      },
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
  // Plain flex column. The router view fills it; the capsule floats above.
  const content = Column({ style: { flex: 1, width: "100%", minHeight: 0, minWidth: 0, gap: 0 } });
  scene.append(shell, content);
  const view = RouterView({
    router,
    background: colors.bg,
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
  const pill = TabPill({ router });
  scene.append(content, pill);
  const toast = Toast({
    get open() { return toastOpen(); },
    get text() { return toastText(); },
    onClose: closeToast,
  });
  scene.append(shell, toast);
  return shell;
}
