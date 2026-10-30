/** Default palette. Hex literals for theme colors live here. */

export type Palette = {
  bg: string;
  background: string;
  onBackground: string;
  surface: string;
  onSurface: string;
  surfaceContainer: string;
  onSurfaceContainer: string;
  text: string;
  textMuted: string;
  accent: string;
  primary: string;
  onPrimary: string;
  primaryContainer: string;
  onPrimaryContainer: string;
  danger: string;
  success: string;
  warning: string;
  focusRing: string;
  scrim: string;
  outline: string;
  outlineVariant: string;
  transparent: string;
};

export const LIGHT: Palette = {
  bg: "#F3F6F4",
  background: "#F3F6F4",
  onBackground: "#14201A",
  surface: "#FFFFFF",
  onSurface: "#14201A",
  surfaceContainer: "#E5EEE8",
  onSurfaceContainer: "#14201A",
  text: "#14201A",
  textMuted: "#5C6B63",
  accent: "#127A32",
  primary: "#127A32",
  onPrimary: "#FFFFFF",
  primaryContainer: "#D7F5E3",
  onPrimaryContainer: "#06401C",
  danger: "#C83B3B",
  success: "#127A32",
  warning: "#A56A00",
  focusRing: "#127A32",
  scrim: "#00000080",
  outline: "#5C6B63",
  outlineVariant: "#D5E0D9",
  transparent: "#00000000",
};

export const DARK: Palette = {
  bg: "#101412",
  background: "#101412",
  onBackground: "#F4F7F5",
  surface: "#1C2420",
  onSurface: "#F4F7F5",
  surfaceContainer: "#27312C",
  onSurfaceContainer: "#F4F7F5",
  text: "#F4F7F5",
  textMuted: "#9AA89F",
  accent: "#3DDC84",
  primary: "#3DDC84",
  onPrimary: "#052E16",
  primaryContainer: "#143524",
  onPrimaryContainer: "#C8F5D8",
  danger: "#FF6B6B",
  success: "#3DDC84",
  warning: "#E6B15A",
  focusRing: "#3DDC84",
  scrim: "#00000099",
  outline: "#6D7C74",
  outlineVariant: "#2C3832",
  transparent: "#00000000",
};

export const HIGH_LIGHT: Palette = {
  bg: "#FFFFFF",
  background: "#FFFFFF",
  onBackground: "#000000",
  surface: "#FFFFFF",
  onSurface: "#000000",
  surfaceContainer: "#FFFFFF",
  onSurfaceContainer: "#000000",
  text: "#000000",
  textMuted: "#5C5C5C",
  accent: "#000000",
  primary: "#000000",
  onPrimary: "#FFFFFF",
  primaryContainer: "#FFFFFF",
  onPrimaryContainer: "#000000",
  danger: "#FF5A5A",
  success: "#000000",
  warning: "#000000",
  focusRing: "#000000",
  scrim: "#00000080",
  outline: "#000000",
  outlineVariant: "#000000",
  transparent: "#00000000",
};

export const HIGH_DARK: Palette = {
  bg: "#000000",
  background: "#000000",
  onBackground: "#FFFFFF",
  surface: "#000000",
  onSurface: "#FFFFFF",
  surfaceContainer: "#000000",
  onSurfaceContainer: "#FFFFFF",
  text: "#FFFFFF",
  textMuted: "#A3A3A3",
  accent: "#FFFFFF",
  primary: "#FFFFFF",
  onPrimary: "#000000",
  primaryContainer: "#000000",
  onPrimaryContainer: "#FFFFFF",
  danger: "#FF5A5A",
  success: "#FFFFFF",
  warning: "#FFFFFF",
  focusRing: "#FFFFFF",
  scrim: "#00000080",
  outline: "#FFFFFF",
  outlineVariant: "#FFFFFF",
  transparent: "#00000000",
};
