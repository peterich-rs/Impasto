# `examples/devtools-desktop` 实现方案 3：六大页面详设、因果深链与分阶段验收

> **适用范围与前置约束**
> - 本文档是 [独立进程外 DevTools 规划](standalone-devtools-plan.md) 的 Stage 2 细化落地文档（第 3/3 篇），定义 6 大诊断页面的完整交互与数据绑定、跨面板因果深链状态机、M0–M3 分阶段实施计划以及端到端自动化测试与视觉验收标准。
> - **依赖文档**：本篇引用 [Rust Core 与协议微补丁](devtools-desktop-rust-core.md) 定义的 `@app/devtools-core` 接口，以及 [视觉、工作台壳与 Canvas 图表引擎](devtools-desktop-ui-and-charts.md) 定义的组件与图表原语。

---

## 1. 跨面板因果深链状态机（`src/state/navigation.ts`）

独立 DevTools 的核心产品竞争力在于**任意异常可一键穿透到根因现场**，拒绝数据孤岛。所有跨页跳转统一经由类型化 `NavigationIntent` 派发，并记录至有界导航栈（支持 `⌘[` / `⌘]` 历史后退前进）：

```ts
export type NavigationIntent =
  | { kind: "overview" }
  | {
      kind: "frame";
      frameId: number;
      inspectorTab?: "breakdown" | "repaints" | "js";
      highlightSpanId?: number;
    }
  | {
      kind: "view";
      nodeId: string;
      mode?: "tree" | "leaderboard" | "layout" | "layers";
      fromFrameId?: number;
      highlightOnTarget?: boolean;
    }
  | {
      kind: "js";
      subTab?: "flamegraph" | "memory" | "resources" | "console";
      frameId?: number;
      search?: string;
    }
  | {
      kind: "network";
      token?: number;
      inspectorTab?: "timing" | "headers" | "body" | "ws";
      statusFilter?: "all" | "pending" | "2xx" | "4xx" | "5xx" | "error";
    }
  | {
      kind: "resources";
      subTab?: "trends" | "bundle" | "archive";
    }
  | {
      kind: "console";
      drawer?: boolean;
      filterFrameId?: number;
      errorSeq?: number;
    };
```

### 6 条核心因果穿透路径

1. **Overview Live Audit → 异常现场**：点击任意诊断告警直接派发对应 `NavigationIntent`（如 `frame.jank_burst` → 跳 `Frames` 选中最慢帧；`view.repaint_hotspot` → 跳 `Views` 定位热点节点）。
2. **Frames 掉帧柱 → 当帧重绘 View → 目标端高亮 → IDE 源码行**：在 Frames 页点击超预算红柱 → 右侧切到 `Repainted Views` 页签 → 点击 `FullPictureRebuild` 节点既可在目标端即时高亮（`setOverlay({ highlightNode })`），也可一键进入 `Views` 树定位并调用 `openSourceInEditor(...)`。
3. **Frames 掉帧柱 → 当帧 JS 火焰图**：点击右侧 `JS & Reactive` 页签中的 `Open in Flamegraph` → 跳 `JS Runtime (flamegraph)` 并自动按该 `frameId` 过滤采样栈。
4. **Frames `compositor_block` → 阻断节点**：帧详情展示 `compositorBlock.reason` 与 `compositorBlock.node` → 点击节点 ID 直接跳转 `Views` 树并高亮该阻断合成器的节点。
5. **Views 设备端 Pick 拾取 → 树自动展开定位**：开启顶部 `🎯 Pick` → 在目标 App 屏幕点击任意元素 → `repaints` 通道推送 `view.picked` → `TickSummary.lastPickedNodeId` 驱动 `Views` 页自动展开全部祖先链、滚动到视图中央并展示盒模型与重绘历史。
6. **Console / Errors 堆栈帧 → IDE 源码行 / 关联帧**：任意 Error 或 Log 若携带 `frame`，点击帧号徽标直达 `Frames` 对应帧；点击已通过 `devtools-symbols` 符号化的源码行直达 `openSourceInEditor`。

---

## 2. 六大功能页面详细规格

### 2.1 Overview（健康总览与智能诊断）

