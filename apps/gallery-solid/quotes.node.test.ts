import assert from "node:assert/strict";
import { installMockHost } from "../../packages/solid-ui/testing/renderTest.ts";

const mock = installMockHost();
await import(new URL("./dist/quotes.js", import.meta.url).href);

let time = 0;
function frame(): void {
  mock.flushHostFrame(time += 250);
}
function click(label: string): void {
  try {
    mock.clickByLabel(label);
    frame();
    frame();
  } catch (cause) {
    throw new Error(`Click ${label} failed`, { cause });
  }
}
function shown(...labels: string[]): void {
  const texts = mock.liveTexts();
  for (const label of labels) assert.ok(texts.includes(label), `missing ${label}: ${texts.join(", ")}`);
}

frame();
shown("综合指数", "全部", "上涨", "下跌", "活跃", "腾讯控股");
assert.ok(mock.calls.some((call) => call.name === "bindCanvas"), "visible rows bind a native Canvas");
const listDrivers = () => mock.calls.filter((call) => call.name === "listDriver").length;
const initialDrivers = listDrivers();

click("综合指数");
shown("科技", "消费", "工业");
click("综合指数");
assert.ok(!mock.liveTexts().includes("科技"));

click("上涨");
click("下跌");
click("活跃");
click("全部");

click("Ⅱ");
shown("▶");
const paused = mock.liveTexts().find((text) => /^\d\d:\d\d:\d\d$/.test(text));
assert.ok(paused);
for (let index = 0; index < 10; index++) frame();
shown(paused);
click("▶");
shown("Ⅱ");

click("腾讯控股");
shown("‹ 返回", "买入", "卖出");
for (let index = 0; index < 8; index++) frame();
shown("‹ 返回", "分时", "盘口", "逐笔成交", "深度", "成交方向", "价格区间", "买入", "卖出");
click("★");
shown("☆");
click("☆");
shown("★");
click("5m");
shown("MA5", "MA10", "MA20");
click("分时");
shown("均价");
click("5 → 10");
shown("A10", "B10", "10 → 5");
click("10 → 5");
assert.ok(!mock.liveTexts().includes("A10"));

click("A1");
for (let index = 0; index < 5; index++) frame();
shown("买入 00700", "模拟下单", "复核订单");
click("市价");
click("100");
click("复核订单");
shown("修改", "确认订单");
click("修改");
shown("限价", "市价");
click("复核订单");
click("确认订单");
assert.ok(mock.liveTexts().some((text) => text.includes("已提交模拟订单")), mock.liveTexts().join(", "));

click("‹ 返回");
shown("综合指数");
assert.equal(listDrivers(), initialDrivers, "cover and exit phases preserve the original list driver");
click("⋯");
click("市场");
shown("涨幅榜");
click("交易");
shown("总资产 (模拟)", "持仓", "待成交", "已成交");
click("我的");
shown("涨跌配色", "行情速度", "语言", "重置模拟账户", "账户");
click("自选");
shown("恒生指数", "搜索");
click("搜索");
shown("热门", "00700.HK");
mock.pressBack();
frame();
frame();
click("行情");
shown("综合指数");

console.log("stocks app interactions ok");
