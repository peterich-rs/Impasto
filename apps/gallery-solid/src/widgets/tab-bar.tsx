import { scene, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "../colors.ts";
import type { Router } from "@impasto/solid-router";
const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const BG = colorBytes("#FFFFFF");
const ACTIVE = colorBytes("#C8102E");
type Tab = {
  key: string;
  label: string;
  path: string;
};
const TABS: Tab[] = [
  {
    key: "stocks",
    label: "行情",
    path: "/stocks",
  },
  {
    key: "watchlist",
    label: "自选",
    path: "/watchlist",
  },
  {
    key: "market",
    label: "市场",
    path: "/market",
  },
  {
    key: "trade",
    label: "交易",
    path: "/trade",
  },
  {
    key: "profile",
    label: "我的",
    path: "/profile",
  },
];

/**
 * Bottom tab bar. The active tab's ink follows `router.stack()`; switching is
 * `router.reset` (single entry, no stack).
 */
export function TabBar(props: { router: Router }) {
  const bar = scene.createElement("view", {
    style: {
      backgroundColor: BG,
      borderRadius: 0,
    },
  });
  scene.patchStyle(bar, {
    outlineWidth: 1,
    outlineColor: colorBytes("#E8E8EC"),
  });
  const row = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 0,
    },
  });
  scene.append(bar, row);
  const tabs = TABS.map((tab) => {
    const cell = scene.createElement("view", {
      style: {},
    });
    scene.patchStyle(cell, {
      flex: 1,
    });
    scene.onClick(cell, () => props.router.reset(tab.path));
    scene.append(row, cell);
    const pad = scene.createElement("view", {
      style: {
        padding: 0,
      },
    });
    scene.patchStyle(pad, {
      paddingTop: 10,
      paddingRight: 0,
      paddingBottom: 10,
      paddingLeft: 0,
    });
    scene.patchStyle(pad, {
      flex: 1,
    });
    scene.append(cell, pad);
    const text = scene.createElement("text", {
      value: tab.label,
      style: {
        fontSize: 12,
        color: MUTED,
      },
    });
    const center = scene.createElement("view", {
      style: {
        alignItems: "center",
        justifyContent: "center",
      },
    });
    scene.patchStyle(center, {
      flex: 1,
    });
    scene.append(center, text);
    scene.append(pad, center);
    return {
      tab,
      text,
      center,
    };
  });
  return {
    node: bar,
    sync() {
      const top = props.router.stack()[props.router.stack().length - 1];
      const path = top?.path ?? "";
      for (const t of tabs) {
        const active = path.startsWith(`/${t.tab.key}`);
        scene.patchStyle(t.text, {
          color: active ? ACTIVE : MUTED,
        });
        scene.patchStyle(t.text, {
          fontWeight: active ? 600 : 400,
        });
      }
    },
  };
}
