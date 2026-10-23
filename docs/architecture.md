# 架构

这是 monorepo 和各层内部的结构。工作约定以 `AGENTS.md` 为准。这里和代码不一致时，先改已经跑起来的那一边。

## 仓库怎么切

一种语言一个绑定，一套 UI 核心。我们写的原生代码用 Cargo 编成 Rust。Kotlin 只用 `frameworks/kotlin-ui` 里的 Gradle。JS 以后可以有只包含前端包的 `pnpm-workspace.yaml`，不把 Kotlin 和 `third_party/solid` 算进产品构建。根上的 `settings.gradle.kts` 继续保持没有 `include`。

场景格式、渲染树、渲染引擎、会话、`hostshell`、macOS 宿主和 Android 宿主已经是 Rust。不要再往 `protocol`、`core`、`engine`、`hosts/macos` 加 C 或 Objective-C。Android 的 NDK 调用留在 `hosts/android`。QuickJS 的胶水 C 只在 `runtimes/quickjs`。给 Kotlin 用的 C ABI 还没写。

```
protocol/                     版本 1 字节和 DrawList
core/                         渲染树。Taffy、等宽排版、命中、click、drag、焦点
engine/                       渲染引擎。只读 DrawList，调用 impeller.h
session/                      会话 trait。metrics、pointer、frame、wants_frame、lifecycle
hosts/macos/                  macOS 窗口。指针入队、CVDisplayLink、UIFW_DUMP
hosts/hostshell/              命令队列、帧调度、渲染线程
hosts/android/                GameActivity、AChoreographer、Vulkan。库名 `uifw`
hosts/ios                     还没有。上游预构建矩阵没有 iOS 列
runtimes/quickjs/             quickjs-ng 会话。包名 quickjs
frameworks/solid-renderer/    版本 1 编码器，以及接到 core 的 universal 运行时
frameworks/solid-ui/          Button、ScrollArea、Field、Switch、Dialog
frameworks/kotlin-ui/         Gradle 根。DSL、控件、以后的 cinterop
  library/
apps/gallery-solid/           Solid 应用。源码 counter.tsx，产物 dist/counter.js
apps/gallery-kotlin/          Kotlin 应用，由 kotlin-ui 的 settings 指向
third_party/                  只读对照和本机 SDK
```

`core` 的 C 头还没有。Kotlin 的 cinterop 等这组树操作跑稳再生成。

依赖只允许向下：

```text
gallery-solid ──► solid-ui ──► solid-renderer ──► quickjs 会话 ──► core ──► protocol
gallery-kotlin ──► kotlin-ui ──────────────────────────────────────┘
hosts/macos ──► hostshell ──► session ──► core ──► protocol
hosts/macos ──► engine ──► protocol
hosts/android ──► hostshell ──► engine ──► protocol
hosts/macos ──► quickjs 会话          `--js` 时。`just host` 不加载会话
hosts/android ──► quickjs 会话        加载 assets 里的 `counter.js`
```

`core` 不包含 QuickJS、Kotlin、AppKit 或 Impeller 的头文件。它通过 crates.io 依赖 Taffy。Impeller 排版由宿主实现 `Shaper` 并调用 `engine`，测试构建不链它。`engine` 不包含 `core`。语言包不实现测量和布局。选型表在 `AGENTS.md` 的「优先复用已有实现」。

两个产品二进制以后都链同一套 `core` 和 `engine`：macOS 上 QuickJS 宿主是 `hosts/macos`，Kotlin/Native 宿主是 `frameworks/kotlin-ui` 编出来的可执行文件。宿主按操作系统写，会话按语言写。一个可执行文件只链一种会话。

## 一帧

平台线程入队，渲染线程做命中、冲刷、布局、录制和上屏。macOS 与 Android 都走这条路径。命中测试用上一帧已经算好的几何。监听器里不布局。布局、录制、上屏都在 vsync 上。

```text
Host            Session           Core              Engine
  |                |                |                  |
  | ① pointer      |                |                  |
  |───────────────►| ② hit-test     |                  |
  |                |───────────────►|                  |
  |                | ③ 区域 id      |                  |
  |                |◄───────────────|                  |
  |                | ④ 监听器改状态 |                  |
  |                |                |                  |
  | ⑤ vsync        |                |                  |
  |───────────────►| ⑥ onFrame 冲刷 |                  |
  |                | ⑦ 脏时 layout  |                  |
  |                |───────────────►|                  |
  |                | ⑧ DisplayList  |                  |
  |                |◄───────────────|                  |
  | ⑨ submit 列表  |                |                  |
  |──────────────────────────────────────────────────►|
  | ⑩ present      |                |                  |
  |◄──────────────────────────────────────────────────|
```

