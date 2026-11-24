# 滚动与合成线程

应用可以在同一个滚动容器上组合下表的 API。结构变更、输入事件和 JavaScript 回调由 UI 队列处理。Split 和 PlatformUi 拓扑允许 Raster 在 UI 空闲期间采样已安装的 LayerTree。

| 选型 | API | 适用场景 |
| --- | --- | --- |
| L1 原生滚动 | `ScrollArea`、`VirtualList`、`scrollTo`、`fling` | 位移、惯性、回弹、分页吸附、滚动条、当前吸顶项 |
| L2 离散规则 | `useScrollThreshold`、`useScrollSegment`、`VirtualList.onEndReached` | 回到顶部按钮、加载下一页、刷新就绪、分区高亮 |
| L3 连续规则 | `bindScrollStyle`、`interpolate`、`interpolateColor`、`createScrollLink(source, target, { factor, offset })` | 视差、透明度、模糊、背景色、着色器参数、两个视口同步 |
| L4 JavaScript 回调 | `ScrollArea.onOffset`、`ScrollController.onOffset`、`scene.onScroll`、`onFrame`、`createScrollLink(source, target, map)` | 任意布局修改、逐帧文案、自定义算法 |

## 离散规则

```tsx
const scroll = createScrollController();
const showTop = useScrollThreshold(scroll, { value: 300 });
const nearEnd = useScrollThreshold(scroll, { from: "end", value: 200 });
const section = useScrollSegment(scroll, () => [180, 420, 860]);
```

偏移和迟滞使用逻辑点。未指定轴时使用 Y。`from: "end"` 以距离末端为准，普通 X/Y 偏移上的负值默认表示距离末端。负的 overscroll 阈值从起始侧触发，例如 `{ axis: "overscrollY", value: -64 }`。非零迟滞让进入和退出边界分开，避免边缘抖动。分段索引从 0 开始，等于已到达的断点数。

列表的 `onEndReached` 在距底部达到 `endReachedDistance` 时触发，默认距离为估算行高乘以 overscan 行数。对于同一个条目数量只触发一次。列表仍按已挂载和完成布局的行计算滚动窗口。Raster 在距窗口末端约两次显示刷新时通知 UI，UI 预挂载、排版和录制新的行，随后复用宿主的异步图片解码与 Raster 的字形准备。吸顶项切换和阈值跨越会在采样目标时间交还 UI。 [列表窗口](../crates/engine/src/tree/list/driver.rs) · [滚动交还](../crates/embedder/common/src/host/compositor_animation.rs)

## 连续规则

```tsx
<View ref={(node) => {
  if (node === null) return;
  onCleanup(bindScrollStyle(node, scroll, {
    translateY: interpolate([0, 160], [0, 72]),
    scale: interpolate([0, 160], [1, 1.15]),
    opacity: interpolate([0, 160], [1, 0.6]),
    backdropBlur: interpolate([0, 160], [0, 12]),
    backgroundColor: interpolateColor([0, 160], ["#eef1ee", "#c6dbe2"]),
  }));
}} />
```

曲线的输入点严格递增，输出与输入一一对应。外推默认钳制到端点，数值曲线可用 `"extend"` 延伸首尾线段。旋转使用弧度，透明度使用 `0..1`，模糊使用逻辑点的标准差。颜色在预乘线性 RGBA 空间求值。规则拥有绑定期间的目标属性，释放规则后恢复普通样式。每个属性同时由一条规则驱动。控制器联动默认映射 Y，可选 `axis: "x"` 或 `"both"`。用函数参数可以表达非线性控制逻辑或多级滚动联动。

`RefreshControl` 内置指示器用 overscroll 规则驱动。自定义指示器可使用 `ready()` 和 `busy()`，需要连续 `progress()` 时 JS 只在手势和回弹活跃期间采样。`NavigationBar` 接受 `scroll` 和 `scrollEffect="fade-backdrop"`。`Tabs` 接受一个水平 `pager` 控制器，以页面偏移驱动指示条。`Sheet` 默认带可下拉关闭的手柄，设置 `dragToDismiss={false}` 可隐藏。Sheet、Dialog 和默认路由转场在满足内容缓存条件时复用 GPU 快照。 [场景滚动契约](../crates/engine/src/scene/scroll.rs) · [组件示例](../apps/gallery-solid/src/pages/compositor-lab.tsx)

