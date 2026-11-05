import { installDevtools } from "@impasto/devtools";
import { boot, host } from "@impasto/solid-renderer";
import { installThemePort, OverlayRoot, type PlatformSnapshot } from "@impasto/solid-ui";
import { wireQuotes } from "./state/quotes-provider.tsx";
import { createAccount } from "./state/account.ts";
import { setAccount } from "./state/account-ref.ts";
import { loadSettings } from "./state/settings.ts";
import { loadWatchlist } from "./state/watchlist.ts";
import { loadSearchHistory } from "./state/search-history.ts";
import { App } from "./app.tsx";

void loadSettings();
void loadWatchlist();
void loadSearchHistory();

type SettingsHost = {
  settings(): PlatformSnapshot;
  onSettings(fn: (snap: PlatformSnapshot) => void): number;
  offSettings(id: number): void;
};

// Quotes paints a light surface. System dark mode must not turn Field and
// Dialog into black controls on that page.
const settingsHost = host as unknown as SettingsHost;
const light = (snap: PlatformSnapshot): PlatformSnapshot => ({ ...snap, brightness: "light" });
installThemePort({
  settings: () => light(settingsHost.settings()),
  onSettings: (fn) => settingsHost.onSettings((snap) => fn(light(snap))),
  offSettings: (id) => settingsHost.offSettings(id),
});

const { engine } = wireQuotes();
setAccount(createAccount(engine));

// The panel stays out of the app's JSX and mounts through registerTopLayer.
installDevtools();

boot(() => (
  <OverlayRoot>
    <App />
  </OverlayRoot>
));