`frame` 的顺序是排空 job、`onFrame(time_ms)`、再排空一轮、布局、录制。`boot` 登记的 `onFrame` 先 `flush`，再跑监听。树不脏就沿用上一帧的 `DrawList`。job 轮数有上限，超过则记入 `script_error` 并打日志。语义树遍历仍然空着。

## 场景协议

版本 1 把已经算好位置的矩形、文字和图片写成字节，不是布局树。`Row` 不会编码成 `LAYER`。运行中的界面不使用这套字节，渲染引擎读的是内存里的 `DrawList`。Kotlin 和 TypeScript 各有一份编码器，输出必须和 `counter.bin` 相同。

进程内的绘制结果是 `protocol::DrawList`。命令是 `Fill`、`Rect`、`Text`、`Image`、`PushClip`、`PushOpacity`、`PushTransform`、`Pop`。`Rect` 带 `corner_radius`，0 是直角。`PushClip`、`PushOpacity`、`PushTransform` 和 `Pop` 成对嵌套，三组开命令共用同一个 `Pop`。渲染引擎校验这套栈，不平衡的列表是 `BadScene`。`protocol` 不校验。坐标是绝对的。文字命令带 `font_size`、`max_width` 和颜色。版本 1 字节没有这些样式，`scene_to_draw_list` 填字号 28、段落宽 280，矩形圆角填 0，也不产生 `PushTransform`。布局用的约束、flex、回调不进字节。`max_width` 是测量时用的行宽，不是排版之后的行宽。文字命令还带段落样式，缺省是左对齐、字重 400、从左到右、不限制行数。版本 1 映射填这些缺省值。`PushTransform` 的顺序是平移、顺时针旋转、等比缩放。旋转单位是弧度，渲染引擎换成度再交给 Impeller。缩放和旋转绕当前原点。命中测试不吃这层变换，仍用布局盒子。

`protocol::scene_to_draw_list` 把 `counter.bin` 转成这份列表，`just host` 用它画第一帧。`core::Tree::record` 交出同一形状的列表。版本 1 编码留给对照编码器。进程内提交用内存里的列表。

## UI 核心

内部是 Rust 渲染对象，类型是 `core::Tree`。给 Kotlin 用的 C 头还没写。节点 id 是槽位加代数，`to_raw` 是 `u64`，高位代数从 1 起、不超过 `2^21 - 1`，这样 JS 的 Number 还能精确表示。删掉的槽位代数对不上，旧 id 得到 `UnknownNode`。代数到顶的槽位不再复用。根是显式的：第一棵挂上的树，或第一个创建的节点；`layout`、`record`、命中都读这个根。`remove` 摘掉子树并释放它的 Taffy 节点，场景节点还在，同一 id 可以再插回去。`discard` 要求节点没有父节点，后序退掉子树槽位，旧 id 变成 `UnknownNode`。空字符串不进 `DrawList`。`layout` 调用 `TaffyTree::compute_layout_with_measure`，再把位置和尺寸写回节点。这些坐标相对父节点。`record` 时加上祖先原点，`DrawList` 里只留绝对坐标。渲染引擎不解释父节点变换。`set_viewport` 和样式、文字的 setter 只标脏，不丢掉已经算好的几何。挂上或摘下节点才会让命中失效。

`Row`、`Column`、`Grid`、`Padding`、`Align`、`Sized`、`Stack`、`Clip`、`Opacity`、`Scroll`、`Transform` 只是 `taffy::Style` 的不同设值：`Display::Flex`、`Display::Grid`、padding、对齐、宽高、gap、flex grow、grid template。`Stack`、`Clip`、`Opacity`、`Transform` 是一格的 grid，子节点放在第 1 行第 1 列。`Scroll` 的轨道是 `max_content`，`overflow` 是 hidden，最小尺寸是 0，这样父约束可以把视口压得比内容矮，子节点仍按完整尺寸布局。`set_scroll_offset` 只标脏，不标 Taffy。下一帧 `layout` 走缓存，坐标不变；`record` 和命中才应用 offset。`set_justify` 写行或列的 `justify_content`，`set_wrap` 写 `flex_wrap`。`set_fill` 把宽高设成父级或视口的 100%，最小尺寸为 0。`set_grow` 写子节点的 `flex_grow`，值必须非负，并标脏。flex 父节点有确定的主轴尺寸时，正的 grow 分到剩余空间。仅有 viewport 时，宽度为 auto 的 flex 根仍按内容收缩，不会出现剩余空间。`Text` 和 `Image` 走 measure 回调。文字回调调用 `Shaper`，图片回调返回固有尺寸。Taffy 不绘制，也不做命中测试。

