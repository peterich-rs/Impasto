import assert from "node:assert/strict";
import { createComponent, createRoot, flush } from "solid-js";
import { Errored } from "solid-js";
import type { SceneModule } from "../../solid-renderer/src/scene.ts";
import { testScene } from "../../solid-renderer/testing/scene.ts";
import { createSceneRenderer } from "../../solid-renderer/testing/render.ts";
import { installMockHost } from "../testing/renderTest.ts";
import { bindFocusRing } from "./focus.ts";
import {
  installThemePort,
  resetThemeForTests,
  type PlatformSnapshot,
} from "./theme.ts";
import { useTheme } from "./use-theme.ts";

const mock = installMockHost();
const options = testScene(mock.host as unknown as SceneModule);
const { render } = createSceneRenderer(options);

let sawFallback = false;
createRoot(() => {
  const parent = options.createElement("view");
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
  mock.calls.some(
    (call) => call.name === "text" && call.args[0] === "fallback",
  ),
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
  const parent = options.createElement("view");
  bindFocusRing(
    () => parent,
    () => theme().brightness === "dark",
  );
  render(() => options.createTextNode(""), parent);
});
flush();
mock.calls.length = 0;
apply?.({ ...light, brightness: "dark" });
flush();
const names = [...new Set(mock.calls.map((call) => call.name))];
assert.equal(names.includes("setText"), false);
assert.equal(
  mock.calls.some((call) => {
    const style = call.args[2] as { outlineWidth?: number } | undefined;
    return (
      call.name === "setStyleSource" &&
      call.args[1] === "component" &&
      style?.outlineWidth === 2
    );
  }),
  true,
  names.join(","),
);
resetThemeForTests();

console.log("boundary.test.ts ok");
