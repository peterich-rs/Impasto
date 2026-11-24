import { createSignal, createTrackedEffect, onCleanup } from "solid-js";
import {
  scene,
  insert,
  type NodeHandle,
} from "@impasto/solid-renderer";
import { Field, type FieldController } from "@impasto/solid-ui";
import { colorBytes } from "../colors.ts";
import { clearChildren, padded } from "../widgets/palette.ts";
import { useBinder } from "../state/quotes-provider.tsx";
import {
  HOT_SYMBOLS,
  clearSearchHistory,
  loadSearchHistory,
  pushSearchHistory,
  searchHistorySig,
} from "../state/search-history.ts";
import { loc } from "../copy.ts";
import type { Router } from "@impasto/solid-router";
const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const PANEL = colorBytes("#FFFFFF");
const HAIR = colorBytes("#E8E8EC");
const PRIMARY = colorBytes("#C8102E");
const DEBOUNCE_MS = 200;
export function SearchPage(props: { router: Router }): NodeHandle {
  const { engine, binder } = useBinder();
  const [term, setTerm] = createSignal("");
  let debounce: ReturnType<typeof setTimeout> | undefined;
  let fieldCtl: FieldController | null = null;
  void loadSearchHistory();
  const page = scene.createElement("view", {
    style: {
      gap: 8,
    },
  });
  scene.patchStyle(page, {
    flex: 1,
  });
  const nav = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 8,
    },
  });
  const navPad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(navPad, {
    paddingTop: 12,
    paddingRight: 16,
    paddingBottom: 0,
    paddingLeft: 16,
  });
  scene.append(navPad, nav);
  scene.append(page, navPad);
  const back = scene.createElement("view", {
    style: {},
  });
  scene.onClick(back, () => props.router.pop());
  scene.append(nav, back);
  const backLabel = scene.createElement("text", {
    value: loc("‹ 返回", "‹ Back"),
    style: {
      fontSize: 14,
      color: PRIMARY,
    },
  });
  scene.append(back, backLabel);
  const title = scene.createElement("text", {
    value: loc("搜索", "Search"),
    style: {
      fontSize: 20,
      color: INK,
    },
  });
  scene.patchStyle(title, {
    fontWeight: 700,
  });
  scene.append(nav, title);
  const fieldPad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(fieldPad, {
    paddingTop: 0,
    paddingRight: 16,
    paddingBottom: 0,
    paddingLeft: 16,
  });
  scene.append(page, fieldPad);
  const field = Field({
    placeholder: loc("代码或名称", "Symbol or name"),
    action: "search",
    label: loc("搜索", "Search"),
    controller: (ctl) => {
      fieldCtl = ctl;
    },
    onChange: (value) => {
      clearTimeout(debounce);
      debounce = setTimeout(() => applyTerm(value, false), DEBOUNCE_MS);
    },
    onSubmit: (value) => {
      clearTimeout(debounce);
      applyTerm(value, true);
    },
  });
  insert(fieldPad, field);
  onCleanup(() => clearTimeout(debounce));
  function applyTerm(value: string, remember: boolean): void {
    const next = value.trim();
    setTerm(next);
    if (remember && next.length > 0) {
      pushSearchHistory(next);
    }
  }
  const body = scene.createElement("view", {
    style: {
      gap: 10,
    },
  });
  scene.patchStyle(body, {
    flex: 1,
  });
  const bodyPad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(bodyPad, {
    paddingTop: 0,
    paddingRight: 16,
    paddingBottom: 16,
    paddingLeft: 16,
  });
  scene.append(bodyPad, body);
  scene.append(page, bodyPad);
  createTrackedEffect(() => {
    const stops: Array<() => void> = [];
    onCleanup(() => {
      for (const stop of stops) {
        stop();
      }
    });
    clearChildren(body);
    const t = term().trim().toLowerCase();
    if (t.length === 0) {
      scene.append(body, sectionTitle(loc("热门", "Hot")));
      scene.append(
        body,
        chipRow(HOT_SYMBOLS, (symbol) => {
          pending = symbol;
          dueAt = 0;
          fieldCtl?.load(symbol);
          applyTerm(symbol, true);
          props.router.push(`/stock/${symbol}`);
        }),
      );
      const hist = searchHistorySig();
      if (hist.length > 0) {
        const head = scene.createElement("view", {
          style: {
            flexDirection: "row",
            gap: 8,
          },
        });
        scene.append(body, head);
        const cap = scene.createElement("text", {
          value: loc("历史", "History"),
          style: {
            fontSize: 13,
            color: INK,
          },
        });
        scene.patchStyle(cap, {
          fontWeight: 600,
        });
        scene.append(head, cap);
        const grow = scene.createElement("view", {
          style: {
            width: 0,
            height: 0,
          },
        });
        scene.patchStyle(grow, {
          flex: 1,
        });
        scene.append(head, grow);
        const clear = scene.createElement("view", {
          style: {},
        });
        scene.onClick(clear, () => clearSearchHistory());
        scene.append(head, clear);
        const clearLabel = scene.createElement("text", {
          value: loc("清除", "Clear"),
          style: {
            fontSize: 12,
            color: PRIMARY,
          },
        });
        scene.append(clear, clearLabel);
        scene.append(
          body,
          chipRow(hist, (item) => {
            pending = item;
            dueAt = 0;
            fieldCtl?.load(item);
            applyTerm(item, true);
          }),
        );
      }
      return;
    }
    const hits = engine
      .instruments()
      .filter(
        (ins) =>
          ins.symbol.toLowerCase().includes(t) ||
          ins.name.toLowerCase().includes(t),
      )
      .slice(0, 16);
    if (hits.length === 0) {
      const empty = scene.createElement("text", {
        value: loc("没有匹配的标的", "No matching symbols"),
        style: {
          fontSize: 13,
          color: MUTED,
        },
      });
      scene.append(body, padded(empty, 16, 0, 0, 0));
      return;
    }
    for (const ins of hits) {
      const row = scene.createElement("view", {
        style: {},
      });
      scene.onClick(row, () => props.router.push(`/stock/${ins.symbol}`));
      const pad = scene.createElement("view", {
        style: {
          padding: 0,
        },
      });
      scene.patchStyle(pad, {
        paddingTop: 8,
        paddingRight: 0,
        paddingBottom: 8,
        paddingLeft: 0,
      });
      scene.patchStyle(pad, {
        flex: 1,
      });
      scene.append(row, pad);
      const line = scene.createElement("view", {
        style: {
          flexDirection: "row",
          gap: 8,
        },
      });
      scene.patchStyle(line, {
        minWidth: 0,
        minHeight: 40,
      });
      scene.patchStyle(line, {
        alignItems: "center",
      });
      scene.append(pad, line);
      const name = scene.createElement("text", {
        value: ins.name,
        style: {
          fontSize: 14,
          color: INK,
        },
      });
      scene.append(line, name);
      const sym = scene.createElement("text", {
        value: ins.symbol,
        style: {
          fontSize: 11,
          color: MUTED,
        },
      });
      scene.append(line, sym);
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
      const last = scene.createElement("text", {
        value: "—",
        style: {
          fontSize: 13,
          color: INK,
        },
      });
      scene.append(line, last);
      stops.push(
        binder.bind(ins.symbol, {
          last,
          flashNode: last,
        }),
      );
      scene.append(body, row);
    }
  });
  return page;
}
function sectionTitle(text: string): number {
  const label = scene.createElement("text", {
    value: text,
    style: {
      fontSize: 13,
      color: INK,
    },
  });
  scene.patchStyle(label, {
    fontWeight: 600,
  });
  return label;
}
function chipRow(
  items: readonly string[],
  onClick: (item: string) => void,
): number {
  const row = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 8,
    },
  });
  scene.patchStyle(row, {
    flexWrap: true ? "wrap" : "nowrap",
  });
  for (const item of items) {
    const cell = scene.createElement("view", {
      style: {},
    });
    scene.onClick(cell, () => onClick(item));
    scene.append(row, cell);
    const pill = scene.createElement("view", {
      style: {
        backgroundColor: PANEL,
        borderRadius: 999,
      },
    });
    scene.patchStyle(pill, {
      outlineWidth: 1,
      outlineColor: HAIR,
    });
    scene.patchStyle(pill, {
      minWidth: 0,
      minHeight: 28,
    });
    scene.append(cell, pill);
    const label = scene.createElement("text", {
      value: item,
      style: {
        fontSize: 11,
        color: INK,
      },
    });
    scene.append(pill, padded(label, 6, 12, 6, 12));
  }
  return row;
}