| 对象 | Taffy 负责 | 绘制 |
|---|---|---|
| `Sized`、`Padding`、`Align`、`Stack`、`Row`、`Column`、`Grid` | 样式和子节点的框 | 容器本身不绘制。`Stack` 的子节点从底到顶画 |
| `Decorated` | 一列子节点，间距 0 | 边框盒一条矩形，圆角记在命令上。命中交给子节点 |
| `Clip` | 与 `Stack` 相同的一格 grid | `PushClip` … 子节点 … `Pop`。盒子为 0 也成对 |
| `Opacity` | 与 `Stack` 相同的一格 grid | `PushOpacity` … 子节点 … `Pop`。子树离屏合成后按 alpha 回贴 |
| `Scroll` | 一格 `max_content` grid。边框盒是视口 | `PushClip` … 子树按 offset 平移 … `Pop`。offset 不进布局 |
| `Transform` | 与 `Stack` 相同的一格 grid | `PushTransform` … 子节点 … `Pop`。命中用未变换的盒子 |
| `Text` | measure 回调给出段落宽高 | `DrawParagraph`。命令带字号、行宽约束、颜色和段落样式 |
| `Image` | measure 回调给出固有尺寸 | `DrawImage` |
| `PointerRegion` | 尺寸等于子节点 | 不绘制。命中后接收手势 |

文字测量是 `Shaper`。测试实现是 `MonoShaper`：一个 Unicode 标量前进 `font_size * 0.5`，行高是 `font_size`，放不下就换行，一行至少一个标量。布局测试因此不链 Impeller。宿主上的实现调用 `Engine::measure_paragraph`。测量和绘制共用同一次排好的段落，颜色在测量时写进前景。从版本 1 场景映射出来的文字，字号 28 和段落宽 280 只在 `protocol` 里填上。

命中测试按绘制顺序从最后一个子节点往前。宽或高为 0 的节点跳过。`Text` 和 `Image` 是 `Opaque`，命中盒子就收下。布局容器、`Decorated`、`Clip`、`Opacity`、`Scroll`、`Transform` 是 `Defer`，只看子节点。点必须落在每个祖先盒子内，所以裁剪框外的子节点不可命中。圆角不改变命中，命中按矩形。`Scroll` 先用视口盒子接纳这个点，再把查询点加上 offset 交给子树。`Transform` 不改变命中。`PointerRegion` 是 `Region`，没有子节点接住时收下自己。

点击和拖拽都在 `core` 的 `Tree::pointer`。按下时做命中测试，记住最近的 `PointerRegion`，并记住最近的 `Scroll` 祖先。`PointerEvent.time_ms` 是宿主事件时间，毫秒。位移不超过 8 点时，按下状态保持，抬起仍在区域内就先 `Release` 再 `Click`。位移超过 8 点且存在 `Scroll` 时，先 `Release` 取消按压，再 `DragBegin`，随后每次移动是 `DragBy`（场景点增量），抬起是 `DragEnd`（场景点/毫秒，取最近两次 `Move`；不足两次则为 0）。没有 `Scroll` 祖先时，超过 8 点仍走原来的按下语义。`Cancel` 在拖拽锁定后补 `DragEnd(0, 0)`。树上不存 JS 函数或 Kotlin lambda。会话在借用结束后分发自己的监听器。监听器里不调用 `layout`。惯性不在这棵树上。

```text
Down 命中
  ├─ Move ≤ 8pt ─────────────── 积累，不转移
  ├─ Move > 8pt 且有 Scroll ─── Release，DragBegin，DragBy…，Up → DragEnd
  ├─ Move > 8pt 且无 Scroll ─── 保持 Press，移出则 Release
  └─ Up 仍在 Press 内 ───────── Release，Click
```

图片固有尺寸在创建 `Image` 节点时传入。宿主解码 RGBA，活到 `submit` 返回。降采样和色彩管理以后换宿主的解码，不改布局对象。

## 渲染引擎