- **布局结构**：
  1. **顶部 KPI 紧凑指标条（6 格等宽横排，无厚重卡片套娃，1px 竖线分隔）**：
     - `FPS`（实时帧率 + 目标刷新率）
     - `Frame P95`（毫秒 + 超预算帧占比徽章）
     - `Compositor-Only`（纯合成快路径占比 %）
     - `JS Heap`（`memory_used_size` + 相对基线增减箭头）
     - `Network`（总请求数 + 活跃 Pending 数 + 失败红点）
     - `Issues`（Errors + Layout Overflows + Active Audits 汇总）
  2. **中部 4 轨时间同步实时心电图（`SyncTrackCharts`）**：
     - 轨道 1：`Frame Total vs Budget`（柱+折线，红线标预算）
     - 轨道 2：`UI / Raster / GPU`（三色折线）
     - 轨道 3：`JS Heap (MiB)`（面积图）
     - 轨道 4：`Active Network & Latency`（并发线 + 完成点）
     - 右上角时间窗切换：`10s` / `30s` / `60s`。
  3. **底部 Live Audits 诊断列表**：展示 `getLiveAudits()` 返回的规则告警（严重级别图标、标题、量化指标胶囊、修复提示、`Inspect →` 跳转按钮）。
- **右侧 Inspector**：当选中某条 Audit 时，展示该规则的详细触发证据（如最慢 5 帧列表或 Top 5 重绘节点），支持直接点击穿透；未选中时显示会话环境摘要（`SessionInfo`：平台、OS、窗口尺寸/DPR/刷新率、线程拓扑 `ui-boot`/`platform`、协商能力集 `Capability`）。

### 2.2 Frames（帧处理流水线与时间轴）

- **动态订阅**：进入本页确保 `Frames` 激活；当切换右侧 `Repainted Views` 页签时按需增补 `Repaints`。
- **布局结构**：
  1. **顶部帧条带（`FrameStripChart`，高 `110pt`）**：展示最近帧柱，支持点击单帧、拖拽框选多帧对比均值、双指捏合缩放、工具栏内置 `Jump to Worst Jank` 按钮与 `Rec Trace`（`Stages` / `Detailed + JS Sampling`）录制控制。
  2. **主工作区（双视图切换）**：
     - **`Timeline (5 Lanes)`（默认）**：渲染 `LaneGanttChart`，展示选中帧的 `Platform / UI / Raster / IO / GPU` 5 泳道甘特瀑布；若当前帧属于已完成的 Trace Capture，自动调用 `loadFrameSpans` 展开完整嵌套 Span 并标注 `omitted_spans`（如有）；
     - **`Capture Summary`（录制完成后自动可用）**：展示本次录制总帧数、掉帧数、Span 丢失统计（`overwritten_spans` / `open_spans_at_stop`）、录制内慢帧排行榜，以及一键 `Export .pftrace` / `Open in Perfetto` 动作条。
  3. **右侧 Inspector（3 联动页签）**：
     - **`Breakdown`**：
       - 顶层调度：`vsync → ui → queue → raster → submit → present` 与 `e2e_ms`、`topology`、`in_flight_depth`、`skip_reason`、`compositor_block`；
       - UI 阶段细分：`motion / dispatch / timer / raf / callback / flush / layout_cb / laywork / record / canvas_record / semantics` 与 `layout_passes`、`layout_cause`、`text_shaped / reused`；
       - Raster 4 阶段与上传：`picture_prepare / glyph_prepare / compose / encode / queue_submit / present`、`draws / passes / offscreen_passes / direct_present`、`vertex / uniform / stop / mesh / atlas` 上传字节；
       - GPU Pass 表（若 `gpu.state === "ready"`）：逐 Pass 展示 `kind`、`draws`、`duration_us`、`shader_id`、`view_id`。
     - **`Repainted Views (N)`**：列出该帧 `queryFrameRepaints(frameId)` 的重绘 View 清单（头部汇总 4 种 `PaintPath` 计数与 `overflow_count`），点击任意节点同步联动目标端高亮并支持跳转到 `Views` 树。
     - **`JS & Reactive`**：展示该帧 `updates`、`sets`、`nodes_created`、`ffi_batches`、`ffi_calls`、`style_resolves`、`canvas_ops/redraws/reused` 以及当帧采集到的符号化 JS 调用栈预览（附 `Open in Flamegraph` 深链）。
     - **多帧框选模式**：当用户框选 `[startId, endId]` 时，右侧自动切换为 `Range Compare` 面板，并排对比该区间与全窗口的 P50 / P95 / Avg UI / Avg Raster / Jank Rate。

### 2.3 Views（View 树、重绘剖析与布局）

