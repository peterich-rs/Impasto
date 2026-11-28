# `examples/devtools-desktop` 实现方案 2：视觉规范、桌面工作台壳与 Canvas 图表引擎

> **适用范围与前置约束**
> - 本文档是 [独立进程外 DevTools 规划](standalone-devtools-plan.md) 的 Stage 2 细化落地文档（第 2/3 篇），定义 `examples/devtools-desktop/src/` 的薄 TS / Solid 2.0 UI 架构、Engineering Precision 视觉令牌系统、矢量图标集、5 区域可调桌面工作台壳以及基于 `CanvasRenderingContext2D` 批量绘制的高性能图表引擎。
> - **框架 API 对齐**：本方案严格基于仓库真实可用能力编写——`@impasto/solid-ui`（[index.tsx](../packages/solid-ui/src/index.tsx)）、`@impasto/solid-renderer`（[scene.ts](../packages/solid-renderer/src/scene.ts)、[jsx.ts](../packages/solid-renderer/src/jsx.ts)、[canvas/types.ts](../packages/solid-renderer/src/canvas/types.ts)）以及自动生成的样式类型（[style.gen.ts](../packages/solid-renderer/src/style.gen.ts)）。
> - **渲染管线铁律**：所有高密度图表（帧条带、5 泳道甘特、火焰图、心电图、网络瀑布、Treemap）全部走 `@impasto/solid-ui` 的 `Canvas` 单一主力光栅路径，禁止用成百上千个 `View` 节点拼装图表柱或火焰图块。

---

## 1. 薄 TS / Solid 2.0 目录与状态架构

### 1.1 `examples/devtools-desktop/src/` 目录布局

```text
examples/devtools-desktop/src/
├── main.tsx                     # boot(App)，安装全局快捷键与 OverlayRoot
├── theme/
│   ├── tokens.ts                # Dark / Light 双主题表面令牌 + 语义色表 + ColorBucket 映射
│   └── typography.ts            # 字体、等宽数字样式预设与紧凑密度常量
├── state/
│   ├── core.ts                  # @app/devtools-core 强类型封装 + ReadableStream<TickSummary> 消费器
│   ├── workspace.ts             # 工作台布局状态（活跃页、分栏宽高、Inspector/Console 抽屉、主题覆盖）
│   ├── selection.ts             # 全局选中上下文（selectedFrameId / Range / nodeId / netToken / auditId）
│   └── navigation.ts            # 因果深链跳转总线（navigate(intent)）与历史回退栈
├── components/
│   ├── Icons.tsx                # 24×24 线性矢量图标集（Canvas + Path2D，1.5px 描边）
│   ├── Divider.tsx              # 1px 水平/垂直分割线（应对 PaintStyle 统一 borderWidth）
│   ├── SplitPane.tsx            # 可拖拽分栏容器（col-resize / row-resize）
│   ├── ToolbarButton.tsx        # 紧凑桌面工具栏按钮（含状态点、计数徽章、hover/active 反馈）
│   ├── Badge.tsx                # 语义状态胶囊（Over Budget / Compositor / PaintPath / HTTP Status）
│   ├── KeyValueTable.tsx        # 等宽对齐键值对检查表（支持行点击复制）
│   ├── SliceTable.tsx           # 驱动 Rust 视口切片的虚拟滚动表格（表头排序 + 固定行高 + 选中高亮）
│   └── EmptyState.tsx           # 未连接 / 无数据 / 离线快照引导态
├── shell/
│   ├── Workbench.tsx            # 5 区域主壳组合与断线琥珀色 Banner
│   ├── Titlebar.tsx             # 顶部 40pt 全局栏（Target 选择器 / Rec / Live / Profile / Overlay / ⌘K）
│   ├── NavRail.tsx              # 左侧 48pt 导航轨（6 模块 + 错误计数徽章 + Console 开关）
│   ├── InspectorDrawer.tsx      # 右侧可调上下文检查器外壳（按页面与选中上下文切换内容）
│   ├── ConsoleDrawer.tsx        # 底部可折叠实时 Logs / Errors 抽屉
│   ├── StatusBar.tsx            # 底部 24pt 状态条（连接/RTT、Hitch/呈现频次/P95、缓冲计数、Profile）
│   └── CommandPalette.tsx       # ⌘K 模态命令面板（模糊过滤 + 键盘上下回车）
├── charts/
│   ├── buffer.ts                # Uint8Array -> Float64Array 安全对齐视图 & ProjectedCanvas 通用绘制器
│   ├── InteractiveCanvas.tsx    # 统一画布壳（CanvasController + pointerPosition 映射 + Zoom/Wheel/Drag/Hover）
│   ├── FrameStripChart.tsx      # 顶部常驻 / Frames 页帧条带阶段柱图
│   ├── LaneGanttChart.tsx       # 单帧 5 泳道甘特瀑布图
│   ├── FlamegraphChart.tsx      # JS CPU 火焰图
│   ├── SyncTrackCharts.tsx      # 4 轨时间同步实时心电图 + Heap/Resource 堆叠面积图
│   ├── NetworkWaterfallBar.tsx  # 9 阶段迷你瀑布条与详情精确瀑布图
│   └── TreemapChart.tsx         # 交互式 Bundle Squarified Treemap
└── pages/
    ├── OverviewPage.tsx
    ├── FramesPage.tsx
    ├── ViewsPage.tsx
    ├── JsRuntimePage.tsx
    ├── NetworkPage.tsx
    └── ResourcesPage.tsx
```

