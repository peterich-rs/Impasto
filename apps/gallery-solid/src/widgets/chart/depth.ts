import { onCleanup } from "solid-js";
import { scene, onLayout, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "../../colors.ts";
import type { QuoteEngine } from "../../engine/engine.ts";
import { round2 } from "../../engine/universe.ts";
import { toneFor, type Updown } from "../../state/settings.ts";
import { fmtVolumeCN } from "../../format.ts";
const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const UP_BG = colorBytes("#FDECEC");
const DOWN_BG = colorBytes("#E8F7EF");

/**
 * Ten-level depth ladder. Prices are deterministic around the last tick; the
 * volume bars widen toward the middle of the book. Re-annotated on each spark
 * refresh (~1Hz), never per frame.
 */
export function DepthLadder(props: {
  symbol: () => string;
  engine: () => QuoteEngine;
  updown: () => Updown;
  onPick?: (price: number) => void;
}) {
  const root = scene.createElement("view", {
    style: {
      gap: 2,
    },
  });
  let lane = 240;
  const askRows: {
    price: number;
    vol: number;
    bar: number;
    level: number;
    side: "ask" | "bid";
  }[] = [];
  const bidRows: {
    price: number;
    vol: number;
    bar: number;
    level: number;
    side: "ask" | "bid";
  }[] = [];

  // asks (reversed so ask5 is on top, ask1 nearest the spread)
  for (let level = 5; level >= 1; level--) {
    askRows.push(buildRow(level, "ask"));
  }
  const spread = scene.createElement("view", {
    style: {
      gap: 2,
    },
  });
  scene.append(root, spread);
  const spreadLabel = scene.createElement("text", {
    value: "—",
    style: {
      fontSize: 13,
      color: INK,
    },
  });
  scene.patchStyle(spreadLabel, {
    fontWeight: 600,
  });
  scene.append(spread, spreadLabel);
  const spreadHint = scene.createElement("text", {
    value: "",
    style: {
      fontSize: 10,
      color: MUTED,
    },
  });
  scene.append(spread, spreadHint);
  for (let level = 1; level <= 5; level++) {
    bidRows.push(buildRow(level, "bid"));
  }
  function buildRow(level: number, side: "ask" | "bid") {
    const row = scene.createElement("view", {
      style: {},
    });
    scene.patchStyle(row, {
      minWidth: 0,
      minHeight: 22,
    });
    scene.append(root, row);
    const bar = scene.createElement("view", {
      style: {
        backgroundColor: side === "ask" ? UP_BG : DOWN_BG,
        borderRadius: 2,
      },
    });
    scene.patchStyle(bar, {
      position: "absolute",
    });
    scene.patchStyle(bar, {
      top: 0,
      right: 0,
      bottom: 0,
      left: 0,
    });
    scene.append(row, bar);
    const price = scene.createElement("text", {
      value: "—",
      style: {
        fontSize: 12,
        color: INK,
      },
    });
    const vol = scene.createElement("text", {
      value: "—",
      style: {
        fontSize: 11,
        color: MUTED,
      },
    });
    const pad = scene.createElement("view", {
      style: {
        padding: 0,
      },
    });
    scene.patchStyle(pad, {
      paddingTop: 4,
      paddingRight: 8,
      paddingBottom: 4,
      paddingLeft: 8,
    });
    scene.patchStyle(pad, {
      flex: 1,
    });
    scene.append(row, pad);
    const line = scene.createElement("view", {
      style: {
        flexDirection: "row",
        gap: 4,
      },
    });
    scene.append(pad, line);
    scene.append(line, price);
    const spacer = scene.createElement("view", {
      style: {
        width: 0,
        height: 0,
      },
    });
    scene.patchStyle(spacer, {
      flex: 1,
    });
    scene.append(line, spacer);
    scene.append(line, vol);
    scene.onClick(row, () => {
      const q = props.engine().quote(props.symbol());
      const step = q.last * 0.001;
      const at = round2(q.last + (side === "ask" ? 1 : -1) * step * level);
      props.onPick?.(at);
    });
    return {
      price,
      vol,
      bar,
      level,
      side: side as "ask" | "bid",
    };
  }
  function refresh(): void {
    const engine = props.engine();
    const symbol = props.symbol();
    const q = engine.quote(symbol);
    const rand = pseudoFor(symbol, q.revision);
    const step = Math.max(0.01, q.last * 0.001);
    const maxBar = Math.max(72, Math.round(lane * 0.72));
    const spreadNow = round2(step * 2);
    scene.setText(spreadLabel, fmtSpread(q.last));
    scene.setText(spreadHint, `价差 ${spreadNow.toFixed(2)}`);
    scene.patchStyle(spreadLabel, {
      color: toneColor(q.last, q.prevClose),
    });
    for (const row of [...askRows, ...bidRows]) {
      const dir = row.side === "ask" ? 1 : -1;
      const price = round2(q.last + dir * step * row.level);
      const vol =
        Math.floor(80 + rand() * 900) *
        (engine.ins(symbol).lot === 1 ? 1 : 100);
      scene.setText(row.price, price.toFixed(2));
      scene.setText(row.vol, fmtVolumeCN(vol));
      scene.patchStyle(row.price, {
        color: toneColor(q.last, q.prevClose),
      });
      const w = Math.max(18, Math.round(maxBar * (0.2 + 0.8 * rand())));
      scene.patchStyle(row.bar, {
        minWidth: w,
        minHeight: 16,
      });
    }
  }
  function toneColor(
    last: number,
    prevClose: number,
  ): [number, number, number, number] {
    const tone = toneFor(last, prevClose, props.updown());
    if (tone === "up") {
      return colorBytes("#EB3E3E");
    }
    if (tone === "down") {
      return colorBytes("#22B05F");
    }
    return MUTED;
  }
  refresh();
  onCleanup(
    onLayout(() => {
      const box = scene.rect(root);
      if (box.w > 40 && Math.abs(box.w - lane) > 1) {
        lane = box.w;
        refresh();
      }
    }),
  );
  return {
    node: root,
    refresh,
  };
}
function fmtSpread(last: number): string {
  return last.toFixed(2);
}
function pseudoFor(symbol: string, revision: number): () => number {
  let x = (revision * 2654435761) ^ hashLite(symbol);
  return () => {
    x = (Math.imul(x, 48271) + 11) >>> 0;
    return (x % 10000) / 10000;
  };
}
function hashLite(value: string): number {
  let h = 5381;
  for (let i = 0; i < value.length; i++) {
    h = (Math.imul(h, 33) ^ value.charCodeAt(i)) >>> 0;
  }
  return h;
}
