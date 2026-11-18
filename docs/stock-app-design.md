# 证券模拟盘

`just host` 打开 `apps/gallery-solid/dist/quotes.js`。入口为 `/stocks`：综合指数可展开涨跌家数和行业表现，列表有全部、上涨、下跌、活跃四个筛选。暂停键同时控制列表和个股的模拟时钟。列表使用 `VirtualList`；每行的走势使用 `Canvas`。

`pages/stocks.tsx` 按场景宽度组合页面。小于 800 点时，点证券进入 `/stock/:symbol`，返回保留列表筛选和位置。800 点起为列表和详情两栏；1240 点起，盘口和逐笔成交移到右栏，详情原地切换证券。`pages/stock.tsx` 共用行情摘要、图表、盘口、深度、成交方向和价格区间。详情星标可加入或移出自选，买卖栏固定在底部。

- `widgets/stocks/chart.tsx` 用 Canvas 画分时、1m、5m、15m、1D、成交量和均线。主图节点与选中状态独立。十字线、圆点和提示框通过覆盖层的变换、透明度和文本更新。按下、拖动和结束用 `CanvasController.toLocal` 取坐标，方向键和前后选点按钮更新同一份选中时间。横向拖动留在图表上，纵向拖动交给页面滚动。
- `widgets/stocks/order-book.tsx` 显示五档或十档盘口；点击卖盘档位打开买入单，点击买盘打开卖出单，初始限价取档位价。深度图使用同一份十二档数据。
- `engine/analytics.ts` 从模拟成交累计分钟 OHLC、成交量、均价、成交额和分档买卖量。逐笔从 `TickRing` 读取；盘口是围绕最新价生成的模拟档位，与挂单成交分别计算。
- `pages/order.tsx` 在窄屏使用 `Sheet`，宽屏使用 `Dialog`。订单支持市价与限价、整手数量、费用预估和复核确认；状态、可用现金、持仓和订单保存在 `state/account.ts`。

自选、旧市场榜单、交易记录、设置和搜索从行情菜单进入。证券和订单都使用应用内模拟器；这些数据不会送到证券交易所。

`pnpm --filter @impasto/gallery-solid test` 验证行情筛选、盘口切档、模拟下单与原有页面。构建 Gallery 后，`cargo test -p quickjs stocks_ -- --test-threads=1` 验证原生会话的窄屏和宽屏交互。`IMPASTO_DUMP` 可从 macOS `Engine` 的首帧导出 PNG。
