import assert from "node:assert/strict";
import { installMockHost } from "../../packages/solid-ui/testing/renderTest.ts";

const mock = installMockHost();

await import(new URL("./dist/quotes.js", import.meta.url).href);

function fail(message: string): never {
  console.error(message);
  console.error(mock.liveTexts().join(", "));
  process.exit(1);
}

const initial = mock.liveTexts();
for (const label of ["自选", "市场", "交易", "我的"]) {
  if (!initial.includes(label)) {
    fail(`missing tab ${label}`);
  }
}
if (!initial.includes("恒生指数")) {
  fail("missing index strip");
}
if (!initial.some((t) => t === "恒生指数" || t === "腾讯控股" || t === "Apple")) {
  fail("missing watchlist row name");
}

// Binder wrote real prices on bind (write-on-bind): price cells show numbers.
const priced = initial.filter((t) => /^\d{1,3}(,\d{3})*(\.\d{2})$/.test(t));
if (priced.length < 4) {
  fail(`expected formatted prices, got ${priced.join(",")}`);
}

// Market tab renders a board
mock.clickByLabel("市场");
mock.flushHostFrame(16);
const market = mock.liveTexts();
if (!market.includes("涨幅榜") || !market.some((t) => t.includes("榜"))) {
  fail(`market page: ${market.join(",")}`);
}

// Trade tab renders the equity card
mock.clickByLabel("交易");
mock.flushHostFrame(32);
const trade = mock.liveTexts();
if (!trade.includes("总资产 (模拟)") || !trade.includes("持仓")) {
  fail(`trade page: ${trade.join(",")}`);
}

// Profile tab renders settings
mock.clickByLabel("我的");
mock.flushHostFrame(48);
const profile = mock.liveTexts();
if (!profile.includes("涨跌配色") || !profile.includes("行情速度")) {
  fail(`profile page: ${profile.join(",")}`);
}

// Back to watchlist, stock detail pushes
mock.clickByLabel("自选");
mock.flushHostFrame(64);
mock.clickByLabel("恒生指数");
mock.flushHostFrame(80);
const stock = mock.liveTexts();
if (!stock.some((t) => t.includes("返回")) || !stock.includes("盘口 五档")) {
  fail(`stock page: ${stock.join(",")}`);
}
if (!stock.includes("买入") || !stock.includes("卖出")) {
  fail(`stock trade buttons: ${stock.join(",")}`);
}

console.log("quotes app render ok");
