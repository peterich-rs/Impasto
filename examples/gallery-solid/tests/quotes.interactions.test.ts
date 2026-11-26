import { assert, fire, screen, test } from "impasto:test";
import "../src/quotes.tsx";

async function shown(...labels: string[]) {
  const visible = new Set((await screen.nodes()).map(node => node.label));
  for (const label of labels) assert.ok(visible.has(label), `Missing ${label}`);
}

async function click(label: string) {
  const matches = await screen.nodes("button", label);
  const nodes = matches.length ? matches : await screen.nodes("", label);
  assert.ok(nodes.length, `No accessible node for ${label}`);
  await fire.click(nodes[0].id);
  await screen.advanceFrames(24);
}

test("navigates the live markets, stock details and account tabs", async () => {
  await screen.viewport(430, 932);
  await screen.advanceFrames(8);
  await shown("综合指数", "全部", "上涨", "下跌", "活跃", "腾讯控股");
  await click("综合指数");
  await shown("科技", "消费", "工业");
  await click("综合指数");
  assert.equal((await screen.nodes("text", "科技")).length, 0);

  await click("上涨");
  await click("下跌");
  await click("活跃");
  await click("全部");
  await click("腾讯控股");
  await screen.advanceFrames(100);
  await shown("分时", "盘口", "买入", "卖出");

  await click("5m");
  await shown("MA5", "MA10", "MA20");
  await click("分时");
  await shown("均价");

  await click("‹ 返回");
  await screen.advanceFrames(80);
  await shown("综合指数");
  await click("⋯");
  await click("市场");
  await shown("涨幅榜");
  await click("交易");
  await shown("持仓", "待成交", "已成交");
  await click("我的");
  await shown("涨跌配色", "行情速度", "语言");
});
