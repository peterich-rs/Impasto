/** Page registry. Built-in pages first; third-party pages append. */
import type { JSX } from "solid-js";
import { unseenErrors } from "./store.ts";
import { ConsolePage } from "./pages/console.tsx";
import { ErrorsPage } from "./pages/errors.tsx";
import { NetworkPage } from "./pages/network.tsx";
import { PerformancePage } from "./pages/performance.tsx";
import { InspectorPage } from "./pages/inspector.tsx";
import { LayoutPage } from "./pages/layout.tsx";
import { InfoPage } from "./pages/info.tsx";

export type DevtoolPage = {
  id: string;
  title: string;
  badge?: () => number;
  render: () => JSX.Element;
};

const pages: DevtoolPage[] = [
  { id: "console", title: "日志", render: () => ConsolePage() },
  { id: "errors", title: "错误", badge: unseenErrors, render: () => ErrorsPage() },
  { id: "network", title: "网络", render: () => NetworkPage() },
  { id: "performance", title: "性能", render: () => PerformancePage() },
  { id: "inspector", title: "检查", render: () => InspectorPage() },
  { id: "layout", title: "布局", render: () => LayoutPage() },
  { id: "info", title: "信息", render: () => InfoPage() },
];

/** Third-party pages append after the built-ins. */
export function registerDevtoolPage(page: DevtoolPage): () => void {
  pages.push(page);
  return () => {
    const index = pages.indexOf(page);
    if (index >= 0) {
      pages.splice(index, 1);
    }
  };
}

export function devtoolPages(): readonly DevtoolPage[] {
  return pages;
}

/** Test-only reset so page lists do not leak between cases. */
export function resetPagesForTests(): void {
  pages.length = 7;
}
