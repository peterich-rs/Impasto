import assert from "node:assert/strict";
import {
  createTheme,
  installThemePort,
  minTarget,
  resetThemeForTests,
  resolveTheme,
  type PlatformSnapshot,
} from "./theme.ts";

const light: PlatformSnapshot = {
  locale: "en",
  textScale: 1,
  brightness: "light",
  reduceMotion: false,
  boldText: false,
  highContrast: false,
};

const darkHighContrast: PlatformSnapshot = {
  ...light,
  brightness: "dark",
  highContrast: true,
};

const contrast = resolveTheme(darkHighContrast, "regular");
assert.equal(contrast.color.text, "#FFFFFF");
assert.equal(contrast.color.bg, "#000000");
assert.equal(contrast.color.accent, "#FFFFFF");
assert.equal(contrast.color.surface, "#000000");

const bold = resolveTheme({ ...light, boldText: true }, "regular");
assert.equal(bold.weight.body, 600);
assert.equal(bold.weight.emphasis, 700);

const scaled = resolveTheme({ ...light, textScale: 2 }, "regular");
assert.equal(scaled.fontSize.body, 16);

const comfortable = resolveTheme(light, "comfortable");
assert.equal(comfortable.fontSize.title, 22);
assert.equal(comfortable.space.m, 8 * 1.1);
assert.equal(resolveTheme(light, "compact").space.m, 8 * 0.9);
assert.equal(resolveTheme(light, "regular").space.s, 4);

assert.equal(minTarget("compact", "compact"), 32);
assert.equal(minTarget("compact", "comfortable"), 48);
assert.equal(minTarget("regular", "regular"), 44);
assert.equal(scaled.typography.body.size, resolveTheme(light, "regular").typography.body.size);

resetThemeForTests();
let settingsCalls = 0;
let listenCount = 0;
let offCount = 0;
installThemePort({
  settings: () => {
    settingsCalls += 1;
    return light;
  },
  onSettings: (_fn) => {
    listenCount += 1;
    return 7;
  },
  offSettings: (_id) => {
    offCount += 1;
  },
});
createTheme();
createTheme();
assert.equal(listenCount, 1);
assert.equal(settingsCalls, 1);

resetThemeForTests();
assert.equal(offCount, 1);
createTheme();
assert.equal(listenCount, 2);

resetThemeForTests();
installThemePort({
  settings: () => light,
  onSettings: () => 1,
  offSettings: () => {},
});
const merged = createTheme({ locale: "fr" });
assert.equal(merged().locale, "fr");
assert.equal(merged().color.text, "#14201A");
assert.equal(merged().space.m, 8);
const replaced = createTheme({ color: { ...merged().color, text: "#abcdef" } });
assert.equal(replaced().color.text, "#abcdef");
assert.equal(replaced().fontSize.body, 16);
resetThemeForTests();
console.log("theme.test.ts ok");
