# 架构

当前代码的结构。未落地的事在 `docs/refactor-plan.md`。和代码不一致时，先改已经跑起来的那一边。

## 仓库

原生代码在 `crates/`，用 Cargo。JS 是仓库根的 pnpm workspace（`packages/*`、`apps/*`）。Kotlin 只用 `kotlin/` 里的 Gradle。根上的 `settings.gradle.kts` 没有 `include`。

```
crates/protocol/         DrawList、LayerTree
crates/uicore/           渲染树。Taffy、命中、手势、焦点
crates/engine/           渲染引擎。读 LayerTree，调用 impeller.h
crates/session/          会话 trait
crates/hostshell/        命令队列、帧调度、渲染线程
crates/macos/            macOS 窗口
crates/android/          HostActivity、HostView、Choreographer、Vulkan
crates/quickjs/          quickjs-ng 会话。没有源码时 NotLinked
packages/solid-renderer/ 接到 uicore 的 universal 运行时。不做布局
packages/solid-ui/       Button、Field、Dialog、theme、overlay
packages/devtools/       应用内调试面板。数据来自 hostshell 的 DiagnosticsHub
packages/solid-router/   路径、栈、RouterView
packages/*               clipboard、linking、app-state、file-system、async-storage、net、form、i18n、permissions
kotlin/                  Gradle 根。路线暂缓
apps/gallery-solid/      Solid 应用。dist/quotes.js 由 just gallery-dist 本地生成，不进 git
```

依赖只向下：gallery → solid-ui / solid-router → solid-renderer → quickjs 会话 → uicore → protocol。宿主 → hostshell → session、engine。`engine` 不依赖 `uicore`。`uicore` 不包含 QuickJS、AppKit 或 Impeller 头文件。语言包不算坐标。选型表在 `AGENTS.md`。一个可执行文件只链一种会话。

`crates/protocol`、`crates/uicore`、`crates/engine`、`crates/macos` 不手写 C 或 Objective-C。QuickJS 的 C 只在 `crates/quickjs`。变更缓冲由 `crates/quickjs/abi.yml` 生成。

## 一帧

平台线程入队，渲染线程做命中、冲刷、布局、录制和上屏。命中用上一帧的几何。监听器只改状态。

```text
pointer → hit-test → 监听器写信号
vsync → 冲刷 → layout → driver tick → 脏了再 layout → onLayout → 脏了再 layout 一次 → record → present
```

`frame` 里先排空 job，再 `onFrame`，再扫掉本轮摘下且没有挂回的子树，然后布局。`ListDriver` 和 `ScrollController` 的 tick 在布局之后、`onLayout` 之前。树不脏且可见集没变时，沿用上一份层树，不调用 `onLayout`。driver 还有未测完的行高，或树上还有运动，会话再要一帧。`requestFrame` 只再要恰好一帧。脚本错误清掉这一次的再要帧。

`layer_epoch` 未变、缩放相同、`needs_present` 为假则跳过 `submit_layers` 和 `present`。为真则 `PresentCached`。`Resumed` 与 `Invalidate` 置 `needs_present`。`Hidden`、`Paused`、`MemoryWarning` 调用 `Engine::trim`，GPU 进入 `Flush`。`Dead` 取消这一帧。present 连续失败从第 3 次起退避。`BadScene` 不进退避，也不重建 Vulkan 链。

macOS 与 Android 宿主始终打开应用内 devtools。面板经 `registerTopLayer` 挂在渲染器根 stack 上，不往 `DrawList` 加调试命令。日志是调用点上的 `tracing` 事件，由 `hostshell` 过滤后分发到控制台、xlog 和 devtools。超预算只在进入和恢复各打一条。全量计时在 `FrameSample`。

## 绘制

进程内没有场景字节。命令是 `Fill`、`Rect`、`Text`、`Image`，以及成对的 `PushClip` / `PushOpacity` / `PushTransform` / `Pop`。不平衡是 `BadScene`。`protocol` 不校验。