- **动态订阅**：进入本页自动增补 `Repaints` 通道并默认拉取一次 `refreshViewSnapshot(2000)`；离开超过 `5s`（且未开 Rainbow/Pick）自动退订 `Repaints`。
- **顶部功能栏**：
  - 视图模式：`Tree` | `Repaint Leaderboard` | `Layout Issues` | `Layer Tree`；
  - 实时控制开关：`🎯 Pick`（调用 `setOverlay({ pickMode })`）、`🌈 Rainbow`（`repaintRainbow`）、`📐 Layout Scan`（`layoutScan`）、`↻ Snapshot` 手动刷新；
  - 筛选框：搜索节点 ID / 组件名 / 文本内容 / 源码路径，以及重绘路径过滤胶囊（`All` / `FullRebuild` / `PicturePatch` / `CanvasRedraw`）。
- **主工作区 4 模式**：
  1. **`Tree`**：基于 `queryViewTree` 的虚拟滚动缩进树。每行显示展开箭头、`kind` 标签、组件名（若有 `source.component`）、截断文本预览、右侧紧贴**累计重绘次数徽章**与**近 60 帧微型热力条**（按最重路径着色）；鼠标 `onHover` 经 `50ms` 防抖调用 `setOverlay({ highlightNode })` 在目标设备上实时框选高亮。
  2. **`Repaint Leaderboard`**：展示 `refreshRepaintLeaderboard(100)` 倒序热力表：节点/组件、`Total Repaints`、`Recent / 60f`、4 路径堆叠占比条、`Total Layouts`、`Last Path & Reason`，一眼锁定无效重绘元凶。
  3. **`Layout Issues`**：调用 `refreshLayoutIssues(true)` 展示所有主轴溢出容器（`parent`、`axis`、`parentSize`、`used`、`excess`、`worstChild`），点击任意条目在目标端高亮容器并展开树。
  4. **`Layer Tree`**：在 `DeepInspect` 档位下调用 `loadLayerTree(frameId)` 展示引擎提交的 `Layer` 层级与 `Picture` 表（`picture_id`、`generation`、`origin`、`commands`、`images`）；非 `DeepInspect` 时提供一键切换至 `DeepInspect` 按钮。
- **右侧 Inspector**：
  - 盒模型图示（位置 `(x, y)` 与宽高 `(w, h)`、`focusable`）；
  - 源码位置卡片（`component` + `path:line:col` + `Open in IDE` 按钮 + 编辑器选择 `VS Code / Cursor / Zed`）；
  - 节点重绘剖析（4 路径计数、最新 `RepaintReason` 位标志解码为可读标签如 `STYLE_PAINT`、`LAYOUT_GEOMETRY`、`TEXT_CONTENT`、`CANVAS_CALLBACK`、以及最近触发重绘的帧号胶囊，点击帧号反跳 `Frames` 页）。

### 2.4 JS Runtime（引擎、内存与响应式）

- **动态订阅**：进入本页自动增补 `Runtime` + `Resources` 通道并触发 `refreshJsRuntime()`。
- **布局结构**：
  1. **顶部操作条**：实时内存摘要（`Used` / `Malloc` / `Limit` 占比条）、`Run GC` 按钮（触发 `runJsGc()` 后弹出紧凑 Diff 条显示回收字节与耗时）、采样器控制组（开启高频采样开关 + `interval_us` 预设 `500µs / 1ms / 2ms` + `max_depth: 32`）以及 `.impsym/.map` 符号表状态徽章（未加载时提供一键 `Load Symbols…`）。
  2. **上部 `JSMemoryUsage` 7 分类堆叠面积图**（`projectJsMemoryChart`）：直观展示 `Objects`、`Strings`、`Atoms`、`Shapes`、`JS Functions + Bytecode`、`Binary Objects`、`Other` 随时间走势。
  3. **下部 3 子页签**：
     - **`CPU Flamegraph`**：展示 `FlamegraphChart`（支持按单帧筛选或全窗口聚合）+ 搜索高亮框 + 切换 `Flamegraph` / `Top-Down` / `Bottom-Up` 视图；点击任意函数行支持 `Open in IDE`。
     - **`Timers & Promises`**：展示 `js.resources` 的实时计数卡（`promises / peak`、`timers / peak`、`listeners`、`queued_events & bytes`、`discarded_events`、`rejection_overflows`、`job_limit_hits`、`budget_overruns`）与活跃 `active_timers` 虚拟表（`id`、`kind: timeout|interval|animation_frame`、`due_in_ms`、`interval_ms`）。
     - **`Console & Errors`**：全屏级日志与异常表（按源 `script / layout / host / net` 与级别 `ERROR / WARN / INFO / DEBUG` 多选过滤，展开异常查看完整已符号化堆栈并支持行级跳转 IDE）。
