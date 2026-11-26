import { enabled as hasDevtools } from "@impasto/devtools/native";
import { boot, scene } from "@impasto/solid-renderer";
import { getInitialURL } from "@impasto/linking";
import {
  installThemePort,
  OverlayRoot,
  type PlatformSnapshot,
} from "@impasto/solid-ui";
import { wireQuotes } from "./state/quotes-provider.tsx";
import { createAccount } from "./state/account.ts";
import { setAccount } from "./state/account-ref.ts";
import { loadSettings } from "./state/settings.ts";
import { loadWatchlist } from "./state/watchlist.ts";
import { App, initialPath } from "./app.tsx";
type SettingsHost = {
  settings(): PlatformSnapshot;
  onSettings(fn: (snap: PlatformSnapshot) => void): number;
  offSettings(id: number): void;
};

// The app paints a dark surface. System light mode must not turn Field and
// Dialog into white controls on top of it.
const settingsHost = scene as unknown as SettingsHost;
const dark = (snap: PlatformSnapshot): PlatformSnapshot => ({
  ...snap,
  brightness: "dark",
});
installThemePort({
  settings: () => dark(settingsHost.settings()),
  onSettings: (fn) => settingsHost.onSettings((snap) => fn(dark(snap))),
  offSettings: (id) => settingsHost.offSettings(id),
});
void loadSettings();
void loadWatchlist();
const { engine } = wireQuotes();
setAccount(createAccount(engine));
const bootGallery = () => boot(() => (
  <OverlayRoot>
    <App initial={initialPath(getInitialURL())} />
  </OverlayRoot>
));
if (hasDevtools()) {
  void import("@impasto/devtools").then(({ installDevtools }) => {
    installDevtools({ launcher: false });
  }).catch(error => { console.error(error); }).then(bootGallery);
} else {
  bootGallery();
}
