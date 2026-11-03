import { createSignal, createTrackedEffect, onCleanup } from "solid-js";
import { host, insert, onFrame, requestFrame, type NodeHandle } from "@impasto/solid-renderer";
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
  let pending = "";
  let dueAt = 0;
  let fieldCtl: FieldController | null = null;

  void loadSearchHistory();

  const page = host.column(8);
  host.setGrow(page, 1);

  const nav = host.row(8);
  const navPad = host.padding(0);
  host.setPaddingEdges(navPad, 12, 16, 0, 16);
  host.append(navPad, nav);
  host.append(page, navPad);
  const back = host.pointerRegion();
  host.onClick(back, () => props.router.pop());
  host.append(nav, back);
  const backLabel = host.text(loc("‹ 返回", "‹ Back"), 14, PRIMARY);
  host.append(back, backLabel);
  const title = host.text(loc("搜索", "Search"), 20, INK);
  host.setFontWeight(title, 700);
  host.append(nav, title);

  const fieldPad = host.padding(0);
  host.setPaddingEdges(fieldPad, 0, 16, 0, 16);
  host.append(page, fieldPad);
  const field = Field({
    placeholder: loc("代码或名称", "Symbol or name"),
    action: "search",
    label: loc("搜索", "Search"),
    controller: (ctl) => {
      fieldCtl = ctl;
    },
    onChange: (value) => {
      pending = value;
      dueAt = -1;
      requestFrame();
    },
    onSubmit: (value) => {
      pending = value;
      dueAt = 0;
      applyTerm(value, true);
    },
  });
  insert({ id: fieldPad } as NodeHandle, field);

  onCleanup(
    onFrame((targetMs) => {
      if (dueAt < 0) {
        dueAt = targetMs + DEBOUNCE_MS;
      }
      if (dueAt > 0 && targetMs >= dueAt) {
        dueAt = 0;
        applyTerm(pending, false);
        return;
      }
      if (dueAt !== 0) {
        requestFrame();
      }
    }),
  );

  function applyTerm(value: string, remember: boolean): void {
    const next = value.trim();
    setTerm(next);
    if (remember && next.length > 0) {
      pushSearchHistory(next);
    }
  }

  const body = host.column(10);
  host.setGrow(body, 1);
  const bodyPad = host.padding(0);
  host.setPaddingEdges(bodyPad, 0, 16, 16, 16);
  host.append(bodyPad, body);
  host.append(page, bodyPad);

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
      host.append(body, sectionTitle(loc("热门", "Hot")));
      host.append(body, chipRow(HOT_SYMBOLS, (symbol) => {
        pending = symbol;
        dueAt = 0;
        fieldCtl?.load(symbol);
        applyTerm(symbol, true);
        props.router.push(`/stock/${symbol}`);
      }));
      const hist = searchHistorySig();
      if (hist.length > 0) {
        const head = host.row(8);
        host.append(body, head);
        const cap = host.text(loc("历史", "History"), 13, INK);
        host.setFontWeight(cap, 600);
        host.append(head, cap);
        const grow = host.sized(0, 0);
        host.setGrow(grow, 1);
        host.append(head, grow);
        const clear = host.pointerRegion();
        host.onClick(clear, () => clearSearchHistory());
        host.append(head, clear);
        const clearLabel = host.text(loc("清除", "Clear"), 12, PRIMARY);
        host.append(clear, clearLabel);
        host.append(body, chipRow(hist, (item) => {
          pending = item;
          dueAt = 0;
          fieldCtl?.load(item);
          applyTerm(item, true);
        }));
      }
      return;
    }

    const hits = engine
      .instruments()
      .filter(
        (ins) =>
          ins.symbol.toLowerCase().includes(t) || ins.name.toLowerCase().includes(t),
      )
      .slice(0, 16);
    if (hits.length === 0) {
      const empty = host.text(loc("没有匹配的标的", "No matching symbols"), 13, MUTED);
      host.append(body, padded(empty, 16, 0, 0, 0));
      return;
    }
    for (const ins of hits) {
      const row = host.pointerRegion();
      host.onClick(row, () => props.router.push(`/stock/${ins.symbol}`));
      const pad = host.padding(0);
      host.setPaddingEdges(pad, 8, 0, 8, 0);
      host.append(row, pad);
      const line = host.row(8);
      host.setMinSize(line, 0, 36);
      host.append(pad, line);
      const name = host.text(ins.name, 14, INK);
      host.append(line, name);
      const sym = host.text(ins.symbol, 11, MUTED);
      host.append(line, sym);
      const spacer = host.sized(0, 0);
      host.setGrow(spacer, 1);
      host.append(line, spacer);
      const last = host.text("—", 13, INK);
      host.append(line, last);
      stops.push(binder.bind(ins.symbol, { last, flashNode: last }));
      host.append(body, row);
    }
  });

  return { id: page };
}

function sectionTitle(text: string): number {
  const label = host.text(text, 13, INK);
  host.setFontWeight(label, 600);
  return label;
}

function chipRow(items: readonly string[], onClick: (item: string) => void): number {
  const row = host.row(8);
  host.setWrap(row, true);
  for (const item of items) {
    const cell = host.pointerRegion();
    host.onClick(cell, () => onClick(item));
    host.append(row, cell);
    const pill = host.decorated(PANEL, 999);
    host.setStroke(pill, 1, HAIR);
    host.setMinSize(pill, 0, 28);
    host.append(cell, pill);
    const label = host.text(item, 11, INK);
    host.append(pill, padded(label, 6, 12, 6, 12));
  }
  return row;
}
