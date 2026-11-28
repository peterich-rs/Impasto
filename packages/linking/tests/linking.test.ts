import { assert, test } from "impasto:test";
import { Linking, mailtoUrl, telUrl, watchUrls } from "../src/index";
import { mockLinking } from "../src/testing";

test("deep link consumers retain both listener and stream APIs", async () => {
  const mock = mockLinking({ initialUrl: "demo://initial" });
  try {
    const reader = watchUrls().getReader();
    let event = "";
    const listener = ({ url }: { url: string }) => { event = url; };
    Linking.addEventListener("url", listener);
    mock.emitUrl("demo://runtime");
    assert.equal((await reader.read()).value, event);
    assert.equal(Linking.getInitialURL(), "demo://initial");
    Linking.removeEventListener("url", listener);
    await Linking.openURL("https://example.com", { mode: "inAppBrowser" });
    assert.equal(mock.openedUrls[0]?.options?.mode, "inAppBrowser");
    await reader.cancel();
  } finally { mock.restore(); }
});

test("mailto and telephone helpers escape URI delimiters", () => {
  assert.equal(mailtoUrl("contact@example.com", { subject: "A & B" }), "mailto:contact@example.com?subject=A%20%26%20B");
  assert.equal(telUrl("+1 (555) 123-4567"), "tel:+15551234567");
});
