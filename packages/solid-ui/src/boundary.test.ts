import assert from "node:assert/strict";
import { createComponent, createRoot, flush } from "solid-js";
import { Errored } from "solid-js";
import { createRenderer } from "@solidjs/universal";
import type { CoreHost } from "../../solid-renderer/src/host.ts";
import { hostRendererOptions } from "../../solid-renderer/src/tree.ts";
import { installMockHost } from "../testing/renderTest.ts";
import { bindFocusRing } from "./focus.ts";
import { installThemePort, resetThemeForTests, type PlatformSnapshot } from "./theme.ts";
import { useTheme } from "./use-theme.ts";

const mock = installMockHost();
const options = hostRendererOptions(mock.host as unknown as CoreHost);
const { render } = createRenderer(options);

let sawFallback = false;
createRoot(() => {
  const parent = options.createElement("column");
  render(
    () =>
      createComponent(Errored, {
        fallback: () => {
          sawFallback = true;
          return options.createTextNode("fallback");
        },
        get children() {
          throw new Error("boom");
        },
      }),
    parent,
  );
});
assert.equal(sawFallback, true);
assert.equal(
  mock.calls.some((call) => call.name === "text" && call.args[0] === "fallback"),
  true,
);

resetThemeForTests();
let apply: ((settings: PlatformSnapshot) => void) | null = null;
const light: PlatformSnapshot = {
  locale: "en",
  textScale: 1,
  brightness: "light",
  reduceMotion: false,
  boldText: false,
  highContrast: false,
};
installThemePort({
  settings: () => light,
  onSettings: (fn) => {
    apply = fn;
    return 1;
  },
  offSettings: () => {},
});

createRoot(() => {
  const theme = useTheme();
  const parent = options.createElement("column");
  bindFocusRing(
    () => parent.id,
    () => theme().brightness === "dark",
  );
  render(() => options.createTextNode(""), parent);
});
flush();
mock.calls.length = 0;
apply?.({ ...light, brightness: "dark" });
flush();
const names = [...new Set(mock.calls.map((call) => call.name))];
for (const name of names) {
  assert.ok(name === "setColor" || name === "setStroke" || name === "setAlpha", names.join(","));
}
assert.equal(names.includes("setText"), false);
assert.equal(names.includes("setStroke"), true, names.join(","));
resetThemeForTests();

console.log("boundary.test.ts ok");
