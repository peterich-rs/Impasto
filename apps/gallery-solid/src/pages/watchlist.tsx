import { createMemo } from "solid-js";
import { host, insert, type NodeHandle } from "@impasto/solid-renderer";
import { VirtualList, createScrollController } from "@impasto/solid-ui";
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
        out.push({ symbol: `group:${market}`, firstOfGroup: market });
      }
      out.push({ symbol });
    }
    return out;
  });

  const page = host.column(0);
  host.setGrow(page, 1);

  const header = host.padding(0);
  host.setPaddingEdges(header, 12, 16, 4, 16);
  host.append(page, header);
  const headerRow = host.row(8);
  host.setCross(headerRow, "center");
  host.append(header, headerRow);
  const title = host.text(loc("自选", "Watchlist"), 26, INK);
  host.setFontWeight(title, 700);
  host.append(headerRow, title);
  const spacer = host.column(0);
  host.setGrow(spacer, 1);
  host.append(headerRow, spacer);
  const search = host.pointerRegion();
  host.onClick(search, () => props.router.push("/search"));
  host.append(headerRow, search);
  const searchLabel = host.text(loc("搜索", "Search"), 14, PRIMARY);
  const searchAlign = host.align("center");
  host.setGrow(searchAlign, 1);
  host.append(searchAlign, searchLabel);
  host.append(search, searchAlign);

  const stripHost = IndexStrip({
    symbols: () => watchlistSig(),
    engine: () => engine,
    binder: () => binder,
    onClick: (symbol) => props.router.push(`/stock/${symbol}`),
  });
  const stripPad = host.padding(0);
  host.setPaddingEdges(stripPad, 8, 16, 8, 16);
  host.append(stripPad, stripHost.id);
  host.append(page, stripPad);

  const panel = host.decorated(PANEL, 12);
  host.setStroke(panel, 1, HAIR);
  host.setGrow(panel, 1);
  host.append(page, panel);

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
      typeOf: (item: RowItem) => (item.symbol.startsWith("group:") ? "header" : "row"),
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
      indicator: ({ progress }) => PullLabel({ progress }),
    },
  });
  insert({ id: panel } as NodeHandle, list);

  function renderRow(item: RowItem, _index: number): NodeHandle {
    if (item.firstOfGroup !== undefined) {
      const head = host.padding(0);
      host.setPaddingEdges(head, 12, 16, 4, 16);
      const labelText = host.text(groupLabel(item.firstOfGroup), 11, MUTED);
      host.setFontWeight(labelText, 600);
      host.append(head, labelText);
      return { id: head };
    }
    return WatchRow({
      item: () => item,
      engine: () => engine,
      binder: () => binder,
      updown: () => settingsSig().updown,
      onClick: (symbol) => props.router.push(`/stock/${symbol}`),
    });
  }

  return { id: page };
}

function emptyState(onSearch: () => void): NodeHandle {
  const col = host.column(10);
  const pad = host.padding(0);
  host.setPaddingEdges(pad, 32, 16, 32, 16);
  host.append(pad, col);
  const hint = host.text(loc("还没有自选", "Watchlist is empty"), 14, MUTED);
  const align = host.align("center");
  host.append(align, hint);
  host.append(col, align);
  const cta = host.pointerRegion();
  host.onClick(cta, onSearch);
  host.append(col, cta);
  const pill = host.decorated(PRIMARY, 999);
  host.setMinSize(pill, 0, 36);
  host.append(cta, pill);
  const label = host.text(loc("去搜索", "Search"), 13, colorBytes("#FFFFFF"));
  const center = host.align("center");
  host.append(center, label);
  host.append(pill, center);
  return { id: pad };
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
