import {
  DARK,
  HIGH_DARK,
  HIGH_LIGHT,
  LIGHT,
  type Palette,
} from "@impasto/solid-renderer/palette";

export type PlatformSnapshot = {
  locale: string;
  textScale: number;
  brightness: "light" | "dark";
  reduceMotion: boolean;
  boldText: boolean;
  highContrast: boolean;
};

export type Density = "compact" | "regular" | "comfortable";
export type ButtonSize = "regular" | "compact";
export type ButtonVariant = "primary" | "quiet";

export type { Palette };

/** Six corner sizes. `full` is the pill radius. Spacing scales with density. */
export type Shape = {
  none: number;
  xs: number;
  s: number;
  m: number;
  l: number;
  full: number;
};

export type TypeStyle = { size: number; weight: number; lineHeight: number };

/** Six type roles. Sizes scale with density. Control chrome multiplies `textScale` at the call site. */
export type Typography = {
  display: TypeStyle;
  headline: TypeStyle;
  title: TypeStyle;
  body: TypeStyle;
  label: TypeStyle;
  caption: TypeStyle;
};

export type Theme = {
  locale: string;
  density: Density;
  fontSize: { body: number; title: number };
  weight: { body: number; emphasis: number };
  space: { s: number; m: number; l: number };
  radius: { s: number; m: number };
  shape: Shape;
  typography: Typography;
  color: Palette;
  reduceMotion: boolean;
  boldText: boolean;
  highContrast: boolean;
  brightness: "light" | "dark";
  textScale: number;
};

export type ThemePort = {
  settings(): PlatformSnapshot;
  onSettings(fn: (s: PlatformSnapshot) => void): number;
  offSettings(id: number): void;
};

const COEFF: Record<Density, number> = {
  compact: 0.9,
  regular: 1,
  comfortable: 1.1,
};

const DEFAULT_SNAPSHOT: PlatformSnapshot = {
  locale: "en",
  textScale: 1,
  brightness: "light",
  reduceMotion: false,
  boldText: false,
  highContrast: false,
};

let port: ThemePort | null = null;
let listening = false;
let listenId = 0;
let snapshot: PlatformSnapshot = DEFAULT_SNAPSHOT;
let density: Density = "regular";
const observers = new Set<() => void>();

function densityCoeff(value: Density): number {
  return COEFF[value];
}

function palette(brightness: "light" | "dark", highContrast: boolean): Palette {
  if (highContrast && brightness === "dark") {
    return HIGH_DARK;
  }
  if (highContrast && brightness === "light") {
    return HIGH_LIGHT;
  }
  if (brightness === "dark") {
    return DARK;
  }
  return LIGHT;
}

function typeStyle(size: number, weight: number): TypeStyle {
  return { size, weight, lineHeight: size * 1.25 };
}

export function resolveTheme(settings: PlatformSnapshot, nextDensity: Density): Theme {
  const coeff = densityCoeff(nextDensity);
  return {
    locale: settings.locale,
    density: nextDensity,
    fontSize: {
      body: 16 * coeff,
      title: 20 * coeff,
    },
    weight: settings.boldText
      ? { body: 600, emphasis: 700 }
      : { body: 400, emphasis: 600 },
    space: {
      s: 4 * coeff,
      m: 8 * coeff,
      l: 16 * coeff,
    },
    radius: {
      s: 4 * coeff,
      m: 6 * coeff,
    },
    shape: {
      none: 0,
      xs: 2 * coeff,
      s: 4 * coeff,
      m: 6 * coeff,
      l: 12 * coeff,
      full: 999,
    },
    typography: {
      display: typeStyle(32 * coeff, settings.boldText ? 700 : 600),
      headline: typeStyle(24 * coeff, settings.boldText ? 700 : 600),
      title: typeStyle(20 * coeff, settings.boldText ? 700 : 600),
      body: typeStyle(16 * coeff, settings.boldText ? 600 : 400),
      label: typeStyle(14 * coeff, settings.boldText ? 600 : 500),
      caption: typeStyle(12 * coeff, settings.boldText ? 600 : 400),
    },
    color: palette(settings.brightness, settings.highContrast),
    reduceMotion: settings.reduceMotion,
    boldText: settings.boldText,
    highContrast: settings.highContrast,
    brightness: settings.brightness,
    textScale: settings.textScale,
  };
}

/** Compact size on compact density is 32. Comfortable density is 48. Otherwise 44. */
export function minTarget(size: ButtonSize, nextDensity: Density): number {
  if (nextDensity === "comfortable") {
    return 48;
  }
  if (size === "compact" && nextDensity === "compact") {
    return 32;
  }
  return 44;
}

function notify(): void {
  for (const fn of observers) {
    fn();
  }
}

function applySnapshot(next: PlatformSnapshot): void {
  snapshot = next;
  notify();
}

export function installThemePort(next: ThemePort): void {
  port = next;
}

export function setDensity(next: Density): void {
  density = next;
  notify();
}

export function setDensityForTests(next: Density): void {
  setDensity(next);
}

export function onThemeChange(fn: () => void): () => void {
  observers.add(fn);
  return () => {
    observers.delete(fn);
  };
}

function ensureListening(): void {
  if (listening) {
    return;
  }
  if (port === null) {
    throw new Error("theme port is not installed");
  }
  snapshot = port.settings();
  listenId = port.onSettings(applySnapshot);
  listening = true;
}

export function currentLocale(): string {
  return snapshot.locale;
}

/**
 * Shallow-merges `overrides` onto the resolved theme.
 * A `color` override replaces the palette object; it does not merge keys.
 */
export function createTheme(overrides?: Partial<Theme>): () => Theme {
  ensureListening();
  return () => {
    const base = resolveTheme(snapshot, density);
    if (overrides === undefined) {
      return base;
    }
    return { ...base, ...overrides };
  };
}

export function resetThemeForTests(): void {
  if (listening && port !== null) {
    port.offSettings(listenId);
  }
  listening = false;
  listenId = 0;
  snapshot = DEFAULT_SNAPSHOT;
  density = "regular";
}