`Tree::record` 交出扁平 `DrawList`（场景坐标）和 `LayerTree`。picture 内坐标相对该 picture 的原点。一段 picture 属于一个 owner：根，或本帧开层的 `Scroll`、半透明 `Opacity`、非单位 `Transform`、子树里还有开层节点的 `Clip`，以及 blur、backdrop、非 `SrcOver` 的 blend。没有这些的 `Clip` 把 `PushClip` 留在父 picture 里。`ListDriver` 把行标成 picture owner 后，每行单独一张 picture。滚动偏移在 `Offset` 层上，不因此重建 picture。行 picture 的 `Offset` 是场景坐标，命令原点是 `(0, 0)`。

文字命令带字号、`max_width`、颜色和段落样式。渲染引擎按命令里的字号绘制，不乘 `point_scale`。`point_scale` 只乘在根 builder 上。

## UI 核心

`uicore::Tree`。节点 id 是槽位加代数，`to_raw` 是 `u64`。`remove` 摘子树但留着 Taffy，同一轮再 `insert` 不 `revive`。轮末仍无父节点才 `discard_detached`。`detach` 不进销毁名单。`discard` 要求没有父节点。

几何交给 Taffy。`layout_point` 是场景坐标，不含滚动和变换。`set_scroll_offset` 只标绘制。

| 对象 | 布局 | 绘制 |
|---|---|---|
| `Row` `Column` `Grid` `Padding` `Align` `Stack` `Sized` | flex 或 grid | 容器本身不绘制 |
| `Decorated` | 一列，间距 0 | 边框盒一条矩形 |
| `Clip` | 一格 grid | 开层则 `Clip` 层，否则父 picture 里 `PushClip` |
| `Opacity` | 一格 grid | alpha 1–254 开层。255 并入父 picture。0 跳过子树 |
| `Scroll` | 滚动轴 `max_content`，交叉轴 `minmax(0, 1fr)` | `Clip` + `Offset`。无行 owner 时一张 child picture |
| `Transform` | 一格 grid | 非单位变换（含纯平移）开层。命中走逆变换 |
| `Text` `Image` | measure 回调 | 段落或图片命令 |
| `PointerRegion` | 等于子节点 | 不绘制 |

`Scroll` 的轴、认领、overscroll 和 fling 以 `uicore/src/tree/scroll.rs`、`uicore/src/motion.rs` 为准。缺省 `Both` + `Content`。`ScrollArea` 缺省 vertical + `Always`。`VirtualList` 固定 vertical。单轴丢掉跨轴增量。`Row` / `Column` 的 `max_size` 是父级 100%；`Scroll` 的直接子节点在滚动轴上把这道上限改回 auto。`Text`、`Image`、`Sized` 不收缩。

脚本能读到的滚动数据：跟手后的 offset、`onDragBegin` / `onDragBy` / `onDragEnd`、`onScrollState` 的相位与速度、`overscroll`、`scrollOffset`、`limits`、`rect`、`flingTarget`。`stopScroll` 摘掉惯性越界弹簧，不把越界量写成 0。`animate` 的 `overscrollY` 可以把越界量收到 0。拖动锁定时摘掉该节点正在跑的越界弹簧。越界弹簧的静止位置仍是 0，结束时写成 0。负的 `overscroll.y` 是视口顶部的缝：录制用 offset 加 overscroll，内容下移后缝是透明的。`PullMode` 的 Edge / Overscroll 提交仍在 `uicore`，它不把缝停住。惯性样本夹到极限且没有父级接着滚时，剩余速度装上越界弹簧；像素对齐只改 offset。tau 仍是 500。

命中从最后一个子节点往前。宽或高为 0 跳过。`Text` 和 `Image` 命中盒子就收下。容器把命中交给子节点。点必须落在每个祖先盒子内。点击和拖拽在 `Tree::pointer`：8 点以内抬起是 click，超过则锁定声明了拖动的区域，否则锁定还能滚的 `Scroll`。

