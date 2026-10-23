# AGENTS.md

本文件是后续 agent 的工作约定。协议、引擎接口和字节测试以代码为准。人类可读的简介在 `README.md`。两边不一致时，先读 Rust 源码和测试，再改文档。

用户和仓库文档使用中文。回复用户用中文。

写 README 和 `docs/` 时用目录名和类型名。渲染在 `engine`。版本 1 计数器场景的对照文件是 `protocol/tests/fixtures/counter.bin`。文档只写仓库里能核对的事实，不自造叫法，也不写某次讨论的背景。

## 名字

项目名是 Future。产品代码的开源协议是 Apache-2.0，见仓库根的 `LICENSE` 和 `NOTICE`。

Rust crate 用目录名，例如 `protocol`、`core`、`engine`。不要写成 `future-core` 或 `uifw-core`。模块路径本身就是层：`core::tree`、`protocol::encode`。

C 符号没有模块，所以前缀用层名，不用产品名：

| 层 | C 前缀 | 例子 |
|---|---|---|
| 场景协议 | `protocol_` | `protocol_decode` |
| UI 核心 | `core_` | `core_tree_layout` |
| 渲染引擎 | `engine_` | `engine_draw` |
| 运行时会话 | `session_` | `session_frame` |

宿主是可执行文件，不导出符号。Rust 模块内部不要再加这些前缀。不要使用 `future_` 或 `uifw_` 作为新符号的前缀。Cargo 把 rlib 放在 `target/`，不要安装成系统里的 `libcore`。

Kotlin 包名 `dev.uifw`、Solid 包名 `@uifw/solid-renderer`、环境变量 `UIFW_COUNTER_BYTES` 和 `UIFW_DUMP` 仍用旧前缀。这些名字留到改对应语言层时再换，不单独做一次改名。

计数器场景的版本 1 编码在 `protocol/tests/fixtures/counter.bin`，208 字节。Kotlin 和 TypeScript 各自编码这一棵树，输出必须和这个文件逐字节相同，目前不调用 Rust。格式是固定的小端字段，开头四字节是 `UIFW`，用来识别这份场景。改这套字节时，Rust、Kotlin、TypeScript 和这个文件一起改。

## 优先复用已有实现

重要模块先找已经广泛使用的开源实现。能按发布版本链接进去，就不要自己写，也不要把上游源码抄进产品目录。引入前看三件事：它负责哪一层、许可证、会不会带进 Dart VM、整份 Flutter engine 或第二套光栅器。对不上这些边界的，只借鉴接口形状。

选定的依赖记在下表。换选型时改这张表，并删掉文档里的旧方案，不要在表下面再留一份被放弃的设计。

