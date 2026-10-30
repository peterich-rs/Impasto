# Quotes 模拟盘应用

`apps/gallery-solid` 的下一个形态：数据全 mock 的模拟盘证券应用，覆盖自选、市场、个股详情（分时与日K）、模拟交易。本文是设计文档；实现以源码和测试为准，未落地的部分以 `docs/refactor-plan.md` 为准。

## 定位与边界

应用是框架的压力验收载体，两条压力线是它的存在理由：

- **高频更新**：全市场 tick 只进 ring buffer，UI 每个 vsync 采样一次，只写可见行。不可见的数据静默累积。
- **无 Path/Canvas 的高密度自绘**：K线、分时、sparkline、盘口全部由 `decorated` 矩形 + `Text` 拼装，横向窗口化控制节点数，节点预算进测试断言。

不做：真实行情源、真实账户、弧线/圆环/路径图、位图图标（logo 以外的资产不新增）、逐帧位图图表、Kotlin 侧。

## 页面与路由

```
boot(App) → OverlayRoot → Screen(theme.color.background)
├─ TabBar: 自选 / 市场 / 交易 / 我的          ← router.reset 切换
└─ RouterView（沿用 StackTransition）
```

| 路由 | 页面 | 内容 |
|---|---|---|
| `/watchlist` | 自选 | 指数横条 + 高频价格列表 + sparkline + 分组 sticky 头 + 下拉刷新 |
| `/market` | 市场 | 三市场 Tabs、指数条、涨跌幅/热度榜（VirtualList 复用） |
| `/search` | 搜索 | Field 防抖过滤 + 热门 + 历史（async-storage） |
| `/stock/:symbol` | 个股 | 大数字报价、分时/日K 切换、盘口五档、下单入口 |
| `/order/:symbol` | 下单 | Sheet：买卖、限价/市价、盘口点价、仓位快捷条 |
| `/trade` | 交易 | 总资产卡片、持仓（实时盈亏）、委托记录三态 Tabs |
| `/profile` | 我的 | 账户、设置（涨跌配色、主题、语言、时间压缩）、关于 |

路由栈推入/弹出的验收职责由 `/stock/:symbol` 与 `/order/:symbol` 承接，原 `/nav/:depth` 退役。

## Mock 行情引擎

全部在 JS 侧，确定性可复现，测试可用 1:1 时钟。

- **随机源**：`mulberry32(hash(symbol))` 种子，Box-Muller 高斯。同一种子序列生成参数、历史、tick。
- **宇宙**：三市场约 48 只（00700、AAPL、600519 一类真实感命名），静态表给出基准价、年化波动（20%–60%）、品牌色（monogram 用）。
- **历史**：每股 250 根日K（种子 GBM 生成）+ 当日分时从开盘价起。
- **价格过程**：`S(t+dt) = S(t)·exp((μ−σ²/2)dt + σ√dt·Z)`；A股 ±10% 涨跌停钳制，港美股按日内 ±3σ 软钳制。
- **时间压缩**：缺省 `TIME_SCALE = 60`（1 现实秒 = 1 模拟分钟，一个交易日约 4 分钟），可注入。模拟时钟跨收盘触发结算：当日K 入历史、分时重置、下一交易日开盘。
- **tick 调度**：不设 per-symbol timer。引擎由根 provider 的 `onFrame` 步进：推进模拟时钟，每股 `nextTickAt`（100–500ms 模拟间隔随机）到期才生成 tick，写入每股 ring buffer（保留最近 240 个）。
- **步进门控**：需要行情的页面（自选、市场、个股、交易）挂载时登记，登记数大于 0 才步进；设置/关于等静态页全停。`Lifecycle` 暂停随会话冻结。

数据形状：

```ts
type Tick = { t: number; price: number; volume: number };
type Quote = {
  symbol: string; last: number; prevClose: number; open: number;
  high: number; low: number; volume: number;
  revision: number;            // tick 后自增，写入层用它跳过未变行
};
```

## 交易

- 初始模拟资金 1,000,000。市价单按下一 tick 成交；限价单进撮合队列，引擎每次步进检查穿越，大单模拟分两笔部分成交。
- 下单校验资金/持仓/涨跌停区间，失败 Toast。持仓 `{ symbol, qty, avgCost }`，盈亏走同一条节流写入管线。
- 待成交/已成交/已撤三态委托列表，限价单可撤；平仓走确认 Dialog + Toast。
- 持仓、委托、自选、设置存 `@impasto/async-storage`；行情历史纯生成不落盘。存储未链接时降级内存，两条路径都有测试。

## 高频渲染管线

三层，热路径绕开 Solid：

```
tick 层    引擎 onFrame 步进 → ring buffer（无信号参与）
节流层     每 vsync 采样最新值，revision 未变的行直接跳过
写入层     只写「已挂载行」注册的节点：host.setText / setColor / setScaleXY
```