测试排版是 `MonoShaper`。宿主测量和绘制共用 `Engine` 的 `Typography`。

## 渲染引擎

`engine::Engine` 只读 `LayerTree`，不读渲染树。没有 SDK 时 `NotLinked`。`Typography`、`Painter`、`PictureCache`、`TextureStore` 各管一类资源。句柄按 `impeller.h`：`New` 且写明 `Release` 的用完释放一次；缓存在对象上的结果不要 `Release`。`register_font` 的 `on_release` 必须非空。

空文字、缺必选图、非正缩放是 `BadScene`。图片是紧排 RGBA8888，`source` 以 `asset:` 开头。

## 宿主

宿主入队、在 vsync 上要帧、把 `LayerTree` 交给引擎并上屏。不做命中，不保存 UI 状态。会话 trait 在 `crates/session`。三端差异在 `docs/platform-hosts.md`。iOS 宿主还没写。

## 运行时会话

quickjs-ng 持有 `JSRuntime` 和一棵 `Tree`。`glue.c` 注册 `core._mutate`、`_query`、`_effect`，以及监听、driver 和宿主模块。具名方法在 `core.js`，编号在 `abi.yml`。句柄是 `u64`，`0` 表示没有节点。没有源码时 `open` 返回 `NotLinked`。

`@impasto/<pkg>/host` 由 `JS_NewCModule` 物化。未注册是链接期错误。大字节（≥4KB）进会话 slab，由 quickjs 的 realloc 钩子回收。

`boot` 独占 `core.onFrame` 和 `core.onLayout`：先 `flush`，再跑监听。抛错的 `onFrame` 监听会从这张表里摘掉。

Kotlin 会话还没接上原生树。见 `docs/refactor-plan.md`。

## 渲染器

`packages/solid-renderer` 的 `hostRendererOptions` 不导入 Solid。宿主标签是 `row`、`column`、`stack`、`align`、`padding`、`decorated`、`pointer`、`clip`、`opacity`、`scroll`、`transform`、`image`、`sized`。`Text` 是组件，不是标签。节点是 `{ id }`。渲染器不算坐标，也不做命中。

`build.mjs` 用 `@solidjs/babel-plugin`（`moduleName: "@impasto/solid-renderer"`、`generate: "universal"`）。依赖 `solid-js` 与 `@solidjs/universal` 的 `2.0.0-rc.17`。

## 控件

语言包只包模板、信号和渲染回调。`Button` 是 `PointerRegion`、`Padding`、`Decorated`、`Text`。`ScrollArea` 和 `VirtualList` 的窗口与夹紧在 `uicore` 的 `ListDriver` 和 `ScrollController`。把下拉缝停在阈值高度、等业务结束再收回，是 `solid-ui` 的 `RefreshControl`：它读 `overscroll`，过阈值时先 `stopScroll` 再把缝 `animate` 到阈值，结束后 `animate` 到 0。指示器是滚动内容列里绝对定位的第一个孩子，`inset` 顶在内容原点之上；视口把它裁掉，负的 `overscroll` 把整段内容下移后才露出来。`VirtualList` 的 `onRefresh` 仍走 `PullMode::Edge`。语言侧在条目变化时推 `{ key, type, sticky }`，用 `noteRow` 登记行节点。可见集变了才经 `onDriver` 交出 `Detach` / `Adopt` / `Set` / `Place`。`createSlot` 仍是语言侧的 owner。`Field` 绑定 `uicore` 的 `TextEditor`，偏移是 UTF-16，编辑按字形簇。

## 应用

`apps/gallery-solid/src/quotes.tsx`。路由是 `/watchlist`、`/market`、`/trade`、`/profile`、`/search`、`/stock/:symbol`、`/order/:symbol`。自选列表打开 `VirtualList` 的 `recycle`。`just gallery-dist` 在源码更新后重编 `dist/quotes.js`。`cargo test` 不跑 node。quickjs 测试缺产物时直接失败。