- **右侧 Inspector**：
  - 选中火焰图函数 → 显示 `Self / Total Samples & %`、符号化文件路径与调用父子栈；
  - 选中 GC 历史记录 → 显示回收前后全 12 项 QuickJS 内存分类增减对比表；
  - 选中 Error → 显示完整原始栈 vs 符号化栈切换与一键复制。

### 2.5 Network（全链路网络与包体）

- **布局结构**：
  1. **顶部过滤栏**：全文检索框（支持匹配 URL / Method / Header / Body 预览）、类型胶囊（`All` / `HTTP` / `WS` / `Image`）、状态胶囊（`All` / `Pending` / `2xx` / `3xx` / `4xx/5xx` / `Failed`）、以及实时汇总（总条数、总下行字节、平均 TTFB）。
  2. **主区虚拟请求表（`SliceTable`）**：
     - 列：`Status`（彩色胶囊，Pending 为动态脉冲点）、`Method`、`Name / Path`（副行展示 Host）、`Initiator`（`fetch` / `http` / `websocket` / `image`）、`Proto`（`h2`/`http/1.1` + 复用图标 `♻`）、`Size`、`Time`、`Waterfall`（内联 `110pt` 9 阶段迷你瀑布条）；
     - 支持点击列头排序（时间、耗时、体积、状态），默认按发起时间正序（新请求追加尾部）。
  3. **右侧 Inspector（4 页签）**：
     - **`Timing`**：完整 **9 阶段精确瀑布图**（`Queueing / Proxy / DNS / TCP / TLS / Request Sent / TTFB / Download / Total`）+ 连接元信息表（`reused`、`remote_addr`、`resolved_ips`、`protocol`、`alpn`、`tls_version`、`tls_cipher`）+ 若 `redirected == true` 则可折叠展开全部 `RedirectHop` 逐跳状态码、`Location` 与分跳瀑布。
     - **`Headers`**：分区展示 General（Request URL / Final URL / Method / Status / Initiator）、Request Headers、Response Headers（对 `***` 脱敏字段显示锁图标与 `IMPASTO_DEVTOOLS_RAW_HEADERS=1` 调试提示）。
     - **`Payload & Response`**：选中时按需调用 `fetchNetworkBody(token, "request" | "response")` 走 Binary Frame 拉取完整包体；提供视图切换：
       - `JSON Tree`（由 Rust `JsonIndex` 驱动虚拟化折叠展开与页内搜索）
       - `Raw`（等宽分页文本预览）
       - `Image`（响应体为图片时落盘缓存路径交 `<Image />` 预览并显示像素尺寸）
       - `Hex`（二进制载荷十六进制 + ASCII 双栏视图）
       - 若存在 `truncated` 或 `evicted` 标记，顶部醒目提示原因；工具栏提供一键 `Copy as cURL`（写入 `@impasto/clipboard`）与 `Copy Body`。
     - **`WS Frames`**（仅 `kind === "socket"` 时显示）：展示 `ws_sent / ws_received` 与收发总字节，表格列出方向箭头（`↑ out` / `↓ in`）、`opcode`（`text`/`binary`/`open`/`close`/`error`）、`bytes_len`、时间戳与载荷预览；点击单帧按需经 `net.get_body(token, frame_seq)` 拉取完整帧体用 JSON/Raw/Hex 查看。

### 2.6 Resources & Bundle（资源趋势、包体 Treemap 与离线归档）

- **3 个紧凑子视图切换**：
  1. **`Runtime Resources`**：展示 `projectResourceTrends` 多维趋势图（`Atlas Bytes & Entries / Hit Rate / Evictions`、`Offscreen Attachment Peak Bytes`、`Image Resident Bytes`、`Texture / Picture / Layer Count`），协助排查显存/图集缓存抖动。
  2. **`Bundle Analyzer`**：
     - 未导入时呈现明确拖拽/点击引导卡片（`Open bundle-size.json…`），说明如何用 `impasto build bundle --analyze-size` 生成；
     - 导入后顶部显示 `Artifact Bytes`、`Startup Bytes`、`Chunks`、`Static Pool Dedup Saved` 指标，主区渲染交互式 `TreemapChart`（支持切换 `Apportioned Bytecode` / `Rendered JS` 指标与点击下钻），右侧列出 Top 模块排行与 `externalAssets` 清单。
  3. **`Session Archive & Artifacts`**：
     - 一键 `Export .impasto-session`（打包当前帧、重绘、网络元数据与预览、JS 内存/火焰图、日志与诊断结论为 Gzip 归档，经原生 `NSSavePanel` 保存，供离线复盘或提 Issue 附带）；
     - 一键 `Open Artifact…`（支持 `.impasto-session` 恢复只读回放现场、`.impsym`/`.map` 加载或替换符号表、`.pftrace` 校验与打开 Perfetto、`.json` 包体报告）；
     - 展示当前已加载的符号表与归档文件元信息。

