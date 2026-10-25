import { createSignal, onCleanup } from "solid-js";
import { createTheme, onThemeChange, type Theme } from "./theme.ts";

/** Live theme. Settings and density changes rewrite the signal. */
export function useTheme(): () => Theme {
  const read = createTheme();
  const [theme, setTheme] = createSignal(read());
  onCleanup(onThemeChange(() => setTheme(read())));
  return theme;
}
