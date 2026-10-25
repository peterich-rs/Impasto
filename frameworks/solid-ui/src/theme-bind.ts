import { host } from "@uifw/solid-renderer";
import { installThemePort, type PlatformSnapshot } from "./theme.ts";

type SettingsHost = {
  settings(): PlatformSnapshot;
  onSettings(fn: (s: PlatformSnapshot) => void): number;
  offSettings(id: number): void;
};

const settingsHost = host as unknown as SettingsHost;

installThemePort({
  settings: () => settingsHost.settings(),
  onSettings: (fn) => settingsHost.onSettings(fn),
  offSettings: (id) => settingsHost.offSettings(id),
});
