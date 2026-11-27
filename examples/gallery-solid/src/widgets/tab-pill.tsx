import { createMemo, createTrackedEffect, For, Show } from "solid-js";
import { scene } from "@impasto/solid-renderer";
import type { NodeHandle } from "@impasto/solid-renderer";
import { Pressable, Role } from "@impasto/solid-ui";
import type { Router } from "@impasto/solid-router";
import { loc } from "../copy.ts";
import { accountRef } from "../state/account-ref.ts";
import { colors } from "./stocks/theme.ts";
import { Icon, type IconName } from "./stocks/icons.tsx";

type Tab = { key: "watchlist" | "account" | "settings"; label: string; icon: IconName };
const TABS: readonly Tab[] = [
  { key: "watchlist", label: loc("自选", "Watchlist"), icon: "star" },
  { key: "account", label: loc("账户", "Account"), icon: "wallet" },
  { key: "settings", label: loc("设置", "Settings"), icon: "gear" },
];

/**
 * Floating route capsule. Tab selection restores the branch stack without a slide.
 * Hidden while a page is pushed on top so detail screens own the full height.
 */
export function TabPill(props: { router: Router }): NodeHandle {
  const top = () => props.router.stack().at(-1);
  const active = createMemo(() => {
    const path = top()?.path ?? "";
    return TABS.find((tab) => path === `/${tab.key}`) ?? null;
  });
  const visible = () => active() !== null && props.router.depth() === 1;
  const root = scene.createElement("view");
  // Both horizontal insets make the parent the alignment box. `justifySelf`
  // centers the shrink-wrapped capsule, so the hit target stays the capsule
  // and does not cover the list on either side.
  scene.patchStyle(root, {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: 16,
    justifySelf: "center",
  });
  const pill = scene.createElement("view");
  scene.patchStyle(pill, {
    flexDirection: "row",
    alignItems: "center",
    gap: 2,
    paddingLeft: 4,
    paddingRight: 4,
    height: 56,
    borderRadius: 28,
    backgroundColor: colors.bar,
    borderWidth: 1,
    borderColor: colors.line,
    shadow: { color: "#000000A0", elevation: 6 },
  });
  scene.append(root, pill);
  const cells = TABS.map((tab) => {
    const cell = Pressable({
      role: Role.Button,
      accessibilityLabel: tab.label,
      onClick: () => props.router.tabs?.select(tab.key),
      style: {
        flexDirection: "row",
        alignItems: "center",
        gap: 6,
        height: 44,
        paddingLeft: 14,
        paddingRight: 14,
        borderRadius: 22,
        backgroundColor: "#00000000",
      },
    });
    scene.append(pill, cell);
    const glyph = Icon({ name: tab.icon, size: 18 });
    scene.append(cell, glyph);
    const label = scene.createElement("text", {
      value: tab.label,
      style: { fontSize: 14, fontWeight: 600, color: colors.dim },
    });
    scene.append(cell, label);
    return { tab, cell, glyph, label };
  });
  createTrackedEffect(() => {
    const current = active();
    scene.patchStyle(root, { display: visible() ? "flex" : "none" });
    for (const entry of cells) {
      const selected = current?.key === entry.tab.key;
      scene.patchStyle(entry.cell, { backgroundColor: selected ? colors.raised : "#00000000" });
      scene.patchStyle(entry.label, { color: selected ? colors.ink : colors.dim });
      scene.patchStyle(entry.glyph, { opacity: selected ? 1 : 0.6 });
    }
  });
  return root;
}