渲染引擎只做光栅，用 Rust 调用 `impeller.h`。它接受 `protocol::DrawList`，再画到宿主给的 drawable 上。它不读渲染树，不解析场景字节，也不处理指针。没有 SDK 时返回 `NotLinked`。以后给别的语言用的符号前缀是 `engine_`。

文字命令自带字号、行宽、颜色和段落样式。渲染引擎按这些字段画，不另设默认字号，也不把字号乘 `point_scale`。测量和绘制共用缓存里的同一段段落。相同文字、字号、宽度、颜色和样式跨帧保留。成功提交结束时丢掉当前列表没有引用的段落和纹理。`register_font` 在下一次提交释放旧显示列表之后清掉段落缓存。上屏失败（`PresentFailed`）保留表面并再要一帧。还没有表面或显示列表（`NotReady`）时丢掉表面，等平台再次给出表面。版本 1 场景没有样式字段，映射时才填 28、280 和缺省样式，矩形圆角填 0。

第一条非 `Fill` 命令之前做一次 `point_scale`。`PushClip`、`PushOpacity` 和 `PushTransform` 也算这类命令，所以 `Save` 记下的矩阵已经含缩放，`Restore` 回到的仍是这份矩阵。`Pop` 不再缩放。半径不大于 0 时矩形走 `DrawRect`、裁剪走 `ClipRect`；更大的半径钳到短边的一半，再走圆角接口。`PushOpacity` 为这一层单独建 paint，颜色是白、alpha 来自命令，`Restore` 之后才释放。`PushTransform` 在 `Save` 之后依次 `Translate`、`Rotate`、`Scale`，`Pop` 走同一个 `Restore`。渲染引擎只用自己的 save 栈：多出来的 `Pop` 不调用 `Restore`；结束时栈非空则整帧 `BadScene`，并且不创建显示列表。失败收尾在释放 builder 之前先 `Restore` 再释放残留的 layer paint。

## 宿主

宿主做三件事：平台事件入队、vsync 上问会话要显示列表、把列表交给渲染引擎并上屏。会话边界是 Rust trait `session::Session`：`metrics`、`pointer`、`frame`、`wants_frame`、`lifecycle`。`wants_frame` 取走「再要一帧」的标志。`frame` 的顺序是处理已经入队的指针、`onFrame(time_ms)`、冲刷指针期间写过的状态、布局、录制、语义树遍历（空）。

macOS 宿主把 `NSEvent` 换成 `core::PointerEvent` 放进队列。先把窗口坐标换进内容视图，再把原点改到左上。AppKit 给出的已经是点，不除 `backingScaleFactor`。`just host` 走 `protocol` 的版本 1 映射，提交一次，不跑会话。`just host-js` 每帧出队并调用会话。宿主不做 click。字体文件字节和 PNG 解码在宿主里，图片像素活到 `submit` 返回。宿主实现 `core::Shaper`，测量时调用同一台 `Engine` 的 `measure_paragraph`。

Android 宿主在 `android_main` 上入队。坐标是物理像素除以 `AConfiguration` 的 density。只保留 pointer index 0。`Pause` / `Resume` 对应 `onPause` / `onResume`。内容区相对窗口的四边写进 `Metrics` 的 inset。会话把它交给 `Tree::set_viewport`，根从左上 inset 排，可用空间扣掉四边。表面销毁要等渲染线程应答之后才 `ANativeWindow_release`。

宿主不知道 `count`、`Button` 或信号。窗口尺寸变化只更新 metrics 并请求下一帧。列表与上次提交相同则不 `submit`。会话申请了下一帧时，画面没变也再登记恰好一帧。

macOS 的窗口和 `CAMetalLayer` 在主线程，`Engine`、会话和 present 在渲染线程。Android 的 `ANativeWindow` 由平台线程持有，Vulkan swapchain 在渲染线程创建和释放。iOS 没有预构建 SDK，宿主还没写。

## 运行时会话

quickjs-ng 会话拥有 `JSRuntime`、一棵 `core::Tree` 和一份区域 id 到 JS 函数的表。`glue.c` 注册全局 `core`。句柄是 `u64`，失败用返回码，`0` 只在这层表示没有节点。解释器按 `quickjs.pin` 的 tag 下载到 `third_party/quickjs`，不抄进 `runtimes/quickjs`。没有源码时 `open` 返回 `NotLinked`。