## Gallery 业务页

行情、自选和市场列表由 `VirtualList` 管理挂载窗口。行情行内的迷你图随该行预挂载，按行情发布批次更新。股票详情的主栏和分栏独立滚动。每次布局后由区块位置和视口高度生成两个 `useScrollThreshold` 边界，让快照在进入视口前 48 个逻辑点恢复接收行情。十字线读数和底部实时价格通过 `RepaintBoundary` 各自录制。 [详情区块](../apps/gallery-solid/src/widgets/stocks/section-visibility.ts) · [分时与 K 线](../apps/gallery-solid/src/widgets/stocks/chart.tsx)

模拟行情使用单调时钟和定时器推进。行情列表仅在分批发布行数据时监听帧，股票详情仅在分批挂载区块时监听帧。Raster 可以在这些 UI 工作之间接管惯性滚动。`virtual-list-observed` 实验页保留连续偏移回调，用于复现 `scroll_callback`。 [行情时钟](../apps/gallery-solid/src/state/quotes-provider.tsx)

## 诊断与验收

DevTools 性能页选中目标帧后，合成帧显示 Raster 托管和该帧的刷新率，回退帧显示阻断原因与可定位的节点。 `scroll_callback` 指向注册通用回调的滚动节点及其联动目标，`global_frame_hook` 指向活跃的逐帧 JavaScript 监听，`list_window_not_settled` 表示已挂载窗口还在测量或更新。普通输入、到期定时器和外部更新会唤醒 UI，并先回写屏幕上最后实际呈现的位置。

在 Gallery 打开以下深链可自动启动惯性滚动。用 `IMPASTO_COMPOSITOR=0` 在相同宿主切换到逐帧 UI 驱动。验证节奏时查看连续 `compositor_frame` 的时间间隔，并区分阈值、预挂载和惯性尾段的主动降频。

- `impasto://compositor-lab/virtual-list` 检查默认刷新指示器、滚动条和吸顶项。
- `impasto://compositor-lab/virtual-list-observed` 检查通用回调触发的 `compositor_deferred` 事件和节点。
- `impasto://compositor-lab/scroll-threshold` 检查单次 `compositor_threshold_handoff` 与 300 点处的按钮显隐。
- `impasto://compositor-lab/scroll-style` 检查毛玻璃导航栏、视差和头图颜色。

macOS 可用 `just host` 打开 Gallery。Android Profile 版用 Gradle 参数 `-PimpastoProfile=true -PimpastoThreadTopology=split` 构建，再用 `tools/android-frame-trace.sh OUTPUT dev.impasto.host.profile 6` 采样 Perfetto。像素回归测试 `declarative_scroll_bindings_render_identical_pixels_on_ui_and_raster` 在相同轨迹与时间戳上逐像素比较同一设备生成的两帧。

### Pixel 9 Split 采样

在 Pixel 9 的 Vulkan Profile 版上，每个深链冷启动并采集 5 秒。表里的 UI 帧只计首末合成帧之间的 `Impasto frame`。高速段 p95 取惯性降为 60 Hz 之前、同一段中相邻 Raster 帧的起始时间差。

| 场景 | Raster 帧 | 合成区间内 UI 帧 | 高速段帧间隔 p95 |
| --- | ---: | ---: | ---: |
| 带下拉刷新的虚拟列表 | 241 | 7 | 8.846 ms |
| 300 点返顶阈值 | 230 | 1 | 8.717 ms |
| 视差、颜色和模糊 | 226 | 0 | 8.970 ms |

阈值示例在一次 `compositor_threshold_handoff` 后进入 UI 帧，随后重新交给 Raster。虚拟列表记录到两次提前预唤醒。加入连续滚动回调的对照组有 0 帧 Raster 托管，Perfetto 记录到 `scroll_callback` 和滚动节点。三组高速段 p95 都超过 8.33 ms。macOS Metal 的轨迹逐像素对照通过，iOS 模拟器完成首帧冒烟测试。
