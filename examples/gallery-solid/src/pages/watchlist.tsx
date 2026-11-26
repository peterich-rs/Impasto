import { createMemo } from "solid-js";
import { scene, insert, type NodeHandle } from "@impasto/solid-renderer";
import { Column, Row, VirtualList, createScrollController } from "@impasto/solid-ui";
import { colorBytes } from "../colors.ts";
import { watchlistSig } from "../state/watchlist.ts";
import { settingsSig } from "../state/settings.ts";
import { useQuotes } from "../state/quotes-provider.tsx";
import { showToast } from "../state/toast.ts";
import { PullLabel } from "../widgets/pull-label.tsx";
import { WatchRow, type RowItem } from "../widgets/watch-row.tsx";
import { IndexStrip } from "../widgets/index-strip.tsx";
import { loc } from "../copy.ts";
import type { Router } from "@impasto/solid-router";
const PANEL = colorBytes("#FFFFFF");
const HAIR = colorBytes("#E8E8EC");
const MUTED = colorBytes("#8A8D91");
const INK = colorBytes("#1F2430");
const PRIMARY = colorBytes("#C8102E");
export function WatchlistPage(props: { router: Router }): NodeHandle {
  const { engine, binder } = useQuotes();
  const controller = createScrollController();
  const items = createMemo<RowItem[]>(() => {
    const out: RowItem[] = [];
    let lastGroup = "";
    for (const symbol of watchlistSig()) {
      const market = engine.ins(symbol).market;
      if (market !== lastGroup) {
        lastGroup = market;
        out.push({
          symbol: `group:${market}`,
          firstOfGroup: market,
        });
      }
      out.push({
        symbol,
      });
    }
    return out;
  });
  const page = Column({
    style: {
      gap: 0,
      flex: 1,
      width: "100%",
      height: "100%",
      minWidth: 0,
      minHeight: 0,
    },
  });
  const header = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(header, {
    paddingTop: 12,
    paddingRight: 16,
    paddingBottom: 4,
    paddingLeft: 16,
  });
  scene.append(page, header);
  const headerRow = Row({
    style: {
      gap: 8,
      alignItems: "center",
    },
  });
  scene.append(header, headerRow);
  const title = scene.createElement("text", {
    value: loc("自选", "Watchlist"),
    style: {
      fontSize: 26,
      color: INK,
    },
  });
  scene.patchStyle(title, {
    fontWeight: 700,
  });
  scene.append(headerRow, title);
  const spacer = scene.createElement("view", {
    style: {
      gap: 0,
    },
  });
  scene.patchStyle(spacer, {
    flex: 1,
  });
  scene.append(headerRow, spacer);
  const search = scene.createElement("view", {
    style: {},
  });
  scene.onClick(search, () => props.router.push("/search"));
  scene.append(headerRow, search);
  const searchLabel = scene.createElement("text", {
    value: loc("搜索", "Search"),
    style: {
      fontSize: 14,
      color: PRIMARY,
    },
  });
  const searchAlign = scene.createElement("view", {
    style: {
      alignItems: "center",
      justifyContent: "center",
    },
  });
  scene.patchStyle(searchAlign, {
    flex: 1,
  });
  scene.append(searchAlign, searchLabel);
  scene.append(search, searchAlign);
  const stripHost = IndexStrip({
    symbols: () => watchlistSig(),
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
    paddingBottom: 8,
    paddingLeft: 16,
  });
  scene.append(stripPad, stripHost);
  scene.append(page, stripPad);
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
    itemKey: (item: RowItem) => item.symbol,
    estimatedHeight: 76,
    restoreKey: "watchlist",
    controller,
    sticky: (item) => item.firstOfGroup !== undefined,
    empty: () => emptyState(() => props.router.push("/search")),
    recycle: {
      typeOf: (item: RowItem) =>
        item.symbol.startsWith("group:") ? "header" : "row",
      render: (item, index) => renderRow(item(), index()),
    },
    refresh: {
      threshold: 64,
      onRefresh: () =>
        new Promise<void>((resolve) => {
          setTimeout(() => {
            showToast(loc("已刷新", "Refreshed"));
            resolve();
          }, 600);
        }),
      indicator: ({ progress }) =>
        PullLabel({
          progress,
        }),
    },
  });
  insert(panel, list);
  function renderRow(item: RowItem, _index: number): NodeHandle {
    if (item.firstOfGroup !== undefined) {
      const head = scene.createElement("view", {
        style: {
          padding: 0,
        },
      });
      scene.patchStyle(head, {
        paddingTop: 12,
        paddingRight: 16,
        paddingBottom: 4,
        paddingLeft: 16,
      });
      const labelText = scene.createElement("text", {
        value: groupLabel(item.firstOfGroup),
        style: {
          fontSize: 11,
          color: MUTED,
        },
      });
      scene.patchStyle(labelText, {
        fontWeight: 600,
      });
      scene.append(head, labelText);
      return head;
    }
    return WatchRow({
      item: () => item,
      engine: () => engine,
      binder: () => binder,
      updown: () => settingsSig().updown,
      onClick: (symbol) => props.router.push(`/stock/${symbol}`),
    });
  }
  return page;
}
function emptyState(onSearch: () => void): NodeHandle {
  const col = scene.createElement("view", {
    style: {
      gap: 10,
    },
  });
  const pad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(pad, {
    paddingTop: 32,
    paddingRight: 16,
    paddingBottom: 32,
    paddingLeft: 16,
  });
  scene.append(pad, col);
  const hint = scene.createElement("text", {
    value: loc("还没有自选", "Watchlist is empty"),
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
  scene.append(align, hint);
  scene.append(col, align);
  const cta = scene.createElement("view", {
    style: {},
  });
  scene.onClick(cta, onSearch);
  scene.append(col, cta);
  const pill = scene.createElement("view", {
    style: {
      backgroundColor: PRIMARY,
      borderRadius: 999,
    },
  });
  scene.patchStyle(pill, {
    minWidth: 0,
    minHeight: 36,
  });
  scene.append(cta, pill);
  const label = scene.createElement("text", {
    value: loc("去搜索", "Search"),
    style: {
      fontSize: 13,
      color: colorBytes("#FFFFFF"),
    },
  });
  const center = scene.createElement("view", {
    style: {
      alignItems: "center",
      justifyContent: "center",
    },
  });
  scene.append(center, label);
  scene.append(pill, center);
  return pad;
}
function groupLabel(market: string): string {
  if (market === "hk") {
    return loc("港股", "HK");
  }
  if (market === "us") {
    return loc("美股", "US");
  }
  return loc("A股", "CN");
}
