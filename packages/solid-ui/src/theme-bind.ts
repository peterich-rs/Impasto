import { directionFor, scene } from "@impasto/solid-renderer";
import {
  installThemeEnvironment,
  installThemePort,
  type TypeStyle,
} from "./theme.ts";

installThemePort({
  settings: () => scene.settings(),
  onSettings: (fn) => scene.onSettings(fn),
  offSettings: (id) => scene.offSettings(id),
});
installThemeEnvironment((theme) => {
  const scale = theme.fontSize.body / 16;
  scene.setStyleEnvironment({
    direction: directionFor(theme.locale),
    dark: theme.brightness === "dark",
    spacing: theme.space.s,
    rootRem: theme.fontSize.body,
    colors: theme.color,
    radii: [
      theme.shape.none, theme.shape.xs, theme.shape.m, theme.space.m,
      theme.shape.l, theme.space.l, theme.shape.full,
    ],
    fontSizes: [
      12 * scale, 14 * scale, 16 * scale, 18 * scale,
      20 * scale, 24 * scale, 30 * scale, 36 * scale,
    ],
    roles: {
      display: nativeRole(theme.typography.display),
      headline: nativeRole(theme.typography.headline),
      title: nativeRole(theme.typography.title),
      body: nativeRole(theme.typography.body),
      label: nativeRole(theme.typography.label),
      caption: nativeRole(theme.typography.caption),
    },
  });
});

function nativeRole(role: TypeStyle) {
  return { size: role.size, weight: role.weight, lineHeight: role.lineHeight / role.size };
}