---

## 3. 分阶段实施路线（M0 → M3）

为保证每阶段均可独立编译、测试与验收，按以下 4 个里程碑推进：

| 里程碑 | 核心交付范围 | 涉及主要路径 | 可验收里程碑产物 |
|---|---|---|---|
| **M0：协议微补丁与工程配置** | 1. 补齐 `view.layout_issues` RPC<br>2. `FrameRecord` 加性透出 `SubmitStats`/`AtlasStats`/`RenderStats` 已有字段<br>3. `DeviceSessionEntry.source_map` 透传<br>4. 更新 `examples/devtools-desktop/{app.toml,package.json,rust/Cargo.toml}` | `crates/devtools-protocol/` · `crates/embedder/common/src/diagnostics/` · `crates/cli/src/devtools.rs` · `examples/devtools-desktop/` | `just check && just boundaries` 全绿；`impasto sync --platform macos` 生成新类型声明 |
| **M1：Rust Core 底座 + Workbench 主壳 + Frames & Console 闭环** | 1. 后台双工 WS v2 引擎（`client.rs`）+ 断线快照保留 + `TypedSink<TickSummary>`<br>2. `FrameStore` / `LogStore` / `projection.rs`（Frame Strip + 5-Lane Gantt）<br>3. Engineering Precision 令牌、矢量图标、5 区域 `Workbench` + `SplitPane` + `⌘K` + `ConsoleDrawer`<br>4. 完整 **Frames** 页（帧条带、5 泳道甘特、Trace 录屏、`.pftrace` 导出） | `examples/devtools-desktop/rust/src/{client,types,projection,actions}.rs` · `rust/src/store/{frames,logs}.rs` · `rust/src/analyzer/gantt.rs` · `src/{theme,state,components,shell,charts,pages/FramesPage}.tsx` | 对 `gallery-solid` 运行 `impasto run -d macos --devtools`，实时以 60/120fps 渲染帧条带与 5 泳道甘特，可录屏导出 `.pftrace` 并实时查看 Console |
| **M2：Network 全链路 + Views 重绘剖析与远程 Overlay** | 1. `NetworkStore` + `analyzer/json_index.rs` + Binary Body 缓存 + `format_curl`<br>2. `RepaintStore` + `view.snapshot` 树索引 + `view.set_overlay` / `view.picked` 联动<br>3. 完整 **Network** 页（虚拟表、9 阶段瀑布、Redirect 展开、JSON 折叠树/Raw/Image/Hex、WS 双向帧）<br>4. 完整 **Views** 页（Tree/Leaderboard/Layout Issues/Layer Tree + Pick/Highlight/Rainbow + IDE 跳转） | `rust/src/store/{network,repaints}.rs` · `rust/src/analyzer/json_index.rs` · `src/charts/NetworkWaterfallBar.tsx` · `src/pages/{NetworkPage,ViewsPage}.tsx` | 实测 `gallery-solid` 行情请求 9 阶段瀑布与 JSON 树检查；开启 Pick 点击目标 App 精准定位树节点；掉帧 → 重绘 View → IDE 源码行全链路打通 |
| **M3：JS Runtime + Overview/Audits + Resources/Bundle/Archive** | 1. `JsStore` + `analyzer/{flamegraph,audits,treemap}.rs` + `archive.rs`（`.impasto-session` 读写）<br>2. 完整 **JS Runtime** 页（7 分类内存图、主动 GC Diff、符号化火焰图、Timers/Promises）<br>3. 完整 **Overview** 页（KPI + 4 轨同步心电图 + Live Audits 一键穿透）<br>4. 完整 **Resources & Bundle** 页（资源趋势 + Squarified Treemap + 离线会话导入导出）<br>5. `justfile` 补充 `devtools-desktop` 检查配方 | `rust/src/store/{js_runtime,resources}.rs` · `rust/src/analyzer/{flamegraph,audits,treemap}.rs` · `rust/src/archive.rs` · `src/charts/{FlamegraphChart,SyncTrackCharts,TreemapChart}.tsx` · `src/pages/{OverviewPage,JsRuntimePage,ResourcesPage}.tsx` | 6 大页面全部完备，离线导出 `.impasto-session` 重载还原现场，导入 `bundle-size.json` 下钻交互流畅，全部自动化测试与视觉巡检通过 |

