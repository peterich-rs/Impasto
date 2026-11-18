# 架构

当前代码的结构。未落地的事在 `docs/refactor-plan.md`。和代码不一致时，先改已经跑起来的那一边。

## 仓库

原生代码在 `crates/`，用 Cargo。JS 是仓库根的 pnpm workspace（`packages/*`、`apps/*`）。Kotlin 只用 `kotlin/` 里的 Gradle。根上的 `settings.gradle.kts` 没有 `include`。

```
crates/protocol/         DrawList、LayerTree
crates/uicore/           渲染树。Taffy、命中、手势、焦点
crates/engine/           渲染引擎。读 LayerTree，调用 impeller.h
crates/js-host/          宿主交换契约。仅 std
crates/session/          会话 trait。原名重导出 js-host 的自有数据类型
crates/shell/        命令队列、帧调度、渲染线程
crates/macos/            macOS 窗口
crates/android/          HostActivity、HostView、Choreographer、Vulkan
crates/quickjs/          quickjs-jit 0.14.2 会话
packages/solid-renderer/ 接到 scene 与 reactive 的运行时。不做布局
packages/solid-ui/       View、Row、Column、Stack、Grid、Text、Image、Button、Field、Dialog 等作者组件
packages/devtools/       应用内调试面板。数据来自 shell 的 DiagnosticsHub
packages/solid-router/   路径、栈、RouterView
packages/*               clipboard、linking、app-state、file-system、async-storage、net、form、i18n、permissions
kotlin/                  Gradle 根。路线暂缓
apps/gallery-solid/      Solid 应用。dist/quotes.js 由 just gallery-dist 本地生成，不进 git
```

依赖只向下：gallery → solid-ui / solid-router → solid-renderer → quickjs 会话 → uicore → protocol。宿主 → shell → session、engine。`session` → `js-host`。`js-host` 只有 std，不依赖 session、uicore、quickjs、engine 或平台库。`engine` 不依赖 `uicore`。`uicore` 不包含 QuickJS、AppKit 或 Impeller 头文件。语言包不算坐标。选型表在 `AGENTS.md`。一个可执行文件只链一种会话。

`crates/protocol`、`crates/uicore`、`crates/engine`、`crates/macos` 不手写 C 或 Objective-C。QuickJS 的 Rust 绑定在 `crates/quickjs/src/linked/rt`。`abi.yml` 生成 Rust、JavaScript 与 TypeScript 常量。

## 一帧

平台线程入队，渲染线程做命中、冲刷、布局、录制和上屏。命中用上一帧的几何。监听器只改状态。

```text
pointer → hit-test → 监听器写信号
vsync → 冲刷 → layout → driver tick → 脏了再 layout → onLayout → 脏了再 layout 一次 → Canvas paint → record → present
```

`frame` 里先排空 job，再 `onFrame`，再扫掉本轮摘下且没有挂回的子树，然后布局。`scene.createElement` 创建的节点在 owner 清理时仍未挂载，也加入这次回收。`ListDriver` 和 `ScrollController` 的 tick 在布局之后、`onLayout` 之前。树不脏且可见集没变时，沿用上一份层树，不调用 `onLayout`。Canvas paint 在最终布局之后、`Tree::record_layers` 之前。driver 还有未测完的行高，或树上还有运动，会话再要一帧。`requestFrame` 只再要恰好一帧。脚本错误清掉这一次的再要帧。

`layer_epoch` 未变、缩放相同、`needs_present` 为假则跳过 `submit_layers` 和 `present`。为真则 `PresentCached`。`Resumed` 与 `Invalidate` 置 `needs_present`。`Hidden`、`Paused`、`MemoryWarning` 调用 `Engine::trim`，GPU 进入 `Flush`。`Dead` 取消这一帧。present 连续失败从第 3 次起退避。`BadScene` 不进退避，也不重建 Vulkan 链。定时器 `Wake` 在已有 vsync token 时不抢跑这一帧，也不取消那次回调；`Resumed` 会丢掉卡住的 token 再武装一次。

