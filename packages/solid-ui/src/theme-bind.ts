import { directionFor, scene } from "@impasto/solid-renderer";
import { installThemeEnvironment, installThemePort, type Theme, type TypeStyle } from "./theme.ts";

type ThemeTokens = { colors?: Record<string, string>; lengths?: Record<string, number> };
const extensions = new Set<(theme: Theme) => ThemeTokens>();
let activeTheme: Theme | undefined;

/** Supply additional native style tokens for every resolved theme. */
export function extendThemeTokens(resolve: (theme: Theme) => ThemeTokens): () => void {
  extensions.add(resolve);
  try {
    if (activeTheme) writeEnvironment(activeTheme);
  } catch (error) {
    extensions.delete(resolve);
    throw error;
  }
  return () => {
    if (!extensions.delete(resolve)) return;
    try {
      if (activeTheme) writeEnvironment(activeTheme);
    } catch (error) {
      extensions.add(resolve);
      throw error;
    }
  };
}

installThemePort({
  settings: () => scene.settings(),
  onSettings: (fn) => scene.onSettings(fn),
  offSettings: (id) => scene.offSettings(id),
});
installThemeEnvironment(writeEnvironment);

function writeEnvironment(theme: Theme): void {
  activeTheme = theme;
  const scale = theme.fontSize.body / 16;
  const colors: Record<string, string> = { ...theme.color };
  const lengths = Object.fromEntries([
    ...Object.entries(theme.space).map(([name, value]) => [`space.${name}`, value] as const),
    ...Object.entries(theme.radius).map(([name, value]) => [`radius.${name}`, value] as const),
    ...Object.entries(theme.shape).map(([name, value]) => [`shape.${name}`, value] as const),
    ...Object.entries(theme.fontSize).map(([name, value]) => [`fontSize.${name}`, value] as const),
    ...Object.entries(theme.typography).map(([name, value]) => [`typography.${name}.size`, value.size] as const),
  ]);
  for (const resolve of extensions) {
    const extra = resolve(theme);
    Object.assign(colors, extra.colors);
    Object.assign(lengths, extra.lengths);
  }
  scene.setStyleEnvironment({
    direction: directionFor(theme.locale),
    dark: theme.brightness === "dark",
    spacing: theme.space.s,
    rootRem: theme.fontSize.body,
    colors,
    lengths,
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
}

function nativeRole(role: TypeStyle) {
  return { size: role.size, weight: role.weight, lineHeight: role.lineHeight / role.size };
}
