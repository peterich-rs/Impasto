import { boot } from "@impasto/solid-renderer";
import { OverlayRoot } from "@impasto/solid-ui";
import { wireQuotes } from "./state/quotes-provider.tsx";
import { createAccount } from "./state/account.ts";
import { setAccount } from "./state/account-ref.ts";
import { App } from "./app.tsx";

const { engine } = wireQuotes();
setAccount(createAccount(engine));

boot(() => (
  <OverlayRoot>
    <App />
  </OverlayRoot>
));