macOS 与 Android 宿主打开应用内 devtools。面板经 `registerTopLayer` 挂在渲染器根 stack 上，复用现有布局和绘制。日志经 `shell` 过滤并分发到控制台、xlog 和 devtools。性能页进入时读取一次最近帧，之后由「刷新快照」更新。布局页按需扫描上一轮布局。连续溢出扫描由 `debugSetScan` 显式开启。

`FrameSample.workMs` 是渲染线程进入当帧至 `present` 返回的墙钟时间，`budgetMs` 是显示刷新间隔。子阶段可嵌套，例如 `scriptMs` 和 `layoutMs` 都包含在会话耗时中。Android 系统 trace 同时记录调用和跳过 `present` 的帧。采集入口见 [Android 帧性能](platform-hosts.md#android-帧性能)。

## 绘制

进程内没有场景字节。命令是 `Fill`、`Rect`、`Text`、`Image`、`Canvas`，以及成对的 `PushClip` / `PushOpacity` / `PushTransform` / `Pop`。不平衡是 `BadScene`。`protocol` 不校验。`Command::Canvas` 携带 `CanvasList`，坐标相对该节点的内容盒。Canvas 可在挂载时启用 `deferWhenOffscreen`。`uicore` 结合视口、祖先裁剪、滚动与越界判断内容盒是否完全在外。旋转、缩放和会扩展绘制范围的效果使用保守判断。QuickJS 在离开视口后仅订阅内部 revision，保留上一张成功录制的 picture，回到视口再用最新数据录制。

运行时用 `Tree::record_layers` 组出完整的 `LayerTree`。层拓扑变化、重新挂载和 picture owner 变化沿祖先链使记录摘要失效。其余子树复用层与 picture owner 摘要及已有 picture。`Tree::record` 同时生成扁平 `DrawList`，使用场景坐标。picture 内坐标相对该 picture 的原点。一段 picture 属于一个 owner：根，或本帧开层的 `Scroll`、半透明 `Opacity`、非单位 `Transform`、子树里还有开层节点的 `Clip`，以及 blur、backdrop、非 `SrcOver` 的 blend。没有这些的 `Clip` 把 `PushClip` 留在父 picture 里。`ListDriver` 把行标成 picture owner 后，每行单独一张 picture。滚动偏移在 `Offset` 层上，不因此重建 picture。行 picture 的 `Offset` 是场景坐标，命令原点是 `(0, 0)`。

文字命令带字号、`max_width`、颜色、段落样式，以及可选的 spans。每个 span 有自己的颜色、字重、字体、下划线和删除线。`gradient` 存在时前景用 `protocol::Gradient`，unit square 映射到该 span 在段落局部坐标里的行盒并集；下划线和删除线仍用 `color`。只改颜色或渐变时标绘制，不重新测量。渲染引擎按命令里的字号绘制，不乘 `point_scale`。`point_scale` 只乘在根 builder 上。

## UI 核心

`uicore::Tree`。节点 id 是槽位加代数，`to_raw` 是 `u64`。`remove` 摘子树但留着 Taffy，同一轮再 `insert` 不 `revive`。轮末仍无父节点才 `discard_detached`。`detach` 不进销毁名单。`discard` 要求没有父节点。

几何交给 Taffy 0.15.1 的低层接口，布局和像素舍入仍由 Taffy 执行。测量回调经 `compute_leaf_layout` 调用文字与图片测量。已布局的固定宽高绝对定位盒，以及固定宽高、basis auto、shrink 0 且无需基线对齐的 flex 子盒，可以重放父布局给过的 `LayoutInput`。局部输出保持一致时只回写这棵子树。尺寸、基线、滚动溢出和定位输出变化时从根重算。纯文字变化先用已记录的查询重新测量，输出相同时沿用祖先几何，更新文字内容与绘制。节点显式使用相对定位，绝对定位的参照仍是直接父节点。`layout_point` 是场景坐标，不含滚动和变换。`set_scroll_offset` 只标绘制和 Canvas 可见性。

`tree::node::Kind` 是 `View`、`Stack`、`Scroll`、`Text`、`Image`。构造预设集中在 `style::defaults`。普通 View 默认 column、min 0、grow 0、shrink 0、basis auto、无隐式 max；填满视口和剩余空间由应用显式声明。Rust 的 row、column、padding 等构造方法仍提供预设，不再夹紧到父级的 100%。

| 对象 | 布局 | 绘制 |
|---|---|---|
| `View` | 默认 flex column，可声明 flex、grid 或 block | 同节点背景、边框、outline 与效果 |
| `Stack` | 一格 grid，子节点重叠 | 与 View 共用样式与效果 |
| `Scroll` | 滚动轴 `max_content`，交叉轴 `minmax(0, 1fr)` | `Clip` + `Offset`，滚动不重建内容 picture |
| `Text` `Image` | measure 回调 | 段落或图片命令 |

`Scroll` 的轴、认领、overscroll 和 fling 以 `uicore/src/tree/scroll.rs`、`uicore/src/motion.rs` 为准。缺省 `Both` + `Content`。`ScrollArea` 缺省 vertical + `Always`。`VirtualList` 固定 vertical。单轴丢掉跨轴增量。

`style/` 使用类型化声明和来源。优先级为 component、classes、inline、imperative、animation，同来源的独立句柄按创建顺序合并。`StyleSourceId` 由 Tree 分配并记录所属节点，释放与重用节点不会让旧句柄修改新节点。提交先校验完整候选，再替换来源；失败保留已接受的来源和几何。每次求值从节点初始样式重建，clear 撤销当前来源，null 阻断低优先级与继承并恢复初始值。

`Tree::flush_styles` 在布局前提交 Taffy 与展示属性，可继承字段沿父链传递。颜色只标绘制；改变字体度量才重新测量。`display: none` 跳过绘制、命中和语义。环境依赖覆盖所有来源的 rem、间距与主题 token；hover、active、focus、disabled 来自树上的实际状态。类规则由 `lru` 缓存，命中提升顺序并按条数和估算字节预算淘汰。依赖刷新当前仍扫描已登记的样式节点。`packages/solid-ui/src/theme-bind.ts` 将默认主题的色板、字号、字重、间距、圆角和 locale 方向经 `scene.setStyleEnvironment` 提交给 `Tree::set_style_environment`；更新保留窗口尺寸。根节点继承环境方向，局部 `layoutDirection` 与 `writingDirection` 可以覆盖或用 null 重置。主题作用域与按依赖建立的节点索引仍未落地。

背景、文字与子节点置于同节点 opacity、transform、blur、backdrop 和 blend 的作用域中；内容裁剪与滚动 offset 在其内侧。outline 不占布局空间。仅改变已有层的透明度或变换值时可以复用 picture。树仍待布局时 `record` 返回 `Error::LayoutRequired`，调用者须先布局。

`just gen` 生成 TypeScript 样式类型、QuickJS 属性表与工具类清单；`just style-check` 检查一致性。脚本的 `setStyle`、`patchStyle`、`clearStyle`、`setProp("style")` 和 class 输入由生成的 `StyleValueRecord` 传入。对象、片段数组与复杂值先复制为拥有型数据，再借用 Tree 校验提交。同步几何查询即使没有待处理 mutation，也会冲刷样式并完成所需布局。

脚本能读到的滚动数据：跟手后的 offset、`onDragBegin` / `onDragBy` / `onDragEnd`、`onScrollState` 的相位与速度、`overscroll`、`scrollOffset`、`limits`、`rect`、`flingTarget`。`stopScroll` 摘掉惯性越界弹簧，不把越界量写成 0。`animate` 的 `overscrollY` 可以把越界量收到 0。拖动锁定时摘掉该节点正在跑的越界弹簧。越界弹簧的静止位置仍是 0，结束时写成 0。负的 `overscroll.y` 是视口顶部的缝：录制用 offset 加 overscroll，内容下移后缝是透明的。`PullMode` 的 Edge / Overscroll 提交仍在 `uicore`，它不把缝停住。惯性样本夹到极限且没有父级接着滚时，剩余速度装上越界弹簧；像素对齐只改 offset。tau 仍是 500。

命中按 zIndex 与同层顺序从前往后。`Text` 和 `Image` 命中自身盒子；容器把命中交给子节点。overflow 可见时允许命中祖先盒子外的后代，Clip 与 Scroll 才在视口边界拒绝。变换先把查询点逆变换到局部坐标。点击和拖拽在 `Tree::pointer`：8 点以内抬起是 click，超过则锁定声明了拖动的区域，否则锁定还能滚的 `Scroll`。

`PointerRegion` 缺省语义角色是 `Generic`；未禁用时提供 `Activate` 动作。按钮角色由 `SemanticsHint` 显式声明。

测试排版是 `MonoShaper`。宿主测量和绘制共用 `Engine` 的 `Typography`。
Taffy 复用既有文字尺寸时，也复用同一份测量的第一基线。Min-content 用同一无界段落读取固有宽度、行高和基线。

## 渲染引擎

`engine::Engine` 只读 `LayerTree`，不读渲染树。没有 SDK 时 `NotLinked`。`Typography`、`Painter`、`PictureCache`、`TextureStore` 各管一类资源。句柄按 `impeller.h`：`New` 且写明 `Release` 的用完释放一次；缓存在对象上的结果不要 `Release`。`register_font` 的 `on_release` 必须非空。

空文字、缺必选图、非正缩放是 `BadScene`。图片是紧排 RGBA8888，`source` 以 `asset:` 开头。

## 宿主

宿主入队、在 vsync 上要帧、把 `LayerTree` 交给引擎并上屏。不做命中，不保存 UI 状态。会话 trait 在 `crates/session`。三端差异在 `docs/platform-hosts.md`。iOS 宿主还没写。
Android 用 `AccessibilityManager` 的监听通知会话启停语义快照。重新开启时会发布当前树的快照。

## 运行时会话

`crates/quickjs` 使用根 `Cargo.toml` 钉住的 quickjs-jit 0.14.2，依赖名是 `rquickjs`。`linked/rt::Engine` 拥有唯一的 `Runtime`、`Context` 和 `Host`；`uicore::Tree`、`reactive::Graph` 与 VM 都留在渲染线程。Rust 绑定注册 `scene`、`reactive`、`#reactive` 和能力模块。`linked/scene.rs` 把属性名映射到 `abi.yml` 的字段号，`linked/mutation.rs` 应用树更新。`scene.js` 提供具名查询、命令与插入范围归一化。节点是 `NodeId::to_raw()`，`0` 表示没有节点，没有全局 `core`。启动先绑定原生入口，再依次执行 `abi.js`、`scene.js`、`prelude.js` 和 ESM `reactive.js`，捕获并删除 `__reactive` 后执行应用并排空 jobs。

`reactive::Graph` 使用带 generation 的 arena 和双向边。进入用户闭包和 cleanup 前结束 arena 借用，JS 值由 `linked/reactive.rs` 的 `Persistent` 持有和释放。signal 写入只标脏，`batch` 同步冲刷。`JsSession::frame` 在帧监听前、帧监听后和布局监听后冲刷图，空队列跳过。帧监听写入的信号在第一次布局前提交。`tick_motions` 在这些调用之前。`boot` 分发帧与布局监听。抛错的帧监听会从监听表摘掉。`Show`、`Switch`、`Match`、`For` 和 `Errored` 在 `packages/reactive`，调用同一张原生图。

能力模块的运行时 specifier 是包入口 `@impasto/clipboard`、`@impasto/linking`、`@impasto/file-system`、`@impasto/async-storage`、`@impasto/app-state`、`@impasto/permissions` 和 `@impasto/devtools`。`ModuleDef` 从 Rust 的 `Module` 方法表注册导出，`cargo run -p shell --bin module-stubs` 从同一份表和方法签名生成各包的 `src/native.d.ts`。只有在 `ModuleExports::events` 声明事件的模块才导出 `addEventListener` 和 `removeEventListener`，监听方法只接受声明的事件名。`Linking` 声明 `url`，`AppState` 声明 `change` 和 `focus`。`Clipboard`、`Linking` 和 `AsyncStorage` 是 QuickJS 直接注册的具名对象。其余能力包的 TypeScript 入口使用生成的声明组合公开 API。Node 测试统一用同目录的 `native.stub.ts`。

CLI 按 `app.toml` 的目标权限生成 `app.modules`，宿主据此限制会话的 `ModuleRegistry`。未注册的能力在链接时报错。原生绑定用 `ArrayBuffer` 持有大字节（≥4KB）的 Rust `Vec<u8>`，较小字节使用 `Uint8Array`。公开的 `FileSystem.read` 总是返回 `Uint8Array`，对大字节创建共享同一缓冲区的视图。

`quickjs::compile` 的脚本和模块路径都生成 IQJS envelope，包含引擎版本与模式，返回只有 `compile` 能构造的 `CompiledBytecode`。`bytecode.rs` 在加载前检查 magic、版本、模式标签和非空载荷；这些检查不能证明载荷是对应类型的字节码。进程内加载走 `open_compiled_with_io`。磁盘上的 `app.bin` 走 `open_trusted_bytecode_with_io`，调用者保证字节来自本工具链的 `compile`，该入口是 `unsafe`。脚本用 `JS_ReadObject` 与 `JS_EvalFunction`，模块用 `Module::load`。`open_program_with_io` 只加载源码。macOS 的 `.app` 和 Android 的资产都可携带 `app.bin`。内存上限 16 MiB，栈上限 8 MiB，eval 限时 1000 ms，其余 turn 为 128 ms，job drain 最多 32 轮。文件模块先按应用根做词法归一化，`..` 留在根内则接受，越出根才拒绝；`AppLoader` 再检查符号化后的真实路径。单个文件上限 16 MiB。脚本异常的 message 和 stack 分开进入 `DiagnosticsHub`。`Engine::shutdown` 在 VM 存活时先释放响应式图，再清空定时器、监听器及 HTTP 等 Persistent，移除 userdata，最后依次释放 Context 和 Runtime；正常 Drop 与启动失败共用此路径。`run --trace` 输出帧内脚本微秒、非空更新轮次和场景 setter 次数。

Kotlin 会话还没接上原生树。见 `docs/refactor-plan.md`。

## 渲染器

`packages/solid-renderer/runtime/index.ts` 再导出 `scene` 的渲染原语。`src/tree.ts` 只定义数字节点句柄和叶组件的接口。作者标签是 `view`、`stack`、`scroll` 和 `image`，展示属性统一放进 `style`、`class`、`classList`。`Text` 是组件。渲染器不算坐标，也不做命中。Node 组件测试的替身留在 `testing/`；QuickJS 测试验证实际的原生插入、属性写入和控制流。

`tools/pack.mjs` 是应用和 gallery 共用的 Babel/esbuild 配置。`@solidjs/babel-plugin` 2.0.0 使用 `moduleName: "@impasto/solid-renderer"`、`generate: "universal"`。`solid-js` 解析到 `@impasto/reactive`，`scene` 和 `reactive` 保留为运行时模块。构建检查 metafile 和产物，拒绝混入 Solid 调度器与 `@solidjs/universal`。官方 `solid-js` 和 `@solidjs/signals` 2.0.0 用于开发与对照测试。应用可用的 API 见 [@impasto/reactive](../packages/reactive/README.md)。

## 应用命令

`crates/cli` 的二进制是 `impasto`。`create` 只创建 `app.toml`、`package.json`、`lib/main.tsx` 和 `assets/`。`run` 默认 debug，构建并启动窗口或通过 adb 安装启动 Android；文件改变后重新构建、重启。`build bundle` 默认 release 字节码，`--debug` 保留 ESM。`build macos` 在 `build/macos` 生成可启动的 `.app`，包含脚本、资产和 Impeller 动态库；`build apk` 在 `build/android` 生成 Gradle 工程并链 `crates/android`。`build ios` 写出缺失宿主的说明后返回非零。

`analyze` 用内置 TypeScript 检查器检查应用；`test` 在 QuickJS 中执行测试入口，也可传入 ESM 或字节码路径；逻辑测试可以不创建渲染树。`clean` 删除应用的 `build/`、`dist/`。`packages get` 将参数交给 `pnpm install`。`tools/distribute.mjs` 生成可移动发布目录，携带按 `crates/cli/toolchain.pin` 校验的 Node、Babel、esbuild、TypeScript、框架包和原生构建输入。源码检出允许回退到本机 Node，`--verbose` 会打印选用路径。

## 控件

语言包只包模板、信号和渲染回调。`Button` 使用一个 View 承载布局、背景、手势、焦点和语义，内部是 Text 与可选图标。`bindStyle` 为组件默认值、class 和 inline 分配独立来源，响应信号并在 owner 清理时只移除自己创建的来源。Button、Field 和浮层子目标的 class 与 inline 可覆盖 component 来源的默认值，StyleInput 的片段数组与 null 直接交给原生样式解析。浮层在关闭时仍跟踪样式 getter，挂载时应用已保存的快照；退出完成后撤销 panel ref，owner 清理不会重复发送 null。`Pressable` 与 `Button` 共用 `bindPressable`，先执行内部交互再调用作者事件；disabled 同步 focusable、语义和原生状态变体。View 的作者 role、label 和 hint 撤销时会提交空语义提示，恢复原生推断。`ScrollArea` 和 `VirtualList` 的窗口与夹紧在 `uicore` 的 `ListDriver` 和 `ScrollController`。把下拉缝停在阈值高度、等业务结束再收回，是 `solid-ui` 的 `RefreshControl`：它读 `overscroll`，过阈值时先 `stopScroll` 再把缝 `animate` 到阈值，结束后 `animate` 到 0。指示器是滚动内容列里绝对定位的第一个孩子，`inset` 顶在内容原点之上；视口把它裁掉，负的 `overscroll` 把整段内容下移后才露出来。`VirtualList` 的 `onRefresh` 仍走 `PullMode::Edge`。语言侧在条目变化时推 `{ key, type, sticky }`，用 `noteRow` 登记行节点。可见集变了才经 `onDriver` 交出 `Detach` / `Adopt` / `Set` / `Place`。`createSlot` 仍是语言侧的 owner；每行的 View 包装承载原生 children 归一化，render 可返回多个节点、文字或访问器。`Field` 绑定 `uicore` 的 `TextEditor`，偏移是 UTF-16，编辑按字形簇。

## 应用

`apps/gallery-solid/src/quotes.tsx`。路由包含 `/stocks`、`/watchlist`、`/market`、`/trade`、`/profile`、`/search`、`/stock/:symbol`、`/order/:symbol`。行情推进和发布频率由应用管理。首页通过稳定的证券分组把行更新分摊到多个帧，并将迷你图错开发布。自选列表打开 `VirtualList` 的 `recycle`。`just gallery-dist` 在源码更新后重编 `dist/quotes.js`。`quickjs` 的原生组件测试用 Node 编译 TSX fixture，再在真实会话中检查。Gallery 测试消费本地 ESM 产物，缺失时直接失败。
