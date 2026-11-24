import { installDevtools } from "@impasto/devtools";
import { getInitialURL } from "@impasto/linking";
import { boot, scene } from "@impasto/solid-renderer";
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
import { loadSearchHistory } from "./state/search-history.ts";
import { App } from "./app.tsx";
import { COMPOSITOR_LAB_URL, CompositorLabPage } from "./pages/compositor-lab.tsx";
import { GPU_LAB_URL, GpuLabPage } from "./pages/gpu-lab.tsx";
type SettingsHost = {
  settings(): PlatformSnapshot;
  onSettings(fn: (snap: PlatformSnapshot) => void): number;
  offSettings(id: number): void;
};

// Quotes paints a light surface. System dark mode must not turn Field and
// Dialog into black controls on that page.
const settingsHost = scene as unknown as SettingsHost;
const light = (snap: PlatformSnapshot): PlatformSnapshot => ({
  ...snap,
  brightness: "light",
});
installThemePort({
  settings: () => light(settingsHost.settings()),
  onSettings: (fn) => settingsHost.onSettings((snap) => fn(light(snap))),
  offSettings: (id) => settingsHost.offSettings(id),
});
const launchUrl = getInitialURL();
if (launchUrl === GPU_LAB_URL || launchUrl === `${GPU_LAB_URL}/invalid`) {
  boot(() => GpuLabPage({ invalidOnLaunch: launchUrl.endsWith("/invalid") }));
} else if (launchUrl === COMPOSITOR_LAB_URL || launchUrl?.startsWith(`${COMPOSITOR_LAB_URL}/`)) {
  boot(() => CompositorLabPage({ launchUrl }));
} else {
  void loadSettings();
  void loadWatchlist();
  void loadSearchHistory();
  const { engine } = wireQuotes();
  setAccount(createAccount(engine));
  installDevtools({ launcher: false });
  boot(() => (
    <OverlayRoot>
      <App />
    </OverlayRoot>
  ));
}