指针到达时，`Tree::pointer` 先返回被激活的区域，点击时再附上焦点转换。借用结束后会话调用全部匹配的 JS 函数。`Session::input` 只在 `focused()` 有值时派发到那个节点。监听器抛错时，文本留在 `take_script_error`，并经宿主日志打出。这一帧不再派发其余区域。进程不退出。监听器只改树。会话不在这个回调里 `layout`。JS `remove` 先摘下子树，焦点若在这棵子树里先派发 `Blur`，再 `Tree::discard` 退掉槽位，并按返回的 id 释放监听。只 `remove`、不 `discard` 时，同一 id 还能再插回去。

vsync 时先排空 QuickJS job，再跑 `core.onFrame(time_ms)`。`boot` 注册的回调先 `flush`，再跑通过 `onFrame` 登记的监听。回调失败留在 `take_script_error` 并记日志，绘制仍用上一帧。树脏了才 `layout` 再 `record`，否则克隆缓存的 `DrawList`。`requestFrame` 让宿主在 present 之后再登记恰好一帧。全局 `console.log`、`info`、`warn`、`error` 进宿主日志。

`core` 上的函数：`column`、`row`、`stack`、`padding`、`pointerRegion`、`decorated`、`clip`、`opacity`、`scroll`、`transform`、`image`、`text`、`append`、`insert`、`remove`、`parent`、`firstChild`、`nextSibling`、`isText`、`setText`、`setFontSize`、`setColor`、`setGap`、`setPadding`、`setGrow`、`setRadius`、`setAlpha`、`setAlign`、`setSize`、`setImage`、`setMaxLines`、`setEllipsis`、`setTextAlign`、`setFontWeight`、`setTextDirection`、`setLineHeight`、`setFamily`、`setScrollOffset`、`scrollOffset`、`rect`、`setTransform`、`setJustify`、`setWrap`、`setFill`、`requestFrame`、`onClick`、`onPress`、`onRelease`、`onDragBegin`、`onDragBy`、`onDragEnd`、`onFrame`。`decorated` 的半径可以省略，省略时是 0。内建 `image` 的 `source`、宽和高在创建时读入。`setImage` 把这三项一起改掉。`onDragBy` 和 `onDragEnd` 收到两个数，分别是位移增量和速度。`onKey` 收到 `"Backspace"` 这类名字，`onText` 收到已提交串，`onComposing` 收到组合串或 `null`。`measureText` 返回 `{w, h}`，走 `Tree::measure_text`。`setImeEnabled` 和 `setImeText` 只改会话里的两个标志，由宿主在帧末投递给平台。同一张 tap 表保存 click、press、release、drag、focus、blur、key、text、composing，摘掉节点时一起释放。

Kotlin 会话以后做同样的事：cinterop 调那时再生成的 C API，信号用已发布的 `com.lightningkite:reactive`。会话代码放在 `frameworks/kotlin-ui`，不放到 `runtimes/`，这样 Gradle 仍然只有一个根。

## 渲染器

`solid-renderer` 有两种用法。`src/scene.ts` 和 `src/renderer.ts` 把 universal 的调用记成版本 1 字节，标签是 `layer`、`rect`、`text`、`image`，和 `counter.bin` 对字节。`src/tree.ts` 的 `hostRendererOptions` 不依赖 Solid，把同一组方法接到 `core`。宿主标签是 `row`、`column`、`stack`、`padding`、`decorated`、`pointer`、`clip`、`opacity`、`scroll`、`transform`、`image`、`sized`。`pointer` 的 `focusable` 走 `setFocusable`。`radius` 和 `alpha` 创建时读入，之后 `setProperty` 走 `setRadius`、`setAlpha`。内建 `image` 的 `source`、宽和高只在创建时读入。`Image` 组件把这三项一起交给 `setImage`。`grow` 走 `setGrow`。`justify`、`wrap`、`fill` 走 `setJustify`、`setWrap`、`setFill`。`transform` 的 `x`、`y`、`rotate`、`scale` 合成一次 `setTransform`。`onPress`、`onRelease`、`onFocus`、`onBlur`、`onKey`、`onText` 和 `onComposing` 跟 `onClick` 一样登记回调。`onDragBegin`、`onDragBy`、`onDragEnd` 登记在滚动节点上。`Text` 不是标签：universal 会把字符串子节点插到父节点下面，而 `core` 的文字是叶子。`Text` 自己建叶子，再用 effect 调 `setText`、`setFontSize`、`setColor`，以及最大行数、省略号、对齐、字重、方向、行高和 family。`boot` 独占 `core.onFrame`：先 `flush`，再调用 `onFrame(timeMs)` 登记的监听。`requestFrame` 向宿主再要恰好一帧。