> **并行实施建议**：M1 合并后，M2 的 **Network** 与 **Views** 数据流完全正交，可并行开发；M3 的 **JS Runtime**、**Overview/Audits**、**Bundle/Archive** 同样边界清晰，可独立按模块落地。

---

## 4. 端到端测试、性能红线与视觉验收清单

### 4.1 Rust Core 自动化测试（`cargo test --manifest-path examples/devtools-desktop/rust/Cargo.toml`）

| 测试套件 | 核心覆盖场景 |
|---|---|
| `client` Mock WS v2 集成测试 | 启动本地 `TcpListener` 模拟 Agent v2：验证认证握手、7 通道增量推送与 `reset=true` 恢复、GPU 回填原地 patch、Binary Frame `NetBody`/`ViewSnapshot`/`TraceExport` 解码、断线后快照保留与新 PID 重连 |
| `projection` 几何与命中测试 | 验证 `FrameStrip`、`5-Lane Gantt`、`Flamegraph`、`Treemap` 输出的 `Bytes` 长度均为 `32` 字节对齐倍数、坐标严格落在 `[0, width] × [0, height]`、且 `hit_test_*` 与绘制矩形坐标完全自洽 |
| `analyzer` 算法单测 | 验证 `devtools-symbols` 堆栈还原、火焰图前缀合并与缩放归一化、`JsonIndex` 折叠/展开/搜索切片、7 条 `Live Audits` 规则边界触发、`bundle-size.json` schema v1 解析与 Squarified Treemap 面积守恒 |
| `archive` & `actions` 安全单测 | 验证 `.impasto-session` 导出→导入无损往返、gzip 超过 `128 MiB` 拒绝（防炸弹）、`open_source_in_editor` 拦截 `../` 路径穿越与非法编辑器名、`format_curl` 正确转义单引号且不还原脱敏头 |

### 4.2 仓库常规门禁与 macOS 自举验收

每完成一个里程碑，必须依次执行以下检查：

1. **静态检查与边界门禁**：
   ```bash
   cargo fmt --all -- --check
   just check
   just boundaries
   cargo clippy -p embedder-macos --all-targets -- -D warnings
   just devtools-desktop
   ```
2. **macOS 应用同步与构建验证**：
   ```bash
   cargo run -p cli -- --project examples/devtools-desktop sync --platform macos
   cargo run -p cli -- --project examples/devtools-desktop build macos
   ```
3. **双进程 Dogfooding 实测与画面验收**：
   - 启动目标应用：`cargo run -p cli -- --project examples/gallery-solid run -d macos --devtools`；
   - 验证自动发现 `gallery-solid` 进程并零手工 Token 直连；
   - 使用 `IMPASTO_DUMP=/tmp/devtools-frame.png` 导出首帧，并分别设置 `IMPASTO_DEVTOOLS_APPEARANCE=dark` 和 `IMPASTO_DEVTOOLS_APPEARANCE=light` 检查两种主题下的核心页面截图，人工核对：
     - 字体清晰度（等宽数字列对齐、中文与符号无缺字）；
     - 1px 分割线、预算红线、色板对比度、文字无截断重叠；
     - `col-resize` / `row-resize` / `pointer` / `grab` 光标形态切换；
   - 连续快速滚动 `gallery-solid` 列表制造高频帧与网络流量，同时用第二个 DevTools 实例连接 `devtools-desktop` 自身（自举观测），确认 **性能验收红线**：
     - DevTools 桌面端在 `10 Hz` 通道持续推送下，自身 UI/JS 帧耗时 `< 1.0ms`、稳定满帧无掉帧；
     - 持续连接 `10 min` 后 DevTools 自身 JS 堆与 Rust 内存稳定在有界平台上，无泄漏爬升；
     - 关闭 `devtools-desktop` 后，目标 `gallery-solid` 自动清理 Overlay/Sampler Lease 且无僵尸 `adb forward` / `iproxy` 子进程残留。
