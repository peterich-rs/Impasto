import { createSignal, createTrackedEffect, onCleanup } from "solid-js";
import {
  scene,
  insert,
  onFrame,
  type NodeHandle,
} from "@impasto/solid-renderer";
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
  const page = scene.createElement("view", {
    style: {
      gap: 10,
    },
  });
  scene.patchStyle(page, {
    flex: 1,
  });
  const title = scene.createElement("text", {
    value: loc("我的", "Me"),
    style: {
      fontSize: 26,
      color: INK,
    },
  });
  scene.patchStyle(title, {
    fontWeight: 700,
  });
  const titlePad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(titlePad, {
    paddingTop: 12,
    paddingRight: 16,
    paddingBottom: 0,
    paddingLeft: 16,
  });
  scene.append(titlePad, title);
  scene.append(page, titlePad);
  const summary = sectionCard();
  scene.append(page, summary.root);
  const sumCap = scene.createElement("text", {
    value: loc("账户", "Account"),
    style: {
      fontSize: 13,
      color: INK,
    },
  });
  scene.patchStyle(sumCap, {
    fontWeight: 600,
  });
  scene.append(summary.body, sumCap);
  const cashLine = scene.createElement("text", {
    value: "",
    style: {
      fontSize: 14,
      color: INK,
    },
  });
  scene.append(summary.body, cashLine);
  const posLine = scene.createElement("text", {
    value: "",
    style: {
      fontSize: 12,
      color: MUTED,
    },
  });
  scene.append(summary.body, posLine);
  let lastPaint = -1;
  onCleanup(
    onFrame((targetMs) => {
      if (lastPaint >= 0 && targetMs - lastPaint < 500) {
        return;
      }
      lastPaint = targetMs;
      const mv = account.marketValue();
      scene.setText(
        cashLine,
        `${loc("总资产", "Equity")} ${fmtPrice(account.cash() + mv)}`,
      );
      scene.setText(
        posLine,
        loc(
          `可用 ${fmtPrice(account.cash())} · 持仓 ${account.positions().length} 只`,
          `Cash ${fmtPrice(account.cash())} · ${account.positions().length} positions`,
        ),
      );
    }),
  );
  const card1 = sectionCard();
  scene.append(page, card1.root);
  const cap1 = scene.createElement("text", {
    value: loc("涨跌配色", "Up/down colors"),
    style: {
      fontSize: 13,
      color: INK,
    },
  });
  scene.patchStyle(cap1, {
    fontWeight: 600,
  });
  scene.append(card1.body, cap1);
  const updownRow = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 8,
    },
  });
  scene.append(card1.body, updownRow);
  for (const opt of ["cn", "intl"] as const) {
    const cell = scene.createElement("view", {
      style: {},
    });
    scene.patchStyle(cell, {
      flex: 1,
    });
    scene.onClick(cell, () => {
      setUpdown(opt);
      updateSettings({
        updown: opt,
      });
    });
    scene.append(updownRow, cell);
    const pill = scene.createElement("view", {
      style: {
        backgroundColor: PANEL,
        borderRadius: 8,
      },
    });
    scene.patchStyle(pill, {
      outlineWidth: 1,
      outlineColor: HAIR,
    });
    scene.patchStyle(pill, {
      minWidth: 0,
      minHeight: 36,
    });
    scene.patchStyle(pill, {
      flex: 1,
    });
    scene.append(cell, pill);
    const label = scene.createElement("text", {
      value:
        opt === "cn" ? loc("红涨绿跌", "Red up") : loc("绿涨红跌", "Green up"),
      style: {
        fontSize: 12,
        color: MUTED,
      },
    });
    const align = scene.createElement("view", {
      style: {
        alignItems: "center",
        justifyContent: "center",
      },
    });
    scene.patchStyle(align, {
      flex: 1,
    });
    scene.append(align, label);
    scene.append(pill, align);
    createTrackedEffect(() => {
      const active = updown() === opt;
      scene.patchStyle(pill, {
        backgroundColor: active ? INK : PANEL,
      });
      scene.patchStyle(label, {
        color: active ? colorBytes("#FFFFFF") : MUTED,
      });
    });
  }
  const cardLang = sectionCard();
  scene.append(page, cardLang.root);
  const capLang = scene.createElement("text", {
    value: loc("语言", "Language"),
    style: {
      fontSize: 13,
      color: INK,
    },
  });
  scene.patchStyle(capLang, {
    fontWeight: 600,
  });
  scene.append(cardLang.body, capLang);
  const langRow = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 8,
    },
  });
  scene.append(cardLang.body, langRow);
  for (const opt of ["zh", "en"] as const) {
    const cell = scene.createElement("view", {
      style: {},
    });
    scene.patchStyle(cell, {
      flex: 1,
    });
    scene.onClick(cell, () => {
      setLocale(opt);
      updateSettings({
        locale: opt,
      });
    });
    scene.append(langRow, cell);
    const pill = scene.createElement("view", {
      style: {
        backgroundColor: PANEL,
        borderRadius: 8,
      },
    });
    scene.patchStyle(pill, {
      outlineWidth: 1,
      outlineColor: HAIR,
    });
    scene.patchStyle(pill, {
      minWidth: 0,
      minHeight: 36,
    });
    scene.patchStyle(pill, {
      flex: 1,
    });
    scene.append(cell, pill);
    const label = scene.createElement("text", {
      value: opt === "zh" ? "中文" : "English",
      style: {
        fontSize: 12,
        color: MUTED,
      },
    });
    const align = scene.createElement("view", {
      style: {
        alignItems: "center",
        justifyContent: "center",
      },
    });
    scene.patchStyle(align, {
      flex: 1,
    });
    scene.append(align, label);
    scene.append(pill, align);
    createTrackedEffect(() => {
      const active = locale() === opt;
      scene.patchStyle(pill, {
        backgroundColor: active ? INK : PANEL,
      });
      scene.patchStyle(label, {
        color: active ? colorBytes("#FFFFFF") : MUTED,
      });
    });
  }
  const card2 = sectionCard();
  scene.append(page, card2.root);
  const cap2 = scene.createElement("text", {
    value: loc("行情速度", "Market speed"),
    style: {
      fontSize: 13,
      color: INK,
    },
  });
  scene.patchStyle(cap2, {
    fontWeight: 600,
  });
  scene.append(card2.body, cap2);
  const speedRow = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 8,
    },
  });
  scene.append(card2.body, speedRow);
  for (const opt of [1, 60, 240]) {
    const cell = scene.createElement("view", {
      style: {},
    });
    scene.patchStyle(cell, {
      flex: 1,
    });
    scene.onClick(cell, () => {
      setSpeed(opt);
      updateSettings({
        timeScale: opt,
      });
    });
    scene.append(speedRow, cell);
    const pill = scene.createElement("view", {
      style: {
        backgroundColor: PANEL,
        borderRadius: 8,
      },
    });
    scene.patchStyle(pill, {
      outlineWidth: 1,
      outlineColor: HAIR,
    });
    scene.patchStyle(pill, {
      minWidth: 0,
      minHeight: 36,
    });
    scene.patchStyle(pill, {
      flex: 1,
    });
    scene.append(cell, pill);
    const label = scene.createElement("text", {
      value: speedLabel(opt),
      style: {
        fontSize: 12,
        color: MUTED,
      },
    });
    const align = scene.createElement("view", {
      style: {
        alignItems: "center",
        justifyContent: "center",
      },
    });
    scene.patchStyle(align, {
      flex: 1,
    });
    scene.append(align, label);
    scene.append(pill, align);
    createTrackedEffect(() => {
      const active = speed() === opt;
      scene.patchStyle(pill, {
        backgroundColor: active ? INK : PANEL,
      });
      scene.patchStyle(label, {
        color: active ? colorBytes("#FFFFFF") : MUTED,
      });
    });
  }
  const cardReset = sectionCard();
  scene.append(page, cardReset.root);
  const resetBtn = scene.createElement("view", {
    style: {},
  });
  scene.onClick(resetBtn, () => setConfirmReset(true));
  scene.append(cardReset.body, resetBtn);
  const resetLabel = scene.createElement("text", {
    value: loc("重置模拟账户", "Reset paper account"),
    style: {
      fontSize: 14,
      color: PRIMARY,
    },
  });
  scene.append(resetBtn, resetLabel);
  const resetHint = scene.createElement("text", {
    value: loc(
      "只清空资金、持仓和委托，自选与设置保留",
      "Clears cash, positions, and orders only",
    ),
    style: {
      fontSize: 11,
      color: MUTED,
    },
  });
  scene.append(cardReset.body, resetHint);
  const card3 = sectionCard();
  scene.append(page, card3.root);
  const canvasBtn = scene.createElement("view", {
    style: { paddingBottom: 10 },
  });
  scene.onClick(canvasBtn, () => props.router.push("/canvas"));
  const canvasLabel = scene.createElement("text", {
    value: loc("绘图", "Canvas"),
    style: { fontSize: 14, color: INK },
  });
  scene.append(canvasBtn, canvasLabel);
  scene.append(card3.body, canvasBtn);
  const gpuLab = scene.createElement("view", {
    style: { paddingBottom: 12 },
  });
  scene.onClick(gpuLab, () => props.router.push("/gpu-lab"));
  scene.append(gpuLab, scene.createElement("text", {
    value: loc("GPU 实验室", "GPU Lab"),
    style: { fontSize: 14, color: INK },
  }));
  scene.append(card3.body, gpuLab);
  const compositorLab = scene.createElement("view", {
    style: { paddingBottom: 12 },
  });
  scene.onClick(compositorLab, () => props.router.push("/compositor-lab"));
  scene.append(compositorLab, scene.createElement("text", {
    value: loc("合成动画", "Compositor animation"),
    style: { fontSize: 14, color: INK },
  }));
  scene.append(card3.body, compositorLab);
  const about = scene.createElement("text", {
    value: loc(
      "Impasto Quotes · 模拟盘 · 数据均为虚构",
      "Impasto Quotes · paper trading · fictional data",
    ),
    style: {
      fontSize: 12,
      color: MUTED,
    },
  });
  scene.append(card3.body, about);
  const confirmBody = scene.createElement("view", {
    style: {
      gap: 8,
    },
  });
  const confirmText = scene.createElement("text", {
    value: loc(
      "确认清空模拟账户？自选和设置不会改动。",
      "Reset the paper account? Watchlist and settings stay.",
    ),
    style: {
      fontSize: 14,
      color: INK,
    },
  });
  scene.append(confirmBody, confirmText);
  const confirmRow = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 8,
    },
  });
  scene.append(confirmBody, confirmRow);
  const no = scene.createElement("view", {
    style: {},
  });
  scene.onClick(no, () => setConfirmReset(false));
  scene.append(confirmRow, no);
  const noLabel = scene.createElement("text", {
    value: loc("取消", "Cancel"),
    style: {
      fontSize: 14,
      color: MUTED,
    },
  });
  scene.append(no, noLabel);
  const yes = scene.createElement("view", {
    style: {},
  });
  scene.onClick(yes, () => {
    account.reset();
    setConfirmReset(false);
    showToast(loc("账户已重置", "Account reset"));
  });
  scene.append(confirmRow, yes);
  const yesLabel = scene.createElement("text", {
    value: loc("确认重置", "Reset"),
    style: {
      fontSize: 14,
      color: PRIMARY,
    },
  });
  scene.append(yes, yesLabel);
  const dialog = Dialog({
    get open() {
      return confirmReset();
    },
    onClose: () => setConfirmReset(false),
    title: loc("重置账户", "Reset account"),
    children: confirmBody,
  });
  insert(page, dialog);
  return page;
}
function sectionCard(): {
  root: number;
  body: number;
} {
  const root = scene.createElement("view", {
    style: {
      backgroundColor: PANEL,
      borderRadius: 12,
    },
  });
  scene.patchStyle(root, {
    outlineWidth: 1,
    outlineColor: HAIR,
  });
  const pad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(pad, {
    paddingTop: 14,
    paddingRight: 16,
    paddingBottom: 14,
    paddingLeft: 16,
  });
  scene.append(root, pad);
  const body = scene.createElement("view", {
    style: {
      gap: 8,
    },
  });
  scene.append(pad, body);
  return {
    root,
    body,
  };
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