编译产物的 `moduleName` 是包入口 `runtime/index.ts`。它用 `createRenderer` 装配 `hostRendererOptions`。渲染器节点是 `{ id }`，不是裸数字。universal 把数字当成文本。`insertNode` 调 `insert`，移动由 Rust 处理。`onClick` 不进显示列表，闭包留在会话那张表里。

渲染器不保存 `x, y`，也不在 TS 里做命中测试。无窗口测试在 `quickjs` crate 里用 `MonoShaper` 断言编译产物上的点击。不要为了测试再写一套 TS 布局。

Kotlin DSL 的形状保持 KiteUI 那种 lambda，落地的是同一套树操作：

```kotlin
column {
  text(count().toString())
  row {
    button("-") { count.set(count() - 1) }
    button("+") { count.set(count() + 1) }
  }
}
```

Kotlin 和 TypeScript 里现有的版本 1 编码器只服务 `counter.bin` 的字节对比。新界面直接改渲染树。

## 基础控件

布局原语只有 `core` 里那一批。语言包给它们薄薄的函数或组件，参数是样式和子节点，不写尺寸计算。

`Button` 写在 `frameworks/solid-ui`。结构是：

```text
PointerRegion      press / release / click
  Padding
    Decorated      抬起是按钮底色，按下换成按压色，圆角 6
      Text         标签
```

按下时控件把 `Decorated` 的填充色改成按压色。抬起时先恢复底色，再调用应用传入的 `onClick`。`pressed` 是控件自己的信号，不是渲染对象的字段，也不是场景字节里的字段。

`Decorated` 按子节点定尺寸，并录一条盖住边框盒的矩形。圆角记在 `Rect` 命令上，不改变命中。按钮外层内边距是 8。`Tree::pointer` 交出 `Press`、`Release`、`Click`，以及滚动锁定后的 `DragBegin`、`DragBy`、`DragEnd`。`set_viewport(x, y, w, h)` 把根放到 `(x, y)`，并把 `(w, h)` 交给 Taffy。命中和录制用的已经是绝对坐标。

`ScrollArea` 包一个 `Scroll` 和一列内容。拖动时把手指位移的反号加进 offset，于是内容跟手。松手后用 `DragEnd` 的速度做指数衰减，每一帧 `requestFrame`，速度降到阈值就停。offset 夹在 0 和「内容尺寸减视口尺寸」之间，两个尺寸都读 `rect`。core 不算这条曲线。

```text
Scroll                 视口，PushClip
  Column               内容，高度可以超过视口
    …行
```

`Field` 也写在 `frameworks/solid-ui`。字符串、光标和组合文本留在组件里。`core` 只记焦点，并把按键和文本送到当前焦点。聚焦时打开输入法，边框换成焦点色，并用 `measureText` 把一根 1 点宽的光标放到文字末尾。聚焦期间每帧 `requestFrame`，光标按时间闪烁。若包在 `Scroll` 里，聚焦时把视口滚到这块输入框。

`Switch` 是一个 `pointer` 加 `decorated`。传入 `checked` 时由调用方控制；省略时组件自己留一个布尔信号。`Dialog` 是铺满的 `stack`：遮罩 `pointer` 的点击调用 `onClose`，内容列叠在遮罩上面，所以点内容不会关掉。这一阶段不做焦点陷阱。

## 应用

`apps/gallery-solid/counter.tsx` 里有计数器、随 `onFrame` 改变透明度的文字、一张 `asset:logo`、两个 `Button`，以及 `ScrollArea` 里的 50 行和两个 `Field`。透明度动画每帧调用 `requestFrame`，所以这份画廊在播放时连续要帧。`boot` 把根列撑满视口，登记 `flush` 并 `render`。`build.mjs` 用 `@solidjs/babel-plugin`（`moduleName: "@uifw/solid-renderer"`、`generate: "universal"`）再交给 esbuild 打成 IIFE。依赖是 npm 上的 `2.0.0-rc.17`。产物不包含 `third_party/solid`。`dist/counter.js` 提交进仓库，QuickJS 会话加载它。`cargo test` 不跑 node。

`apps/gallery-kotlin` 用 `com.lightningkite:reactive` 的 `Signal` 重建计数器场景，并打印计数值 0 和 1 的调试文本。`count == 0` 的编码与 `counter.bin` 相同。`protocol` 的测试读取这份金样。

应用代码里不出现约束、命中测试、Metal 或窗口。字节布局和原生层验收在 `docs/native-phase.md`。