- **行自注册**：每行组件挂载时把 `symbol → { priceNode, pctNode, flashNode, spark }` 注册进 `QuoteBinder`，卸载注销；回收复用（VirtualList `recycle`）时 render 效果重跑，先注销旧 symbol 再注册新的。注册时同步写一次当前值（写于绑定，write-on-bind），消灭回收行的陈旧帧。
- **可见性即挂载集**：窗口 + overscan + 池决定挂载行，滚出即注销，滚回对齐最新值。断言：不可见行零写入。
- **布局稳定**：价格/涨跌列固定宽容器右对齐。启动时用 `measureText` 探测数字是否等宽：等宽则零改动；不等宽则右对齐 + 最小宽度兜底（等宽字体注册列为可选框架改动）。
- **flash 而非滚动**：行情数字变化用颜色衰减表达（300ms `clock` easeOutCubic，JS 混色写 `setColor`），新 tick 可打断重启。高频路径不触碰 transform 弹簧。
- **Solid 只管结构**：增删行、排序、分组、路由仍走信号；热路径数值写节点 id，不产生 Solid 计算。
- **PresentCached 语义**：纯价格更新推进 picture，层树结构不变。`retain_measured` 对每帧都变的文本失效，`picture_writes` 每帧增量 ≤ 可见行 × 常数，进断言。

## 图表系统与节点预算

| 图表 | 实现 | 单屏预算 |
|---|---|---|
| 日K | 可见窗口约 80 根 ×（影线 1px + 实体矩形）；MA 为 3×80 个 2px 重叠块链（P3） | ≤ 500（含成交量） |
| 分时 | 聚合 80 列，1Hz 重算；实时点呼吸 | ≤ 100 |
| sparkline | 每行 24–30 列，`setScaleXY` 高度缩放 + 锚点补偿（探针定锚点），1Hz 且分帧错峰 | 每行 30 |
| 盘口 | 5+5 档价格/量/深度条，条宽 `setSize`（同屏仅 10 条） | ~40 |
| 十字光标 | 2 条 1px 线 + 浮标，长按激活，拖动跟手 | 4 |

- 横向窗口化：横向 `scroll`（复用 `ScrollPhysics` 惯性），内容一个 `sized` 定总宽；蜡烛 `absolute + insetLeft = i·slotW` 挂载，`onOffset` 计算可见 index 区间与挂载集 diff，只增删不重建。平移是 paint-only，不触发 Taffy。
- 缩放：`onZoom` 调 `slotW` → 一次 `setSize` 改总宽 + 重算窗口 + 更新已挂载蜡烛 inset（≈80 次 setter），按帧节流。
- 收盘价虚线、涨跌色、A股红涨绿跌与港美股反转由全局 `updown` 信号派生，设置可切。
- `transform` 缩放锚点未知：P0 写探针测试定锚点（中心则平移补偿），sparkline 与条形图共用结论。

## 动效

弹簧（低频结构）与 clock（高频数值）分层：

| 动效 | 实现 | 频率 |
|---|---|---|
| 价格 flash | `clock 300ms easeOutCubic` 混色 `setColor`，可打断 | 高频 |
| 指数条/大数字 | 跳值 + flash；入场 `clock 400ms` 滚动一次 | 高频/一次性 |
| 分时实时点 | `spring scale` bounce 循环 | 1Hz |
| 周期切换、Sheet、Dialog、TabBar 指示条 | 复用 spring/现有组件 | 低频 |
| 自选增删 | stagger 入场 / 折叠消失 | 低频 |

`reduceMotion` 下 flash 退化为静态涨跌色，弹簧落终值。时钟层时间源统一 `onFrame(targetMs)`，不用 `performance.now()`。

## 框架侧改动

最小集，全部 TS 侧：

1. `solid-ui` motion：`clock(durationMs, { ease, update, done })`、`stagger()`、缓动函数库，挂进现有 `installMotionTestHooks`。
2. `Toast` 增加 `action?: { label; onClick }`。
3. `solid-renderer` 导出 `colorBytes`（热路径直接 `host.setColor` 需要字节）。
4. 可选：等宽数字不满足时注册等宽 family（走现有 `register_font`）；`glue.c` 透传 `set_box_shadows`。

## 构建产物

入口 `apps/gallery-solid/src/quotes.tsx`，产物 `dist/quotes.js`（改名一次到位：justfile、`crates/macos` 加载路径、QuickJS 测试、文档同一提交更新）。`just gallery-dist`、`just host`、`just check` 语义不变。

## 测试与性能断言

| 断言 | 手段 |
|---|---|
| 引擎确定性 | 同种子两次运行 tick 序列逐位相等 |
| 每行每帧至多一次价格写入 | mock host 计数 setText |
| 不可见行零写入 | 长列表滚动后检查挂载集外节点 |
| 写入层 + 引擎步进 < 3ms | mock host 计时 |
| `picture_writes` 每帧增量有界 | `layerTreeStats` |
| 图表挂载节点 ∈ 窗口界 | 拖动/缩放后遍历树计数 |
| 静态页零帧申请 | `frameListenerCount() === 0` |
| 交易守恒 | 限价穿越成交、资金/持仓守恒、撤单 |
| 金样 | 确定性种子下初始帧重钉，容差每通道 2 |

## 阶段

| 阶段 | 交付 |
|---|---|
| P0 | 探针测试（等宽数字、缩放锚点）+ motion 层 + 引擎 + 路由骨架 + 产物改名 |
| P1 | `/watchlist` 全量（管线、flash、sparkline、自选管理、刷新） |
| P2 | `/stock/:symbol` + `/search`（分时、盘口、详情） |
| P3 | 日K 窗口化 + 十字光标 + 缩放 + MA + `/market` |
| P4 | 交易闭环 + `/profile` 设置 |
| P5 | 压测模式、旧 counter 退役、金样重钉、文档收尾 |
