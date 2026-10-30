import { host, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "../colors.ts";
import type { Router } from "@impasto/solid-router";

const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const BG = colorBytes("#FFFFFF");
const ACTIVE = colorBytes("#C8102E");

type Tab = { key: string; label: string; path: string };

const TABS: Tab[] = [
  { key: "watchlist", label: "自选", path: "/watchlist" },
  { key: "market", label: "市场", path: "/market" },
  { key: "trade", label: "交易", path: "/trade" },
  { key: "profile", label: "我的", path: "/profile" },
];

/**
 * Bottom tab bar. The active tab's ink follows `router.stack()`; switching is
 * `router.reset` (single entry, no stack).
 */
export function TabBar(props: { router: Router }): NodeHandle {
  const bar = host.decorated(BG, 0);
  host.setStroke(bar, 1, colorBytes("#E8E8EC"));

  const row = host.row(0);
  host.append(bar, row);

  const tabs = TABS.map((tab) => {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, () => props.router.reset(tab.path));
    host.append(row, cell);

    const pad = host.padding(0);
    host.setPaddingEdges(pad, 8, 8, 8, 8);
    host.append(cell, pad);

    const text = host.text(tab.label, 12, MUTED);
    const center = host.align("center");
    host.append(center, text);
    host.append(pad, center);
    return { tab, text, center };
  });

  return {
    id: bar,
    sync() {
      const top = props.router.stack()[props.router.stack().length - 1];
      const path = top?.path ?? "";
      for (const t of tabs) {
        const active = path.startsWith(`/${t.tab.key}`);
        host.setColor(t.text, active ? ACTIVE : MUTED);
        host.setFontWeight(t.text, active ? 600 : 400);
      }
    },
  };
}
