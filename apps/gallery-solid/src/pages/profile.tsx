import { createSignal, createTrackedEffect } from "solid-js";
import { host, type NodeHandle } from "@impasto/solid-renderer";
import { Dialog } from "@impasto/solid-ui";
import { colorBytes } from "../colors.ts";
import { useBinder } from "../state/quotes-provider.tsx";
import { settingsSig, updateSettings } from "../state/settings.ts";
import type { Router } from "@impasto/solid-router";

const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const PANEL = colorBytes("#FFFFFF");
const HAIR = colorBytes("#E8E8EC");

export function ProfilePage(props: { router: Router }): NodeHandle {
  void props;
  useBinder(); // page shows live quotes nowhere; no market consumption
  const [updown, setUpdown] = createSignal(settingsSig().updown);
  const [speed, setSpeed] = createSignal(settingsSig().timeScale);
  const [confirmReset, setConfirmReset] = createSignal(false);

  const page = host.column(10);
  host.setGrow(page, 1);

  const title = host.text("我的", 26, INK);
  host.setFontWeight(title, 700);
  const titlePad = host.padding(0);
  host.setPaddingEdges(titlePad, 12, 16, 0, 16);
  host.append(titlePad, title);
  host.append(page, titlePad);

  // === quote convention ===
  const card1 = sectionCard();
  host.append(page, card1.root);
  const cap1 = host.text("涨跌配色", 13, INK);
  host.setFontWeight(cap1, 600);
  host.append(card1.body, cap1);
  for (const opt of ["cn", "intl"] as const) {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, () => {
      setUpdown(opt);
      updateSettings({ updown: opt });
    });
    host.append(card1.body, cell);
    const pill = host.decorated(PANEL, 8);
    host.setStroke(pill, 1, HAIR);
    host.setMinSize(pill, 0, 34);
    host.append(cell, pill);
    const label = host.text(opt === "cn" ? "红涨绿跌" : "绿涨红跌", 12, MUTED);
    const align = host.align("center");
    host.append(align, label);
    host.append(pill, align);
    createTrackedEffect(() => {
      const active = updown() === opt;
      host.setColor(pill, active ? INK : PANEL);
      host.setColor(label, active ? colorBytes("#FFFFFF") : MUTED);
    });
  }

  // === market speed ===
  const card2 = sectionCard();
  host.append(page, card2.root);
  const cap2 = host.text("行情速度", 13, INK);
  host.setFontWeight(cap2, 600);
  host.append(card2.body, cap2);
  for (const opt of [1, 60, 240]) {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, () => {
      setSpeed(opt);
      updateSettings({ timeScale: opt });
    });
    host.append(card2.body, cell);
    const pill = host.decorated(PANEL, 8);
    host.setStroke(pill, 1, HAIR);
    host.setMinSize(pill, 0, 34);
    host.append(cell, pill);
    const label = host.text(speedLabel(opt), 12, MUTED);
    const align = host.align("center");
    host.append(align, label);
    host.append(pill, align);
    createTrackedEffect(() => {
      const active = speed() === opt;
      host.setColor(pill, active ? INK : PANEL);
      host.setColor(label, active ? colorBytes("#FFFFFF") : MUTED);
    });
  }

  // === about ===
  const card3 = sectionCard();
  host.append(page, card3.root);
  const about = host.text("Impasto Quotes · 模拟盘 · 数据均为虚构", 12, MUTED);
  host.append(card3.body, about);

  void confirmReset;
  void setConfirmReset;
  void Dialog;
  return { id: page };
}

function sectionCard(): { root: number; body: number } {
  const root = host.decorated(PANEL, 12);
  host.setStroke(root, 1, HAIR);
  const pad = host.padding(0);
  host.setPaddingEdges(pad, 14, 16, 14, 16);
  host.append(root, pad);
  const body = host.column(8);
  host.append(pad, body);
  return { root, body };
}

function speedLabel(scale: number): string {
  if (scale === 1) {
    return "实时";
  }
  if (scale === 60) {
    return "60×";
  }
  return "240×";
}
