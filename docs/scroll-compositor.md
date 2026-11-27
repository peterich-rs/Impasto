# 滚动与合成线程

应用可以在同一个滚动容器上组合下表的 API。结构变更、输入事件和 JavaScript 回调由 UI 队列处理。独立的 Raster 队列可以按显示时钟采样已安装的 LayerTree。

指针事件在 UI 队列按顺序处理，按下以最新可用布局命中，抬手和取消沿已建立的手势路线交付。平台合并移动事件时只用最终位置更新滚动，中间采样用于估算速度。抬手时只考虑最近 120 ms 的采样。等待第二次点击或长按时，宿主按最早到期时间安排一次唤醒。

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

`RefreshControl` 内置指示器用 overscroll 规则驱动。自定义指示器可使用 `ready()` 和 `busy()`，需要连续 `progress()` 时 JS 只在手势和回弹活跃期间采样。`NavigationBar` 接受 `scroll` 和 `scrollEffect="fade-backdrop"`。`Tabs` 接受一个水平 `pager` 控制器，以页面偏移驱动指示条。`Sheet` 默认带可下拉关闭的手柄，设置 `dragToDismiss={false}` 可隐藏。转场中需要透明度分组或毛玻璃的内容使用瞬态离屏纹理。 [场景滚动契约](../crates/engine/src/scene/scroll.rs)

## Gallery 业务页

自选与行情列表由 `VirtualList` 管理挂载窗口。行情行内的迷你图随该行预挂载，按行情发布批次更新。股票详情的主栏和分栏独立滚动。每次布局后由区块位置和视口高度生成两个 `useScrollThreshold` 边界，让快照在进入视口前 48 个逻辑点恢复接收行情。十字线读数和底部实时价格通过 `RepaintBoundary` 各自录制。 [详情区块](../examples/gallery-solid/src/widgets/stocks/section-visibility.ts) · [分时与 K 线](../examples/gallery-solid/src/widgets/stocks/chart.tsx)

模拟行情使用单调时钟每 200 ms 推进并发布一次。唤醒超时会限制模拟时间增量。行情列表仅在分批发布行数据时监听帧，股票详情仅在分批挂载区块时监听帧。Raster 可以在这些 UI 工作之间接管惯性滚动。 [行情时钟](../examples/gallery-solid/src/state/quotes-provider.tsx)

列表记录当前行 key 和行内偏移。空闲时，数据重排与高度校正以该位置为准。原行删除后，沿更新前的次序查找最近的存活行，并按新范围裁剪偏移。活动手势和显式滚动保持优先。

Gallery 的 Tab 分支按最近使用顺序保留。非活动分支退出绘制、命中、语义和页面帧回调。应用可以通过 `usePageActive()` 暂停页面订阅。系统内存压力会释放非活动分支。

## 诊断与验收

DevTools 性能页选中目标帧后，合成帧显示 Raster 托管和该帧的刷新率，回退帧显示阻断原因与可定位的节点。`scroll_callback` 指向注册通用回调的滚动节点及其联动目标，`global_frame_hook` 指向活跃的逐帧 JavaScript 监听，`list_window_not_settled` 表示已挂载窗口还在测量或更新。普通输入、到期定时器和外部更新会唤醒 UI。UI 按 FrameToken 取消 Raster 的下一次回调，并回写最近完成呈现的位置。已经提交的 Raster 帧可以继续上屏。Android 主线程收到触摸时会预先注册 Choreographer 回调。双帧提交队列满时，UI 仍处理本帧输入，并保存最新待提交的场景。

惯性滚动、刷新指示器和吸顶项直接在 Gallery 自选列表上验证：拖动列表后观察 `compositor_frame` 的时间间隔，并区分阈值、预挂载和惯性尾段的主动降频。

macOS 可用 `just host` 打开 Gallery。Gallery Android 产物的应用 ID 是 `dev.impasto.gallery`。框架独立验证壳使用 `dev.impasto.host` 和本地回放 fixture。Profile 版用 Gradle 参数 `-PimpastoProfile=true` 构建。构建环境变量 `IMPASTO_SURFACE_LATENCY` 可指定交换链延迟提示值。用 `tools/android-frame-trace.sh OUTPUT dev.impasto.gallery 6` 采样 Perfetto。像素回归测试 `declarative_scroll_bindings_render_identical_pixels_on_ui_and_raster` 在相同轨迹与时间戳上逐像素比较同一设备生成的两帧。
