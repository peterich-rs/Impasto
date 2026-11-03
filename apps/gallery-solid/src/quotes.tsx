import { installDevtools } from "@impasto/devtools";
import { boot } from "@impasto/solid-renderer";
import { OverlayRoot } from "@impasto/solid-ui";
import { wireQuotes } from "./state/quotes-provider.tsx";
import { createAccount } from "./state/account.ts";
import { setAccount } from "./state/account-ref.ts";
import { App } from "./app.tsx";

const { engine } = wireQuotes();
setAccount(createAccount(engine));

// The host no-ops this when IMPASTO_DEVTOOLS is off. The panel stays out of
// the app's JSX and mounts through registerTopLayer.
installDevtools();

boot(() => (
  <OverlayRoot>
    <App />
  </OverlayRoot>
));
