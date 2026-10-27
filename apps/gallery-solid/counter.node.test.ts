import assert from "node:assert/strict";
import { installMockHost } from "../../frameworks/solid-ui/testing/renderTest.ts";

const mock = installMockHost();

await import(new URL("./dist/counter.js", import.meta.url).href);

function fail(message: string): never {
  console.error(message);
  console.error(mock.liveTexts().join(", "));
  process.exit(1);
}

const initial = mock.liveTexts();
for (const label of ["调度", "导航", "列表", "微任务改字", "还是旧的"]) {
  if (!initial.includes(label)) {
    fail(`missing ${label}`);
  }
}
if (initial.includes("第 1 页")) {
  fail("nav page is mounted on the schedule route");
}
if (mock.calls.filter((call) => call.name === "onClick").length < 4) {
  fail("expected a click handler on each tab and the schedule action");
}
if (!mock.calls.some((call) => call.name === "onFrame" && call.args[0] === "function")) {
  fail("expected onFrame");
}
if (!mock.calls.some((call) => call.name === "setRadius" && call.args[1] === 999)) {
  fail("expected a pill radius on buttons");
}

mock.clickByLabel("微任务改字");
await new Promise((resolve) => queueMicrotask(resolve));
mock.flushHostFrame();
if (!mock.liveTexts().includes("已经改了") || mock.liveTexts().includes("还是旧的")) {
  fail(`microtask label: ${mock.liveTexts().join(", ")}`);
}

mock.clickByLabel("导航");
mock.flushHostFrame();
const nav = mock.liveTexts();
if (!nav.includes("第 1 页") || nav.includes("还是旧的") || nav.includes("已经改了")) {
  fail(`nav route: ${nav.join(", ")}`);
}

mock.clickByLabel("推进");
mock.flushHostFrame(0);
if (!mock.liveTexts().includes("第 2 页") || !mock.liveTexts().includes("第 1 页")) {
  fail(`push keeps the previous page until the spring settles: ${mock.liveTexts().join(", ")}`);
}
mock.flushHostFrame(400);
if (!mock.liveTexts().includes("第 2 页")) {
  fail("nested push did not stay mounted");
}

mock.clickByLabel("弹出");
const beforeSettle = mock.liveTexts();
if (!beforeSettle.includes("第 2 页")) {
  fail("pop removed the page on the same turn");
}
mock.flushHostFrame();
if (!mock.liveTexts().includes("第 2 页")) {
  fail("exit spring must keep the page on the pop frame");
}
mock.flushHostFrame();
if (mock.liveTexts().includes("第 2 页") || !mock.liveTexts().includes("第 1 页")) {
  fail(`after settle: ${mock.liveTexts().join(", ")}`);
}

mock.clickByLabel("推进");
mock.flushHostFrame();
mock.flushHostFrame();
if (!mock.liveTexts().includes("第 2 页")) {
  fail("could not reopen a page for the dialog");
}
mock.clickByLabel("对话框");
mock.flushHostFrame();
mock.flushHostFrame();
if (!mock.liveTexts().includes("返回键关闭")) {
  fail(`dialog: ${mock.liveTexts().join(", ")}`);
}
if (!mock.pressBack()) {
  fail("back did not close the dialog");
}
if (!mock.liveTexts().includes("第 2 页")) {
  fail("back popped the page instead of closing the dialog");
}
mock.flushHostFrame();
if (!mock.pressBack()) {
  fail(`back did not pop after the dialog closed: ${mock.liveTexts().join(", ")}`);
}
mock.flushHostFrame();
if (!mock.liveTexts().includes("第 2 页")) {
  fail("pop removed the page on the back frame");
}
mock.flushHostFrame();
if (mock.liveTexts().includes("第 2 页") || !mock.liveTexts().includes("第 1 页")) {
  fail(`after back pop: ${mock.liveTexts().join(", ")}`);
}

mock.clickByLabel("列表");
mock.flushHostFrame();
const list = mock.liveTexts();
if (!list.some((label) => label.startsWith("Item ")) || list.includes("第 1 页")) {
  fail(`list route: ${list.join(", ")}`);
}
if (!mock.calls.some((call) => call.name === "scroll")) {
  fail("expected a scroll viewport");
}

console.log("gallery-solid counter render ok");
