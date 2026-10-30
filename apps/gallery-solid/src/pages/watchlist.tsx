import { createMemo, createSignal } from "solid-js";
import {
  Text,
  host,
  insert,
  requestFrame,
  type NodeHandle,
} from "@impasto/solid-renderer";
import { RefreshControl, VirtualList, createScrollController } from "@impasto/solid-ui";
import { colorBytes } from "../colors.ts";
import { loadWatchlist, watchlistSig } from "../state/watchlist.ts";
import { settingsSig } from "../state/settings.ts";
import { useQuotes } from "../state/quotes-provider.tsx";
import { WatchRow, type RowItem } from "../widgets/watch-row.tsx";
import { IndexStrip } from "../widgets/index-strip.tsx";
import type { Router } from "@impasto/solid-router";

const PANEL = colorBytes("#FFFFFF");
const HAIR = colorBytes("#E8E8EC");
const MUTED = colorBytes("#8A8D91");
const INK = colorBytes("#1F2430");

export function WatchlistPage(props: { router: Router }): NodeHandle {
  const { engine, binder } = useQuotes();
  const controller = createScrollController();
  const [refreshCount, setRefreshCount] = createSignal(0);

  void loadWatchlist();

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
  const title = host.text("自选", 26, INK);
  host.setFontWeight(title, 700);
  host.append(header, title);

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

  // Library components return Solid accessors; `insert` mounts them.
  const list = VirtualList({
    items: items(),
    itemKey: (item: RowItem) => item.symbol,
    estimatedHeight: 76,
    restoreKey: "watchlist",
    controller,
    recycle: {
      typeOf: (item: RowItem) => (item.symbol.startsWith("group:") ? "header" : "row"),
      render: (item, index) => renderRow(item(), index()),
    },
    onRefresh: () => setRefreshCount((n) => n + 1),
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

  void refreshCount;
  void Text;
  void RefreshControl;
  void requestFrame;
  return { id: page };
}

function groupLabel(market: string): string {
  if (market === "hk") {
    return "港股";
  }
  if (market === "us") {
    return "美股";
  }
  return "A股";
}
