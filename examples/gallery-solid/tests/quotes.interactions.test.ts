import { assert, fire, screen, test } from "impasto:test";
import "../src/quotes.tsx";

async function shown(...labels: string[]) {
  const visible = new Set((await screen.nodes()).map(node => node.label));
  for (const label of labels) assert.ok(visible.has(label), `Missing ${label}`);
}

async function absent(label: string) {
  assert.equal((await screen.nodes("", label)).length, 0, `Unexpected ${label}`);
}

async function click(label: string) {
  const matches = await screen.nodes("button", label);
  const nodes = matches.length ? matches : await screen.nodes("", label);
  assert.ok(nodes.length, `No accessible node for ${label}`);
  await fire.click(nodes[0].id);
  await screen.advanceFrames(24);
}

/** Buttons whose label carries data (position rows) are found by prefix. */
async function buttonStartingWith(prefix: string) {
  const nodes = (await screen.nodes("button")).filter(node => node.label.startsWith(prefix));
  assert.ok(nodes.length, `No button starting with ${prefix}`);
  return nodes[0].id;
}

test("watchlist opens details and returns without leaving a detail behind", async () => {
  await screen.viewport(430, 932);
  await screen.advanceFrames(8);
  await shown("综合指数", "全部", "上涨", "下跌", "活跃", "腾讯控股", "自选", "账户", "设置");

  await click("综合指数");
  await shown("科技", "消费", "工业");
  await click("综合指数");
  await absent("科技");

  for (const filter of ["上涨", "下跌", "活跃", "全部"]) await click(filter);
  await click("腾讯控股");
  await screen.advanceFrames(100);
  await shown("分时", "盘口", "买入", "卖出");
  await absent("账户");

  await click("返回");
  await screen.advanceFrames(80);
  await shown("综合指数");
});

test("tabs switch the root page and hide the capsule on pushed pages", async () => {
  await screen.viewport(430, 932);
  await screen.advanceFrames(8);
  await click("账户");
  await shown("总资产", "可用资金", "持仓市值", "累计盈亏", "成本价", "仓位", "今日盈亏", "00700");
  await absent("综合指数");
  await click("设置");
  await shown("涨跌配色", "行情速度", "语言", "重置模拟账户");
  await absent("总资产");
  await click("自选");
  await shown("综合指数");
  await click("搜索");
  await shown("热门");
  await absent("自选");
  await click("返回");
  await screen.advanceFrames(80);
  await shown("综合指数", "自选");
});

test("a position row survives market updates and opens the matching detail", async () => {
  await screen.viewport(430, 932);
  await screen.advanceFrames(8);
  await click("账户");
  const row = await buttonStartingWith("NVDA");
  await screen.advanceFrames(60);
  assert.equal(await buttonStartingWith("NVDA"), row);
  await fire.click(row);
  await screen.advanceFrames(24);
  await screen.advanceFrames(60);
  await shown("返回");
  await click("返回");
  await screen.advanceFrames(80);
  await shown("总资产");
});
