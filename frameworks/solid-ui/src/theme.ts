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

export type Theme = {
  locale: string;
  density: Density;
  fontSize: { body: number; title: number };
  weight: { body: number; emphasis: number };
  space: { s: number; m: number; l: number };
  radius: { s: number; m: number };
  color: {
    bg: string;
    surface: string;
    text: string;
    textMuted: string;
    accent: string;
    danger: string;
    scrim: string;
  };
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

function palette(
  brightness: "light" | "dark",
  highContrast: boolean,
): Theme["color"] {
  if (highContrast && brightness === "dark") {
    return {
      bg: "#000000",
      surface: "#000000",
      text: "#FFFFFF",
      textMuted: "#A3A3A3",
      accent: "#FFFFFF",
      danger: "#FF5A5A",
      scrim: "#00000080",
    };
  }
  if (highContrast && brightness === "light") {
    return {
      bg: "#FFFFFF",
      surface: "#FFFFFF",
      text: "#000000",
      textMuted: "#5C5C5C",
      accent: "#000000",
      danger: "#FF5A5A",
      scrim: "#00000080",
    };
  }
  if (brightness === "dark") {
    return {
      bg: "#111111",
      surface: "#282828",
      text: "#FFFFFF",
      textMuted: "#A3A3A3",
      accent: "#7CFF6B",
      danger: "#FF5A5A",
      scrim: "#00000080",
    };
  }
  return {
    bg: "#FFFFFF",
    surface: "#F2F2F2",
    text: "#111111",
    textMuted: "#5C5C5C",
    accent: "#148A32",
    danger: "#FF5A5A",
    scrim: "#00000080",
  };
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

export function createTheme(): () => Theme {
  ensureListening();
  return () => resolveTheme(snapshot, density);
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