| 层 | 使用 | 不引入 |
|---|---|---|
| 布局 | [Taffy](https://github.com/DioxusLabs/taffy)（MIT，Rust crate）。`core` 直接调它的 Rust API，覆盖 Flexbox、Grid、Block | 自研 flex。Yoga。Flutter 的 RenderObject（在 Dart 里，拆不出来）。Taffy 未完成的 C 绑定 |
| 文字测量和绘制 | 已钉住的 Impeller Typography。测试用一份等宽的假排版器 | 再链一套 HarfBuzz / FreeType |
| 光栅 | Impeller standalone SDK | `embedder.h`、Flutter shell、Skia、engine 源码。这三样都会带上 Dart VM，或等于把 engine 克隆进仓库 |
| 宿主窗口 | 各端系统 API。macOS 用 objc2 调 AppKit。Android 用 `android-activity` 0.6.1（MIT OR Apache-2.0）包 GameActivity，vsync 用 `AChoreographer`。图片解码用 `AImageDecoder`。`ndk` 0.9.0 只提供 NDK 绑定 | GLFW。Flutter embedding。这两样都会带进另一套窗口或 Dart VM |
| JavaScript | [quickjs-ng](https://github.com/quickjs-ng/quickjs)（MIT）v0.17.0。`runtimes/quickjs/quickjs.pin` 钉 tag 和 sha256，`cc` 编进包名 `quickjs` 的 crate。源码在 `third_party/quickjs`，不进 git | bellard/quickjs（上游基本停更）、Hermes、V8、React Native JSI |
| Solid 状态 | `solid-js` 的 signal 和 `@solidjs/universal` | 改 `third_party/solid` |
| Kotlin 状态 | 接到信号时依赖已发布的 `com.lightningkite:reactive`，版本那时再钉 | 把 KiteUI 或这份库的源码拷进仓库 |
| 手势 | 点击写在 `core/`。没有独立、可嵌入、又不绑死某一套 UI 的库 | React Native Gesture Handler、Flutter gestures |
| 图片解码 | 宿主平台的解码器，现在是 ImageIO。像素以 RGBA 交给 Impeller | Flutter 的 `dart:ui` 图片栈。没有平台解码器的宿主再考虑 `stb_image` |
| 场景字节和显示列表 | 自己的薄 ABI。Impeller 的 C API 不稳定，也不能当序列化格式 | 把 Impeller 的内部 DisplayList 类型暴露成产品 ABI |

Taffy 只算盒子的位置和大小，含 Grid。文字和图片用它的 measure 回调把尺寸交回去，排版仍走 Impeller。命中测试、手势和绘制仍在 `core/`。

我们自己写的原生代码用 Rust：渲染树、场景格式、用 Impeller 绘制的渲染引擎、macOS 宿主。Rust 编码以 `.agents/skills/rust-best-practice` 为准，调用 `/rust-best-practice`。写本仓库的 Rust 时重点看其中的 trait、强类型、引用和泛型。布局和绘制在渲染线程上同步执行，不放进 async 运行时。异步留给以后的文件、网络和脚本加载。平台线程做窗口、输入换算和 vsync 登记。`Engine` 是 `!Send`。`register_font`、`submit`、`draw_metal`、`draw_vulkan` 和 typography 只在渲染线程上调用，并且在该线程上创建和销毁。`CAMetalLayer` 在主线程创建并挂到视图上；渲染线程只用层指针取 drawable。Metal context 仍用 `MTLCreateSystemDefaultDevice()`。

`protocol`、`core`、`engine`、`session`、`hosts/hostshell`、`hosts/macos`、`hosts/android`、`runtimes/quickjs` 是 Cargo workspace。`protocol`、`core`、`engine`、`hosts/macos` 在进程内部不经过 C。不要再给这四层加 C 或 Objective-C，也不要新增产品 C 头文件。QuickJS 的 C 只在 `runtimes/quickjs/src/glue.c`：解释器回调进 Rust 的 `Tree`，不把 Rust 类型漏出进程。没有 quickjs-ng 源码时这个 crate 返回 `NotLinked`。Kotlin 的 C ABI 留到 cinterop 有调用者时再写。工具链用 `rust-toolchain.toml` 钉住 stable。原生构建以 Cargo 为准。

## 保持仓库干净

代码和文档只保留当前路径还要用的东西。方案被替换、文件没有引用、文档在描述已经放弃的做法时，直接删。不要留兼容壳、注释掉的旧实现、空目录，或「以前曾经」的段落。

删除前用搜索确认没有引用，并跑受影响的测试。`third_party` 里钉住的上游对照和 SDK 不按废弃代码删。测试还在读的样本文件留着；测试改完，旁边的临时编码器和临时绘制路径一起删。

改文档时替换旧段落。不要在下面追加更正，让两套说法同时存在。

## 项目背景

这是一个跨平台 UI 框架，产品代码在这一个仓库里。上层是细粒度响应式，下层用同一份场景字节，交给 Impeller 绘制，让各端像素一致。

两条产品路线都要做，共用场景协议、引擎嵌入和以后的图片管线：

| 路线 | 目录 | 运行时 | 现在的状态 |
|---|---|---|---|
| Solid 2.0 TSX + QuickJS | `frameworks/solid-renderer`、`frameworks/solid-ui` | QuickJS，脚本可下发 | 场景编码器已与 `counter.bin` 对字节。`apps/gallery-solid/dist/counter.js` 是编译产物，由 QuickJS 会话驱动 `core` |
| Kotlin lambda DSL | `frameworks/kotlin-ui` | 目标是 Kotlin/Native AOT | 只有 JVM 上的 DSL 和编码器。信号和原生目标还没接 |

Kotlin 路线的写法参考 KiteUI（`scene { layer { rect(...) } }`）。KiteUI 画的是各端原生控件。这里的 DSL 要重写，输出 `protocol/` 里的场景，不沿用它的 View / UIKit / DOM 后端。

图片增强（降采样、色彩管理、缓存）等框架能在真机上跑起来再做。Flutter 自带的图片解码在 Dart 侧，这条路径用不到。

## 技术路线

场景格式是 ABI。以 `protocol` 的 Rust 编码器和 `counter.bin` 为准。Kotlin 和 TypeScript 各有一份对照编码器，输出必须和这个文件逐字节相同，目前不调用 Rust。以后 Kotlin 走 cinterop，再删掉这两份对照实现。QuickJS 会话在进程内直接调用 `core::Tree`。

绘制不经过 Dart。`embedder.h` / `FlutterEngineRun` 会启动 Dart VM，本进程不使用它。macOS 宿主用 Impeller standalone SDK（`impeller.h`）的 Metal 后端，调用 `Engine::draw_metal`。Android 宿主用同一份 SDK 的 Vulkan 后端，调用 `Engine::create_vulkan` 和 `Engine::draw_vulkan`。不加载脚本时，`protocol` 把 `counter.bin` 转成 `DrawList`，渲染引擎只画这份列表：矩形、段落、RGBA 纹理。iOS 预构建矩阵没有这一列，宿主还没写。UI 代码继续写 Kotlin 或 JS。

Impeller 的 C API 没有稳定 ABI。钉住的是一份 Flutter 框架提交，不是 engine 仓库的提交。创建 context 时传入该头文件里的 `IMPELLER_VERSION`。升级 pin 时一起改调用代码。

不加载脚本时，macOS 用 `counter.bin` 画出第一帧。`just host` 打开 360×640 的窗口，提交一次这份 `DrawList`。`UIFW_DUMP` 把第一帧 Metal 纹理写成 PNG 后退出。`just host-js` 加载 `apps/gallery-solid/dist/counter.js`。宿主把指针换成左上角的场景点放进队列。`just host` 不把队列交给会话。`--js` 由会话出队。计数器的加减写在应用里，不写在宿主里。click 的验收是 `core` 里的 `Gesture`，以及 QuickJS 会话里减号把文字改成 `Count: -1`。按下和抬起另外分发 press 与 release。抬起仍在按下区域内时，焦点沿父链找到 `focusable` 节点，发出 `Blur` 再 `Focus`。滑出、取消、以及越过拖动阈值交给 `Scroll` 的手势不改焦点。

计数器加减是第一条应用，用来验收框架，不是把逻辑写进宿主。`count`、`onClick` 和按钮摆放都属于上层。

仍然守住的外部约束：不要把 Flutter engine 源码克隆进本仓库。engine fork 是另一个仓库。iOS 要等上游预构建矩阵有这一列再写宿主。场景 diff 在有实测工作负载之前不做：静止时宿主已经不登记下一帧，交互帧的固定开销在纹理上传和排版。macOS 与 Android 都是平台线程入队，渲染线程绘制。

## 框架分层

测量、布局、命中测试、绘制录制只有一份，放在 `core/`。Solid 和 KiteUI 都不提供这套跨端实现，不要在 TypeScript 或 Kotlin 里再写一套。

Solid 的 `@solidjs/universal` 只有 `createElement`、`insertNode`、`setProperty` 这种树更新。网页上的测量和布局是浏览器做的。KiteUI 的 `RowOrCol` 是 `expect/actual`，分别落到 Android `onMeasure`、UIKit `layoutSubviews` 和 CSS。`ProgrammaticLayoutDelegate.measure` 再去问平台子节点的尺寸。沿用它们会得到多套布局，也画不出同一份 Impeller 画面。

对照是 Flutter 的管线：组件配置和状态在上层，RenderObject 做约束布局、绘制录制和命中测试，DisplayList 交给光栅器。Compose 同样把重组和 `MeasurePolicy` 分开。这里的光栅器是 Impeller，语言运行时是 Solid 或 Kotlin。

| 层 | 落点 | 职责 |
|---|---|---|
| 宿主 | `hosts/macos`、`hosts/android`。iOS 还没有预构建 SDK | 表面、vsync、视图尺寸、缩放、安全区、原始指针、按键和输入法、字体与图片字节、把 DisplayList 交出去并上屏 |
| 运行时会话 | `runtimes/quickjs`，以后 Kotlin 绑定 | 把一种语言接到 `core/`。一帧里的顺序是：处理输入、冲刷信号、改元素树、布局、录制、提交。宿主不关心语言 |
| UI 核心 | `core/` | Rust 渲染树。布局调用 Taffy。测试里的文字测量是等宽 `Shaper`。命中测试走这棵树。`record` 交出 `protocol::DrawList`。给 Kotlin 用的 C ABI 还没写。QuickJS 在会话 crate 里直接调这棵树 |
| 场景协议 | `protocol/` | 布局和绘制之后的显示列表 ABI。绝对坐标。没有 flex，没有回调，没有应用状态 |
| 渲染器 | `frameworks/solid-renderer`，`frameworks/kotlin-ui` | 把 JSX 或 Kotlin DSL 变成对 `core/` 的增删改。只标脏，不算坐标 |
| 响应式 | Solid signal；Kotlin 侧以后的 signal | 应用和控件的状态。写信号会标脏对应元素并请求下一帧 |
| 基础控件 | 布局原语在 `core/`。`Button`、`ScrollArea`、`Field` 这类组合控件在各语言包一层 | `Text`、`Row`、`Column`、`Stack`、`Padding`、`Clip`、`Opacity`、`Scroll`、`Transform` 是渲染对象。`Button` 用它们加上手势组成。`Field` 自己保存字符串、光标和组合文本 |
| 应用 | `apps/gallery-solid`、`apps/gallery-kotlin` | 只写状态和控件组合 |

一帧在渲染线程上完成。平台线程把指针、metrics 和生命周期放进命令队列后立即返回。命中测试用上一帧的几何，在渲染线程排空队列时做。监听器只写状态。接着才冲刷、布局、录制。列表与上次提交不同才 `submit` 和 present。没有新的指针、metrics 或表面事件时不再登记下一帧，除非会话申请了恰好下一帧。这个申请在 present 之后被取走。动画要连续播放，就在每一帧里再申请。目录、依赖方向和各层内部见 `docs/architecture.md`。三端差异见 `docs/platform-hosts.md`。

```
pointer → hit-test（上一帧）→ 监听器写信号
key / text / composing → 有焦点才派发到该节点
vsync → 冲刷信号 → layout → 录成 DisplayList → player → present
present 之后 wants_ime 变化才让平台弹出或收起键盘
```

指针事件是 `core::PointerEvent`：down、move、up、cancel，坐标是场景点，原点在左上。macOS 宿主先把窗口坐标换进内容视图，再把 Y 翻到左上。AppKit 给出的已经是点，不除 `backingScaleFactor`。宿主只入队。命中测试在 `core` 里沿着上一帧的盒子走，不读场景字节。`Text` 和 `Image` 命中自己的盒子。布局容器把命中交给子节点。`click` 由 `Tree::pointer` 合成：按下时记住最近的 `PointerRegion`，移出则取消，抬起时仍是该节点或其子孙就返回该区域的 `NodeId`。同一次抬起才移动焦点：从命中节点沿父链找 `focusable`，先 `Blur` 旧节点，再 `Focus` 新节点。已经聚焦的节点再点一次不重复发 `Focus`。锁定成拖动之后不再发 click，也不改焦点。会话在这次借用结束后分发监听器。监听器只改状态，例如 `set_text`。`set_text` 保留上一帧的几何，到下一次 `layout` 才重算。

按键和输入法不进 `core`。`session::InputEvent` 是 `Key`、`Text` 或 `Composing`。无焦点时会话丢掉事件。有焦点时，借用结束后把事件派发给该节点的监听器，监听器只改自己的字符串。`Tree::measure_text` 用这棵树的 `Shaper`，参数与 `Shaper::measure` 相同。`Session::wants_ime` 表示键盘是否该弹出，宿主在 `frame` 之后比较上次投递的值，变了才调用 `Services::set_ime`。`take_ime_text` 只在聚焦和由 `Key` 改动已提交串时取走一份，用来调用 `set_ime_text` 把平台缓冲写成这份串；`Text` 和 `Composing` 不回写，避免清掉输入法正在用的组合区间。用户把键盘收起时，宿主投递 `ImeDismissed`，会话失焦并清掉 `wants_ime`。光标闪烁只在聚焦时 `requestFrame`。

版本 1 字节记录已经算好坐标的矩形、文字和图片。渲染引擎不解析这套字节。`just host` 的第一帧由 `protocol::scene_to_draw_list` 收成 `DrawList`。运行中的界面由 `core::Tree::record` 交出同一份列表，坐标已经是绝对的。圆角、裁剪、透明度已进 `DrawList`。文字命令还带段落样式：对齐、字重、方向、最大行数、省略号、行高倍数，以及可选的 family。缺省是左对齐、字重 400、从左到右、不限制行数，行高用字体自己的。family 缺省时用渲染引擎注册的那一份字体。`PushTransform` 把随后的命令平移、顺时针旋转再等比缩放，直到配对的 `Pop`。旋转单位是弧度。命中仍用布局盒子，不吃变换。版本 1 字节不产生这条命令。布局样式留在渲染树上，不进字节，也不由 `core` 再编码成版本 1。Kotlin 和 TypeScript 的版本 1 编码器只用来和 `counter.bin` 对照。

文字测量的实现必须和渲染引擎绘制文字的实现是同一个排版器。`DrawList` 的文字命令带 `font_size`、`max_width` 和颜色。版本 1 字节没有样式，映射时填字号 28、段落宽 280，这两个常数在 `protocol`。渲染引擎按命令里的字号绘制，不乘 `point_scale`。宿主注入的排版器调用 `Engine::measure_paragraph`，和绘制共用同一段已经带上前景色的段落。测试用 `MonoShaper`。不要往渲染引擎里加布局。

布局原语是 `Sized`、`Padding`、`Align`、`Stack`、`Row`、`Column`、`Grid`、`Text`、`Image`、`PointerRegion`、`Decorated`、`Clip`、`Opacity`、`Scroll`、`Transform`。任何节点都可以 `set_focusable`。几何交给 Taffy，含 flex grow 和 grid。`Stack`、`Clip`、`Opacity`、`Transform` 是一格的 grid，子节点放在第 1 行第 1 列。`Scroll` 也是一格 grid，轨道是 `max_content`：视口是边框盒，子节点按完整尺寸布局，可以高出视口。`set_scroll_offset` 不进 Taffy，只让下一帧的录制和命中平移。绘制坐标减去 offset，命中查询加上 offset，并用 `PushClip` 裁到视口。`Transform` 录 `PushTransform` … `Pop`。`set_justify` 和 `set_wrap` 写行或列的主轴对齐和换行。`Text` 和 `Image` 用 measure 回调报告尺寸。`Decorated` 按子节点定尺寸，命中交给子节点，并录一条盖住边框盒的矩形，圆角记在命令上。宽或高为 0 时不录矩形。`Clip`、`Opacity`、`Scroll` 和 `Transform` 在子节点前后录 `Push` 和 `Pop`，盒子为 0 也成对。

已落地的原生层写在 `docs/native-phase.md`。那是 `protocol`、`core`、渲染引擎、会话、`hostshell`、macOS 宿主和 Android 宿主的规格，不是待办清单。iOS 没有预构建 SDK，宿主还没写。字节保持版本 1。不新增产品 C 头文件。计数器的 Solid 产物由 QuickJS 加载。Kotlin 还没有原生调用者，所以不写 C ABI。

不要在 `frameworks/solid-renderer` 或 `frameworks/kotlin-ui` 里实现第二套 measure / layout。

## 必须守住的边界

- 产品代码只在 `third_party/` 以外。`third_party/solid` 和 `third_party/kiteui` 是本地的上游 git 检出，保持它们自己的历史，不提交进本仓库。克隆命令在 `third_party/README.md`。不在里面加功能，不把它们并进产品的构建。Solid 是 `next` 分支（Solid 2.0）。KiteUI 是 `version-8.3`。KiteUI 的 signal 在外部库 `com.lightningkite:reactive`，这里没有带进来。
- `third_party/kiteui/AGENTS.md` 描述的是上游 KiteUI。它和本文件冲突时，产品决策以本文件为准。
- 根目录 `settings.gradle.kts` 只有 `rootProject.name`，没有 `include`。它挡住 Gradle 从仓库根向上找到用户家目录里的工程。Kotlin 工程在 `frameworks/kotlin-ui`。始终用 `frameworks/kotlin-ui/gradlew -p frameworks/kotlin-ui`。不要把 Kotlin 模块写进根 settings，也不要做覆盖所有语言的单一 pnpm 或 Gradle 根。
- 没有 Impeller SDK 时 `Engine::create_metal` 返回 `Status::NotLinked`。没有 C stub。
- 不要提交 `third_party/impeller-sdk/`、`target/`，也不要解压约 1GB 的 `libimpeller.a`。`engine/fetch-impeller-sdk.sh` 按 `engine/engine.pin` 把每个平台解到 `third_party/impeller-sdk/<平台>/`，只留 `include/*`、动态库和 `LICENSE.sdk.md`。darwin 的 install name 是 `@rpath/libimpeller.dylib`。Android 的 SONAME 是 `libimpeller.so`。Cargo 的输出在 `target/`。
- 改场景字节时同时改 Rust `protocol`、Kotlin、TypeScript 和 `protocol/tests/fixtures/counter.bin`。调试文本不是 ABI。
- 节点的 `x, y` 是绝对坐标。`parent` 只表示结构，渲染引擎不加父节点变换。`LAYER` 不产生绘制调用。
- 宿主不保存 UI 状态，不创建按钮，不解释 `count`，也不做命中测试。它把指针入队，并把 `DrawList` 交给渲染引擎。`just host` 提交一次列表。`--js` 每帧把队列交给会话。命中、手势和绘制录制在 `core`。场景格式在 `protocol`。Impeller 绘制在 `engine`。布局只调用 Taffy。Solid 渲染器和 Kotlin DSL 只改元素树。不要给 `protocol`、`core`、`engine`、`hosts/macos` 再加 C 或 Objective-C。QuickJS 的胶水 C 只留在 `runtimes/quickjs`。
- `register_font`、`submit`、`draw_metal`、`draw_vulkan` 只在渲染线程上调用。`Engine` 是 `!Send`。Impeller 的 typography context 不能离开这条线程。`CAMetalLayer` 在主线程创建并挂到视图上；渲染线程只用层指针取 drawable。Metal context 仍用 `MTLCreateSystemDefaultDevice()`。Android 的 `ANativeWindow` 由平台线程持有，要等渲染线程应答 `SurfaceDropped` 之后才 `release`。

## Monorepo

```
protocol/                  版本 1 场景格式、解码、counter.bin、DrawList
core/                      渲染树。Taffy、等宽排版、命中、click
engine/                    渲染引擎。只读 DrawList，调用 impeller.h
session/                   会话 trait。宿主只看见 metrics、pointer、frame、lifecycle
hosts/hostshell/          命令队列、帧调度、渲染线程
hosts/macos/               macOS 窗口。指针入队、CVDisplayLink、UIFW_DUMP
hosts/android/             Android 宿主。GameActivity、AChoreographer、Vulkan。`just android` 打 APK
hosts/ios                  还没有。上游预构建矩阵没有 iOS 列
runtimes/quickjs/          quickjs-ng 会话。包名 `quickjs`。没有源码时返回 NotLinked
frameworks/solid-renderer/ 版本 1 编码器，以及接到 core 的运行时。不做布局
frameworks/solid-ui/       Button、ScrollArea、Field、Switch、Dialog
frameworks/kotlin-ui/      Kotlin Gradle 根。DSL、控件、以后的 cinterop
apps/gallery-solid/        Solid 应用。源码 counter.tsx，产物 dist/counter.js
apps/gallery-kotlin/       Kotlin 应用
third_party/solid          solidjs/solid，分支 next。本地检出，不进 git
third_party/kiteui         lightningkite/kiteui，分支 version-8.3。本地检出，不进 git
third_party/impeller-sdk/  本地 SDK，不进 git
third_party/quickjs/       quickjs-ng 源码，不进 git
.agents/skills/             跟随仓库的 agent skill。Rust 编码以 rust-best-practice 为准
docs/architecture.md       仓库划分和各层内部结构
docs/native-phase.md       已落地的原生层规格
docs/platform-hosts.md     macOS、Android、iOS 的平台差异。iOS 宿主还没写
```

原生构建是 Cargo，输出在 `target/`。根上没有 CMake。`runtimes/quickjs` 在 workspace 里。没有 `third_party/quickjs` 时不编译解释器。`just quickjs` 按 pin 下载源码。

Kotlin：Kotlin 2.3.20，Gradle wrapper 9.2.1，JVM target 21，group `dev.uifw`。`settings.gradle.kts` 里 `include(":library")`，再用 `projectDir` 把 `:gallery` 指到 `apps/gallery-kotlin`。

Solid：包名 `@uifw/solid-renderer`，`type: module`。`just solid` 用 `node --experimental-strip-types` 跑字节对比、`tree.test.ts` 和 gallery，不安装 `solid-js`。编码器标签是 `layer`、`rect`、`text`、`image`。`sceneRendererOptions()` 和 `hostRendererOptions()` 的方法集对齐 `@solidjs/universal` 的 `RendererOptions`。宿主标签是 `row`、`column`、`stack`、`padding`、`decorated`、`pointer`、`clip`、`opacity`、`scroll`、`transform`、`image`、`sized`。`Text` 是组件。编译在 `apps/gallery-solid/build.mjs`：

```js
solidPlugin({
  moduleName: "@uifw/solid-renderer",
  generate: "universal",
})
```

依赖钉 `solid-js` 和 `@solidjs/universal` 的 `2.0.0-rc.17`。`just solid-app` 安装依赖、重编 `dist/counter.js`，再用 mock host 跑一遍。`just check` 不跑这一步。

## 场景协议

格式在 `protocol` crate，版本 1，小端。测试在 `protocol/tests/scene.rs`，对照文件是 `protocol/tests/fixtures/counter.bin`。

- 0–3：`UIFW`
- uint32 版本，uint32 `node_count`
- 每个节点：uint32 id，uint32 parent（0 表示挂在根上），uint8 kind，3 字节 0，float32 x,y,w,h，uint8 r,g,b,a，uint32 `str_len`，随后是这么长的 UTF-8，不带 NUL
- kind：`LAYER=1`，`RECT=2`，`TEXT=3`，`IMAGE=4`
- id 从 1 起，按创建顺序。parent 可以出现在子节点后面。`Scene` 在创建时拒绝未知 parent
- 固定前缀 36 字节再加字符串。短缓冲区返回 `Error::ShortBuffer`，并且不写输出

计数器场景（`protocol::counter`、Kotlin `counterScene()`、TypeScript `counterScene()`）是同一棵树：

- layer，id 1，parent 0
- rect，id 2，parent 1，`(0, 0, 320, 180)`，rgba `(17, 17, 17, 255)`
- text `"Count"`，`(24, 36)`，白色
- text `"0"`，`(24, 72)`，rgba `(124, 255, 107, 255)`
- image `"asset:logo"`，`(240, 24, 56, 56)`

编码长度 208 = 12 + 5×36 + len(`Count`) + len(`0`) + len(`asset:logo`)。测试把编码器输出和 `counter.bin` 逐字节比较，不要用 Rust 重新生成这个文件。

`decode` 拥有解码出来的 `String`，并拒绝截断、错误魔数、错误版本、非法 kind、尾部多余字节和非法 UTF-8。

## 绘制

渲染引擎是 `engine::Engine`。它接收 `protocol::DrawList`，不解析场景字节，也不依赖 `core`。

- `Engine::create_metal`：没有 SDK 时返回 `Status::NotLinked`
- `register_font`：拷贝字体字节。宿主默认注册 `/System/Library/Fonts/Supplemental/Arial.ttf`，family 名 `UIFWSans`
- `submit`：`DrawList`、`point_scale`、图片回调。`point_scale` 把一个场景单位乘成像素。回调的 source 是 `&str`。RGBA 活到 `submit` 返回
- `draw_metal`：系统默认设备上的 `CAMetalLayer` drawable，指针是 `*mut c_void`
- `create_vulkan` / `draw_vulkan`：只在 Android 目标上编译。swapchain 由宿主的 `Surface` 持有，尺寸变化不丢链

成功是 `Result::Ok`。失败是 `Status::NotLinked`、`BadScene`、`NotReady`（还没有 drawable、context 或 display list，或者 present 失败）。空文字、缺图、非正缩放是 `BadScene`。

`Fill` 在缩放之前铺满画布。缩放只做一次，放在第一条非填充命令之前。版本 1 映射先放一条 `Fill`，颜色是 `(17, 17, 17, 255)`，并把文字填成字号 28、段落宽 280、缺省段落样式。渲染引擎使用命令里的 family、字号、字重、对齐、方向和 `max_width`，字号不乘 `point_scale`。左上角放在命令的 `(x, y)`。`measure_paragraph` 和绘制共用同一段段落。相同文字、字号、宽度、颜色和样式跨帧复用。纹理按 `source` 跨帧复用，资产在进程内不可变。成功提交结束时只释放当前显示列表没有引用的纹理和段落。`register_font` 使旧段落在下一次提交释放显示列表之后失效。上屏失败是 `PresentFailed`，宿主保留表面并再要一帧。还没有表面或显示列表是 `NotReady`。图片要调用方给出紧排 RGBA8888。Impeller 不接受 PNG 或 JPEG。`source` 以 `asset:` 开头。macOS 把它映射成启动参数里的资产目录下的 `<name>.png`，默认目录是 `hosts/macos/assets`。Android 映射成 APK 里的 `assets/images/<name>.png`。缺图仍是 `BadScene`。

窗口标题 `uifw counter`，内容默认 360×640，可关闭、可最小化、可缩放。缩放和改尺寸都投递 `Metrics`。`contentsScale` 和 `drawableSize` 来自窗口所在屏幕的 `backingScaleFactor`（取不到时用 2），尺寸变化时按内容视图重算。这一层 `framebufferOnly = NO`，这样 `UIFW_DUMP` 能读回纹理。

SDK pin 在 `engine/engine.pin`：Flutter `77f043bc63196b9979cc09c5b97bb6d9b00be34d`，平台 `darwin-arm64`。换提交前先对 URL 做 HEAD，确认返回 200，再写进 pin。

## 构建和测试

工具：稳定版 rustc（`rust-toolchain.toml`）、Cargo、just、JDK 21、仓库内的 Gradle wrapper、带类型剥离的 Node（`--experimental-strip-types`）。macOS 宿主需要 Xcode 和本机 arm64。

新机器先取 SDK，再跑检查：

```sh
engine/fetch-impeller-sdk.sh
just check
```

`just check` 先 `cargo test --workspace`，再跑 Kotlin 的 `:library:jvmTest` 与 `:gallery:runGallery`，以及 Solid 的字节对比和 gallery。它不会打开窗口。没有 quickjs-ng 源码时 QuickJS 测试走 `NotLinked`。有源码时会编译解释器并跑编译后的计数器。没有 SDK 时 `Engine` 返回 `NotLinked`，图形测试不编译；Kotlin 和 Solid 不需要 SDK。`just host` 需要 SDK。

GitHub Actions 工作流是 `.github/workflows/check.yml`。它跑 `cargo fmt --all -- --check`、Linux 上的 `cargo test --workspace --exclude macos`、macOS 上的 `cargo test -p macos`，以及和 `just kotlin`、`just solid` 相同的检查。`macos` 这个 crate 只在 macOS 上编译。工作流不下载 Impeller SDK，也不下载 quickjs-ng，因此渲染引擎走 `NotLinked`，QuickJS 测试同样走 `NotLinked`。窗口、`just android` 和 `just solid-app` 不在这套检查里。

| 命令 | 作用 |
|---|---|
| `just check` | `cargo test`，然后 Kotlin 和 Solid。不启动 GUI |
| `just test` | `cargo test --workspace` |
| `just kotlin` | JVM 测试和 Kotlin gallery。Gradle 任务把 `UIFW_COUNTER_BYTES` 指到 `counter.bin` |
| `just solid` | 用 `UIFW_COUNTER_BYTES` 对比字节，跑宿主树测试，再跑 Solid gallery |
| `just solid-app` | 安装依赖，重编 `dist/counter.js`，再用 mock host 跑渲染 |
| `just host` | 打开 macOS 窗口。参数是 `protocol/tests/fixtures/counter.bin` 和 `hosts/macos/assets/logo.png` |
| `just quickjs` | 按 `runtimes/quickjs/quickjs.pin` 下载 quickjs-ng 源码 |
| `just host-js` | 打开 macOS 窗口并加载 `apps/gallery-solid/dist/counter.js`。需要 SDK 和 quickjs-ng 源码 |
| `just android` | 编 `aarch64-linux-android` 的 `libuifw.so`，再打 GameActivity APK。需要 NDK 和 `android-arm64` SDK |
| `UIFW_DUMP=/tmp/uifw-frame.png just host` | 画出第一帧，写成 PNG，然后进程退出 |

单独跑 Gradle 时也要钉住工程目录：

```sh
frameworks/kotlin-ui/gradlew -p frameworks/kotlin-ui :library:jvmTest :gallery:runGallery
```

JVM 测试的环境变量由 `library/build.gradle.kts` 设成 `protocol/tests/fixtures/counter.bin`。Solid 测试读 `just solid` 导出的同一个变量。

看像素用 `UIFW_DUMP` 的读回，按解码后的像素比较，不要比 PNG 文件字节，也不要用 `screencapture -l` 当作成功标准。窗口进程用完要退出。dump 模式会自己终止；普通 `just host` 会留下窗口，验证完结束掉它。

## 改代码时

- 动到节点布局或编码，先让 `protocol` 的测试、Kotlin `CounterEncodeTest`、`frameworks/solid-renderer/src/encode.test.ts` 和 `counter.bin` 仍是同一份字节。
- 渲染引擎调用在 `engine/src/player.rs`，对照 `impeller.h`。没有 stub。没有 SDK 时走 `Status::NotLinked`。
- 动到 pin 或 `impeller.h` 的调用，保持 `IMPELLER_VERSION` 来自刚解压的那份头文件。fetch 脚本继续改 darwin 的 install name，并核对 Android 的 SONAME。`engine` 和 `hosts/macos` 的 `build.rs` 都要把 rpath 指到 `third_party/impeller-sdk/darwin-arm64/lib`。Android 目标链接 `third_party/impeller-sdk/android-arm64/lib`。
- 给宿主加第二种图片时，在解析回调里按 source 区分，并把像素保持到 `submit` 返回。
- 不要为了对齐调试文本去改编码器。不要把节点坐标改成相对父节点，除非协议版本一起升级，并且 Kotlin、TypeScript 和渲染引擎一起改。