### 1.2 零轮询响应式状态流（`state/core.ts`）

用 `watchTicks()` 产出的 `ReadableStream<TickSummary>` 替代现有 [main.tsx](../examples/devtools-desktop/src/main.tsx#L90) 的 `setInterval(..., 500)`：

- **单一后台读取协程**：应用启动后调用 `watchTicks(abortController.signal)`，每收到一条 `TickSummary`（最多 `10 Hz`），在一个 `batch(...)` 内更新极薄信号：
  - 连接态信号：`connectionState()`（`"idle" | "connecting" | "connected" | "reconnecting" | "offline_snapshot"`）、`sessionInfo()`、`rttMs()`、`reconnectAttempt()`；
  - 实时 KPI 信号：`fps()`、`p95Ms()`、`jankCount()`、`compositorPct()`、`jsHeapBytes()`、`activeNetCount()`、`errorCount()`、`auditCount()`；
  - **通道版本号信号**：`framesRev()`、`repaintsRev()`、`networkRev()`、`logsRev()`、`errorsRev()`、`runtimeRev()`、`resourcesRev()`、`auditsRev()`；
  - 远程拾取事件：`lastPickedNode()`（当 `pickedSeq` 变化时自动派发 `navigate({ kind: "view", nodeId })`）。
- **细粒度重绘约束**：
  - 未展示的页面不订阅对应 `revision` 信号（利用 `<Show>` 条件挂载，切换页签时卸载非活跃页的 `Canvas` 与 `Effect`）；
  - 活跃页面的 `Canvas` 仅在自身关心的 `revision` 或视口缩放/选中信号变动时调用 `controller.invalidate()`，避免整棵 UI 树重排。

---

## 2. Engineering Precision 视觉设计令牌系统（`src/theme/`）

### 2.1 三阶中性表面与 `1px` 极细线条（Dark 默认 + Light 跟随系统）

通过 `@impasto/solid-ui` 的 `useTheme()` 监听 macOS 深浅外观切换（同时允许用户在设置中手动锁定 Dark / Light / System）：

| 语义角色 | Dark（默认） | Light | 用途 |
|---|---|---|---|
| `surface.canvas` | `#0D1014` | `#F5F7FA` | 窗口底板、图表背景 |
| `surface.panel` | `#14181F` | `#FFFFFF` | 主工作区、侧栏、检查器面板 |
| `surface.elevated` | `#1C222C` | `#EDF1F7` | 表头、选中态胶囊、弹出菜单、工具栏浮块 |
| `surface.hover` | `#232A36` | `#E4E9F2` | 列表行悬停、按钮悬停 |
| `surface.selected` | `#1E324D` | `#DCEAFE` | 表格/树当前选中行底色 |
| `border.hairline` | `#262D3A` | `#D8DFEA` | 所有区域 `1px` 分割线（由 `<Divider />` 渲染） |
| `border.strong` | `#364154` | `#B8C4D6` | 输入框聚焦外框、拖拽分割条高亮 |
| `text.primary` | `#E6ECF5` | `#121822` | 主标签、主数值 |
| `text.secondary` | `#9AA6B8` | `#4C596E` | 次级说明、列名、轴刻度 |
| `text.muted` | `#637085` | `#7E8B9E` | 占位符、无数据提示、辅助分隔符 |
| `accent.primary` | `#4C9AFF` | `#1D6FE8` | 主交互高亮、选中框、当前导航指示条 |
| `status.ok` | `#2EC486` | `#12945E` | Connected、2xx、预算内帧 |
| `status.warn` | `#F0B429` | `#C78200` | Reconnecting、3xx、警告 Audit、PicturePatch |
| `status.danger` | `#F05D5E` | `#D63638` | Over Budget / Jank、4xx/5xx、Error、FullRebuild |
| `status.info` | `#5AC8FA` | `#0E84C2` | Compositor-Only、FastLayerPatch、DNS |

> **样式边界实现要点**：核查 [style.gen.ts](../packages/solid-renderer/src/style.gen.ts#L91-L97) 可知 `PaintStyle` 当前提供统一的 `borderWidth` / `borderColor`，暂不提供 `borderRightWidth` / `borderBottomWidth` 等单边属性。因此所有面板间的 `1px` 分割线统一通过 `<Divider orientation="vertical" | "horizontal" />`（宽或高为 `1`、背景色为 `tokens.border.hairline`、`flexShrink: 0`）实现，既精确对齐 `1px` 视觉又零布局副作用。

### 2.2 与 Rust `projection.rs` 共享的 `ColorBucket` 语义色表

Rust Core 不硬编码十六进制色值，只向 Canvas 投影缓冲区写入稳定 `u16` 枚举索引 `ColorBucket`，由 TS 侧按当前 Dark/Light 主题映射为实际颜色字符串：

| `ColorBucket` 区间 | 枚举项 | Dark 色值 | Light 色值 | 对应图表维度 |
|---|---|---|---|---|
| `0..=7`（帧流水线） | `FrameScript` / `FrameLayout` / `FrameRecord` / `FrameQueue` / `FrameRaster` / `FrameGpu` / `FrameCompositor` / `FrameOverBudget` | `#F5B041` / `#7C83FD` / `#2DD4BF` / `#64748B` / `#3B82F6` / `#22C55E` / `#38BDF8` / `#F05D5E` | `#D9820B` / `#555CE0` / `#0D9488` / `#7C8EA3` / `#2563EB` / `#16A34A` / `#0284C7` / `#DC2626` | 帧条带、5 泳道甘特、Overview 轨道 |
| `16..=24`（网络阶段） | `NetQueue` / `NetProxy` / `NetDns` / `NetTcp` / `NetTls` / `NetSent` / `NetTtfb` / `NetDownload` / `NetError` | `#64748B` / `#94A3B8` / `#22D3EE` / `#FB923C` / `#A78BFA` / `#60A5FA` / `#34D399` / `#3B82F6` / `#F05D5E` | `#64748B` / `#94A3B8` / `#0891B2` / `#EA580C` / `#7C3AED` / `#3B82F6` / `#059669` / `#1D4ED8` / `#DC2626` | 网络 9 阶段瀑布条 |
| `32..=35`（重绘路径） | `PathFastLayer` / `PathPicturePatch` / `PathFullRebuild` / `PathCanvasRedraw` | `#3B82F6` / `#F59E0B` / `#F05D5E` / `#C084FC` | `#2563EB` / `#D97706` / `#DC2626` / `#9333EA` | 重绘热力胶囊、排行占比条 |
| `48..=54`（JS 堆分类） | `HeapObj` / `HeapStr` / `HeapAtom` / `HeapShape` / `HeapFunc` / `HeapBinary` / `HeapOther` | `#3B82F6` / `#22C55E` / `#2DD4BF` / `#A78BFA` / `#F59E0B` / `#FB7185` / `#64748B` | 同族加深版 | JS Heap 堆叠面积图 |
| `64..=68`（Bundle 分组） | `BundleApp` / `BundleFramework` / `BundleDeps` / `BundleAsset` / `BundlePool` | `#3B82F6` / `#2DD4BF` / `#8B5CF6` / `#F59E0B` / `#64748B` | 同族加深版 | Bundle Treemap |
| `80..=85`（辅助线与选中） | `GridLine` / `BudgetLine` / `Crosshair` / `SelectionFill` / `SelectionBorder` / `LabelMuted` | `#262D3A` / `#F05D5E` / `#9AA6B8` / `rgba(76,154,255,0.22)` / `#4C9AFF` / `#9AA6B8` | `#D8DFEA` / `#DC2626` / `#4C596E` / `rgba(29,111,232,0.18)` / `#1D6FE8` / `#4C596E` | 坐标网格、预算红线、悬停游标、框选区 |

### 2.3 排版规范（`src/theme/typography.ts`）

- **数据等宽铁律**：所有帧号、毫秒耗时、呈现频次、百分比、内存字节、HTTP 状态码、坐标矩形、源码行号、Hex/JSON 文本强制 `fontFamily: "Monospace"`（对应 macOS 宿主已注册的 `SF Mono` / `Menlo`），杜绝数字实时刷新时横向抖动。
- **高密度桌面字阶**：
  - `micro`: `11pt`（状态栏、图表轴刻度、热力徽章、快捷键提示）
  - `table`: `12pt`（表格行、树节点、日志行、键值对，固定行高 `24pt`~`26pt`）
  - `ui`: `13pt`（工具栏按钮、页签标题、筛选输入、检查器节标题）
  - `kpi`: `20pt` / `fontWeight: 600`（Overview 顶部主 KPI 数值）
- 严禁使用装饰性紫蓝渐变、呼吸光晕、Emoji 代替专业图标或嵌套三层圆角卡片。

---

## 3. 线性矢量图标系统（`src/components/Icons.tsx`）

参照 [examples/gallery-solid/src/widgets/stocks/icons.tsx](../examples/gallery-solid/src/widgets/stocks/icons.tsx) 的成熟方案，使用 `@impasto/solid-ui` 的 `Canvas` + `Path2D` 在 `24×24` 逻辑网格绘制 `1.5px` 线性矢量图标（圆头 `lineCap = "round"`、圆角 `lineJoin = "round"`），零位图资产且 Retina 2x/3x 下绝对锐利：

- **导航轨 6 图标**：`overview`（脉冲折线）、`frames`（胶片/时间轴柱）、`views`（嵌套图层树）、`js`（闪电/CPU 芯片）、`network`（双向球/传输箭头）、`resources`（网格 Treemap/存储块）。
- **工具栏与检查器图标**：`record`（实心圆点）、`stop`（实心方块）、`play`、`pause`、`trash`（清空）、`crosshair`（Pick 拾取）、`layers`（Repaint Rainbow）、`scan`（Layout Overflow）、`search`、`command`（`⌘K`）、`folderOpen`（导入产物）、`download`（导出）、`externalLink`（打开 IDE / Perfetto）、`copy`、`gc`（回收循环箭头）、`terminal`（Console 抽屉）、`chevronRight` / `chevronDown`、`close`、`refresh`、`warning`、`error`、`sun` / `moon`。
- **交互图标按钮（`ToolbarButton`）**：统一 `28×28` 紧凑命中区、`borderRadius: 5`、`cursor: "pointer"`，支持 `active` 高亮底色、右上角 `6px` 语义状态点（如录屏中红点、Pick 开启蓝点）以及悬浮 `Tooltip`。

---

## 4. 5 区域桌面工作台壳与交互协议（`src/shell/`）

### 4.1 空间布局与默认尺寸

```text
┌───────────────────────────────────────────────────────────────────────────────────────┐
│ Titlebar (40pt)                                                                       │
├───────────────────────────────────────────────────────────────────────────────────────┤
│ [条件] Reconnecting / Offline Snapshot Banner (28pt, 仅断线或离线回放态显示)              │
├────┬──────────────────────────────────────────────────────────────────────────────────┤
│    │ TopFrameStrip (76pt, 可折叠，Overview/Frames/Views 下可常驻)                        │
│ N  ├─────────────────────────────────────────────────────┬────────────────────────────┤
│ a  │                                                     │ InspectorDrawer            │
│ v  │ Main Workspace (flex: 1, minWidth: 420pt)           │ (默认 380pt, 可拖 280..640) │
│ R  │                                                     │                            │
│ a  ├─────────────────────────────────────────────────────┴────────────────────────────┤
│ i  │ [可折叠] ConsoleDrawer (默认 200pt, 可拖 120..420pt, 随时 ⌘` 拉起)                  │
│ l  ├──────────────────────────────────────────────────────────────────────────────────┤
│48pt│ StatusBar (24pt)                                                                 │
└────┴──────────────────────────────────────────────────────────────────────────────────┘
```

### 4.2 可拖拽分栏组件（`SplitPane.tsx`）

充分利用前置改造落地的 `CursorStyle::ColResize` / `RowResize` 与 `DragEvent`：
- **分割柄结构**：`5pt` 宽（或高）透明命中热区（`cursor: orientation === "horizontal" ? "col-resize" : "row-resize"`），内部居中一条 `1px` 视觉线；`onHover` / 拖拽中视觉线高亮为 `tokens.accent.primary`；
- **拖拽防抖与钳制**：`onDragBegin` 记录起始尺寸 `startSize` 并累计 `dx`（或 `dy`），`onDragBy` 直接将新尺寸钳制在 `[minSize, maxSize]` 更新局部 signal，不跨 FFI、无弹簧回弹延迟，拖拽跟手度与系统原生 App 一致；
- **双击复位**：支持 `onDoubleTap` 恢复默认推荐分栏宽度。

### 4.3 全局键盘快捷键与 `⌘K` 命令面板

利用 [scene.ts](../packages/solid-renderer/src/scene.ts#L343) 原生 `scene.onShortcut(spec, cb)` 在 `App` 根挂载时一次性注册，不干扰 macOS `NSMenu`（`⌘Q`/`⌘W`/`⌘M`/`⌘C`/`⌘V`/`⌘A` 由 [menu.rs](../crates/embedder/macos/src/menu.rs) 处理）：

| 快捷键 | `scene.onShortcut` / `onKey` 规格 | 动作 |
|---|---|---|
| `⌘K` | `"mod+k"` | 打开/关闭全局命令面板（切换目标、切页、触发 GC、切 Profile、开关 Overlay、导入导出） |
| `⌘1` ~ `⌘6` | `"mod+1"` .. `"mod+6"` | 直接切换 6 大主模块（Overview / Frames / Views / JS / Network / Resources） |
| `⌘F` | `"mod+f"` | 聚焦当前活跃页面的主搜索框（`FieldController.focus()`） |
| `⌘E` | `"mod+e"` | 导出当前会话 `.impasto-session`（或 Trace 页导出 `.pftrace`） |
| `⌘O` | `"mod+o"` | 打开原生文件面板导入产物（`.impasto-session` / `.pftrace` / `.impsym` / `.map` / `bundle-size.json`） |
| `⌘J` | `"mod+j"` | 展开/收起底部 `ConsoleDrawer` |
| `⌘I` | `"mod+i"` | 展开/收起右侧 `InspectorDrawer` |
| `Space` | 根聚焦容器 `onKey(" ")`（非输入框焦点时） | 暂停/恢复实时数据流（`setPaused(!paused())`） |
| `←` / `→` | `onKey("Left")` / `onKey("Right")` | 在 Frames 页按帧单步前/后步进选中帧 |
| `Esc` | `onKey("Escape")` | 依次关闭命令面板 → 退出 Pick 拾取模式 → 清除框选区间 |

---

## 5. Canvas 2D 高性能图表引擎（`src/charts/`）

### 5.1 通用投影绘制管线（`charts/buffer.ts` + `charts/InteractiveCanvas.tsx`）

所有 6 类图表复用同一套底层绘制与交互原语，杜绝重复代码：

1. **零分配缓冲区解码与批量提交（`drawProjectedCanvas`）**：
   ```ts
   export function asFloat64(bytes: Uint8Array): Float64Array {
     if ((bytes.byteOffset & 7) === 0 && (bytes.byteLength & 7) === 0) {
       return new Float64Array(bytes.buffer, bytes.byteOffset, bytes.byteLength >>> 3);
     }
     const copy = bytes.slice();
     return new Float64Array(copy.buffer, 0, copy.byteLength >>> 3);
   }

   export function drawProjectedCanvas(
     ctx: CanvasRenderingContext2D,
     proj: ProjectedCanvas,
     palette: readonly string[],
   ): void {
     // 1. 批量填充所有矩形色桶（主图元）
     for (const bucket of proj.rectBuckets) {
       if (bucket.rects.byteLength === 0) continue;
       ctx.fillStyle = palette[bucket.color] ?? "#888";
       ctx.fillRects(asFloat64(bucket.rects));
     }
     // 2. 批量绘制直线段（网格线、预算红线、游标、框选边）
     for (const seg of proj.segments) {
       if (seg.points.byteLength === 0) continue;
       ctx.strokeStyle = palette[seg.color] ?? "#888";
       ctx.lineWidth = seg.width;
       ctx.setLineDash(seg.dashed ? [4, 4] : []);
       ctx.beginPath();
       ctx.lineSegments(asFloat64(seg.points));
       ctx.stroke();
     }
     ctx.setLineDash([]);
     // 3. 批量绘制折线（心电图、趋势线）
     for (const line of proj.polylines) {
       if (line.points.byteLength === 0) continue;
       ctx.strokeStyle = palette[line.color] ?? "#888";
       ctx.lineWidth = line.width;
       ctx.beginPath();
       ctx.polyline(asFloat64(line.points));
       ctx.stroke();
     }
     // 4. 绘制少量标签（仅在矩形宽度充足或坐标轴刻度处）
     ctx.font = "11px Monospace";
     ctx.textBaseline = "middle";
     for (const label of proj.labels) {
       ctx.fillStyle = palette[label.color] ?? "#E6ECF5";
       ctx.textAlign = label.align === 1 ? "center" : label.align === 2 ? "right" : "left";
       ctx.fillText(label.text, label.x, label.y, label.maxWidth);
     }
   }
   ```
2. **统一指针坐标与手势适配（`InteractiveCanvas.tsx`）**：
   - 遵循 [examples/gallery-solid/src/widgets/stocks/chart.tsx](../examples/gallery-solid/src/widgets/stocks/chart.tsx#L115-L188) 的框架官方模式：
     - 通过 `controller={(c) => { canvasCtrl = c; }}` 绑定 `CanvasController`；
     - 在 `onPress`、`onClick`、`onHover`、`onDragBy`、`onWheel`、`onZoom` 内调用 `canvasCtrl?.toLocal(scene.pointerPosition())` 获取画布局部 `(x, y)`；
     - 在 `draw(ctx, size)` 首次或窗口缩放导致 `(size.width, size.height)` 变化时同步通知上游重算 Rust 投影尺寸（避免 `onLayout` 额外往返）；
     - 悬停 Tooltip 不重绘整张主图，可叠放极简绝对定位浮动 `<View>` 气泡（自动左右翻边防溢出画布）。

### 5.2 六大专业图表规格

| 图表组件 | 渲染形态 | 交互能力 | 性能与细节约束 |
|---|---|---|---|
| `FrameStripChart` | Split 帧左柱堆叠 UI 阶段，右柱并排显示 Raster 与 GPU。Shared 帧的 UI 和 Raster 累加为 CPU 柱。Compositor 帧使用全宽瓶颈柱。超预算阶段加红帽，未呈现帧加底部红条。预算虚线使用可见区最新帧的有效预算 | 单击选中单帧；拖拽水平框选多帧触发区间对比；滚轮左右平移历史帧；双指捏合 `onZoom` 调节柱宽（`2pt` ~ `18pt`） | `visible_count` 按 `width / barPitch` 由 Rust 裁剪，超出 2400 帧时仅投影视口可见柱 |
| `LaneGanttChart` | 5 条水平泳道（`Platform` / `UI` / `Raster` / `IO` / `GPU`），每条泳道内支持多层嵌套 Span 条；左侧 `64pt` 固定泳道头；GPU 泳道背景加细微区分并标注 `GPU Clock (rel 0.0ms)` | `onWheel` 左右平移 + `onZoom` 缩放时间轴；点击任意 Span 高亮并在检查器显示 `fields`、耗时与源码信息；`cursor: "grab" / "grabbing"` | 宽度 `< 1.5pt` 的微细 Span 由 Rust 在投影期合并为 LOD 摘要条，避免海量零宽绘制 |
| `FlamegraphChart` | 自顶向下（或自底向上翻转）的调用栈矩形层，同函数名用稳定哈希微调色相；宽度 `>= 42pt` 的帧内绘制截断函数名 | 单击任意函数块下钻放大至 100% 宽；双击空白或点击顶部 `Reset Zoom` 复位；悬停显示 `self%`、`total%`、符号化 `file:line:col`；支持顶部搜索框高亮匹配帧（未匹配帧降透明度） | 符号化与前缀树布局完全在 Rust `analyzer/flamegraph.rs` 完成，切换搜索词响应 `< 5ms` |
| `SyncTrackCharts` | Overview 4 轨同步心电图（Frame bottleneck / CPU+GPU / JS Heap / Active Net）与 JS Heap 7 分类堆叠面积图、Resources 趋势图 | 多轨共享同一 `hoverTimeMs` signal：鼠标滑过任意一轨时，其余 3 轨同步绘制垂直虚线游标并刷新各轨读数标签 | 采样点超过画布像素宽时由 Rust 执行 Min-Max 降采样，保留尖峰不削峰 |
| `NetworkWaterfallBar` | 表格行内 `110×12pt` 微型 9 阶段水平条 + 右侧 Inspector 精确 9 阶段瀑布（每阶段一行：阶段名、水平条、精确 `ms` 数值） | 行内迷你条直接由 `query_network_slice` 返回的紧凑 `segments: [startRatio, lenRatio, colorBucket]` 绘制（或直接由轻量化 `View`/共享 `Canvas` 渲染）；详情页可点击展开每跳 `RedirectHop` | Pending 进行中请求绘制动态推进条（截至当前时刻），杜绝空白列 |
| `TreemapChart` | 按 `bytes` 面积比例铺满画布的 Squarified Treemap 矩形，按 `group` 着色，一级分组带 `1px` 边框与左上角标题 | 悬停显示完整模块路径、`renderedBytes`、`apportionedBytecodeBytes`、`retainedPoolBytes`；点击目录/Chunk 下钻，点击顶部面包屑返回上级 | 矩形面积 `< 20×14pt` 自动隐藏文字标签，保持画面干净易读 |

---

## 6. 虚拟列表与大表渲染规范（`components/SliceTable.tsx`）

针对可能出现数千条记录的列表（Network 请求表、Logs 流、View 树、大 JSON 树、JS Profile 表格），明确两种框架协同策略：

1. **标准规模列表（`<= 2000` 行，如 Network 请求表、View 树可见展开行、Repaint 排行榜）**：
   - 使用 `@impasto/solid-ui` 内置 `VirtualList`（底层 Rust `ListDriver` 仅挂载可视区 + `overscan: 6` 行），列表 `items` 仅存放从 Rust `query_*_slice` 返回的轻量摘要结构体（或只存行索引/ID + 轻量显示字段，每行 `< 200 B`），**绝不把 Body、Stack、Full Headers 放进列表项对象**；
   - 启用 `recycle` 池（`poolSize: 12`），消除快速滚轮浏览时的节点反复创建与销毁。
2. **超大/流式列表（Logs 4000+ 行、大 JSON 50,000+ 行）**：
   - 使用 `ScrollArea`（固定 `contentMin = totalRows * ROW_HEIGHT`）配合 `onOffset` 监听滚动位置 `scrollY`；
   - 计算 `firstRow = Math.floor(scrollY / ROW_HEIGHT) - 8` 与 `visibleCount = Math.ceil(viewportH / ROW_HEIGHT) + 16`，向 Rust 请求 `query_*_slice(..., firstRow, visibleCount)`；
   - 仅渲染这 `~40~60` 行绝对定位（`top = (firstRow + i) * ROW_HEIGHT`）的等高行组件；
   - **Auto-Follow 尾部跟随**：Logs 流默认开启自动滚底，用户向上滚动 `> 24pt` 时自动取消锁定，底部出现紧凑 `↓ Jump to Latest` 浮动胶囊。
