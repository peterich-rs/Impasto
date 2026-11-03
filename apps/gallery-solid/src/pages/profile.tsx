import { createSignal, createTrackedEffect, onCleanup } from "solid-js";
import { host, insert, onFrame, type NodeHandle } from "@impasto/solid-renderer";
import { Dialog } from "@impasto/solid-ui";
import { colorBytes } from "../colors.ts";
import { useBinder } from "../state/quotes-provider.tsx";
import { settingsSig, updateSettings } from "../state/settings.ts";
import { accountRef } from "../state/account-ref.ts";
import { showToast } from "../state/toast.ts";
import { fmtPrice } from "../format.ts";
import { loc } from "../copy.ts";
import type { Router } from "@impasto/solid-router";

const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const PANEL = colorBytes("#FFFFFF");
const HAIR = colorBytes("#E8E8EC");
const PRIMARY = colorBytes("#C8102E");

export function ProfilePage(props: { router: Router }): NodeHandle {
  void props;
  useBinder();
  const account = accountRef();
  const [updown, setUpdown] = createSignal(settingsSig().updown);
  const [speed, setSpeed] = createSignal(settingsSig().timeScale);
  const [locale, setLocale] = createSignal(settingsSig().locale);
  const [confirmReset, setConfirmReset] = createSignal(false);

  const page = host.column(10);
  host.setGrow(page, 1);

  const title = host.text(loc("我的", "Me"), 26, INK);
  host.setFontWeight(title, 700);
  const titlePad = host.padding(0);
  host.setPaddingEdges(titlePad, 12, 16, 0, 16);
  host.append(titlePad, title);
  host.append(page, titlePad);

  const summary = sectionCard();
  host.append(page, summary.root);
  const sumCap = host.text(loc("账户", "Account"), 13, INK);
  host.setFontWeight(sumCap, 600);
  host.append(summary.body, sumCap);
  const cashLine = host.text("", 14, INK);
  host.append(summary.body, cashLine);
  const posLine = host.text("", 12, MUTED);
  host.append(summary.body, posLine);
  let lastPaint = -1;
  onCleanup(
    onFrame((targetMs) => {
      if (lastPaint >= 0 && targetMs - lastPaint < 500) {
        return;
      }
      lastPaint = targetMs;
      const mv = account.marketValue();
      host.setText(cashLine, `${loc("总资产", "Equity")} ${fmtPrice(account.cash() + mv)}`);
      host.setText(
        posLine,
        loc(
          `可用 ${fmtPrice(account.cash())} · 持仓 ${account.positions().length} 只`,
          `Cash ${fmtPrice(account.cash())} · ${account.positions().length} positions`,
        ),
      );
    }),
  );

  const card1 = sectionCard();
  host.append(page, card1.root);
  const cap1 = host.text(loc("涨跌配色", "Up/down colors"), 13, INK);
  host.setFontWeight(cap1, 600);
  host.append(card1.body, cap1);
  const updownRow = host.row(8);
  host.append(card1.body, updownRow);
  for (const opt of ["cn", "intl"] as const) {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, () => {
      setUpdown(opt);
      updateSettings({ updown: opt });
    });
    host.append(updownRow, cell);
    const pill = host.decorated(PANEL, 8);
    host.setStroke(pill, 1, HAIR);
    host.setMinSize(pill, 0, 36);
    host.setGrow(pill, 1);
    host.append(cell, pill);
    const label = host.text(opt === "cn" ? loc("红涨绿跌", "Red up") : loc("绿涨红跌", "Green up"), 12, MUTED);
    const align = host.align("center");
    host.setGrow(align, 1);
    host.append(align, label);
    host.append(pill, align);
    createTrackedEffect(() => {
      const active = updown() === opt;
      host.setColor(pill, active ? INK : PANEL);
      host.setColor(label, active ? colorBytes("#FFFFFF") : MUTED);
    });
  }

  const cardLang = sectionCard();
  host.append(page, cardLang.root);
  const capLang = host.text(loc("语言", "Language"), 13, INK);
  host.setFontWeight(capLang, 600);
  host.append(cardLang.body, capLang);
  const langRow = host.row(8);
  host.append(cardLang.body, langRow);
  for (const opt of ["zh", "en"] as const) {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, () => {
      setLocale(opt);
      updateSettings({ locale: opt });
    });
    host.append(langRow, cell);
    const pill = host.decorated(PANEL, 8);
    host.setStroke(pill, 1, HAIR);
    host.setMinSize(pill, 0, 36);
    host.setGrow(pill, 1);
    host.append(cell, pill);
    const label = host.text(opt === "zh" ? "中文" : "English", 12, MUTED);
    const align = host.align("center");
    host.setGrow(align, 1);
    host.append(align, label);
    host.append(pill, align);
    createTrackedEffect(() => {
      const active = locale() === opt;
      host.setColor(pill, active ? INK : PANEL);
      host.setColor(label, active ? colorBytes("#FFFFFF") : MUTED);
    });
  }

  const card2 = sectionCard();
  host.append(page, card2.root);
  const cap2 = host.text(loc("行情速度", "Market speed"), 13, INK);
  host.setFontWeight(cap2, 600);
  host.append(card2.body, cap2);
  const speedRow = host.row(8);
  host.append(card2.body, speedRow);
  for (const opt of [1, 60, 240]) {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, () => {
      setSpeed(opt);
      updateSettings({ timeScale: opt });
    });
    host.append(speedRow, cell);
    const pill = host.decorated(PANEL, 8);
    host.setStroke(pill, 1, HAIR);
    host.setMinSize(pill, 0, 36);
    host.setGrow(pill, 1);
    host.append(cell, pill);
    const label = host.text(speedLabel(opt), 12, MUTED);
    const align = host.align("center");
    host.setGrow(align, 1);
    host.append(align, label);
    host.append(pill, align);
    createTrackedEffect(() => {
      const active = speed() === opt;
      host.setColor(pill, active ? INK : PANEL);
      host.setColor(label, active ? colorBytes("#FFFFFF") : MUTED);
    });
  }

  const cardReset = sectionCard();
  host.append(page, cardReset.root);
  const resetBtn = host.pointerRegion();
  host.onClick(resetBtn, () => setConfirmReset(true));
  host.append(cardReset.body, resetBtn);
  const resetLabel = host.text(loc("重置模拟账户", "Reset paper account"), 14, PRIMARY);
  host.append(resetBtn, resetLabel);
  const resetHint = host.text(loc("只清空资金、持仓和委托，自选与设置保留", "Clears cash, positions, and orders only"), 11, MUTED);
  host.append(cardReset.body, resetHint);

  const card3 = sectionCard();
  host.append(page, card3.root);
  const about = host.text(loc("Impasto Quotes · 模拟盘 · 数据均为虚构", "Impasto Quotes · paper trading · fictional data"), 12, MUTED);
  host.append(card3.body, about);

  const confirmBody = host.column(8);
  const confirmText = host.text(loc("确认清空模拟账户？自选和设置不会改动。", "Reset the paper account? Watchlist and settings stay."), 14, INK);
  host.append(confirmBody, confirmText);
  const confirmRow = host.row(8);
  host.append(confirmBody, confirmRow);
  const no = host.pointerRegion();
  host.onClick(no, () => setConfirmReset(false));
  host.append(confirmRow, no);
  const noLabel = host.text(loc("取消", "Cancel"), 14, MUTED);
  host.append(no, noLabel);
  const yes = host.pointerRegion();
  host.onClick(yes, () => {
    account.reset();
    setConfirmReset(false);
    showToast(loc("账户已重置", "Account reset"));
  });
  host.append(confirmRow, yes);
  const yesLabel = host.text(loc("确认重置", "Reset"), 14, PRIMARY);
  host.append(yes, yesLabel);

  const dialog = Dialog({
    get open() {
      return confirmReset();
    },
    onClose: () => setConfirmReset(false),
    title: loc("重置账户", "Reset account"),
    children: { id: confirmBody },
  });
  insert({ id: page } as NodeHandle, dialog);

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
    return loc("实时", "Realtime");
  }
  if (scale === 60) {
    return "60×";
  }
  return "240×";
}
