# AGENTS.md

本文件是后续 agent 的工作约定。协议、引擎接口和字节测试以代码为准。人类可读的简介在 `README.md`。两边不一致时，先读 Rust 源码和测试，再改文档。当前结构在 `docs/architecture.md`。还没落地的目标在 `docs/refactor-plan.md`。本文件里和那份规划冲突的远期禁令以规划为准。

用户和仓库文档使用中文。回复用户用中文。

写 README 和 `docs/` 时用目录名和类型名。渲染在 `engine`。文档只写仓库里能核对的事实，不自造叫法，也不写某次讨论的背景。

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

Kotlin 包名 `dev.uifw`、Solid 包名 `@uifw/solid-renderer`、环境变量 `UIFW_DUMP` 仍用旧前缀。这些名字留到改对应语言层时再换，不单独做一次改名。

## 优先复用已有实现

重要模块先找已经广泛使用的开源实现。能按发布版本链接进去，就不要自己写，也不要把上游源码抄进产品目录。引入前看三件事：它负责哪一层、许可证、会不会带进 Dart VM、整份 Flutter engine 或第二套光栅器。对不上这些边界的，只借鉴接口形状。

选定的依赖记在下表。换选型时改这张表，并删掉文档里的旧方案，不要在表下面再留一份被放弃的设计。

| 层 | 使用 | 不引入 |
|---|---|---|
| 布局 | [Taffy](https://github.com/DioxusLabs/taffy)（MIT，Rust crate）。`core` 直接调它的 Rust API，覆盖 Flexbox、Grid、Block | 自研 flex。Yoga。Flutter 的 RenderObject（在 Dart 里，拆不出来）。Taffy 未完成的 C 绑定 |
| 文字测量和绘制 | 已钉住的 Impeller Typography。测试用一份等宽的假排版器 | 再链一套 HarfBuzz / FreeType |
| 光栅 | Impeller standalone SDK | `embedder.h`、Flutter shell、Skia、engine 源码。这三样都会带上 Dart VM，或等于把 engine 克隆进仓库 |
| 宿主窗口 | 各端系统 API。macOS 用 objc2 调 AppKit。Android 用 `Activity` 与 `HostView`：`SurfaceView` 给出 `ANativeWindow`，vsync 用主线程 `Choreographer`，输入法用 `InputConnection`。图片解码用 `AImageDecoder`。`ndk` 0.9.0 只提供 NDK 绑定 | GLFW。Flutter embedding。这两样都会带进另一套窗口或 Dart VM |
| JavaScript | [quickjs-ng](https://github.com/quickjs-ng/quickjs)（MIT）v0.17.0。`runtimes/quickjs/quickjs.pin` 钉 tag 和 sha256，`cc` 编进包名 `quickjs` 的 crate。源码在 `third_party/quickjs`，不进 git | bellard/quickjs（上游基本停更）、Hermes、V8、React Native JSI |
| Solid 状态 | `solid-js` 的 signal 和 `@solidjs/universal` | 改 `third_party/solid` |
| Kotlin 状态 | 接到信号时依赖已发布的 `com.lightningkite:reactive`，版本那时再钉 | 把 KiteUI 或这份库的源码拷进仓库 |
| 手势 | 点击写在 `core/`。没有独立、可嵌入、又不绑死某一套 UI 的库 | React Native Gesture Handler、Flutter gestures |
| 图片解码 | 宿主平台的解码器，现在是 ImageIO。像素以 RGBA 交给 Impeller | Flutter 的 `dart:ui` 图片栈。没有平台解码器的宿主再考虑 `stb_image` |
| 场景字节和显示列表 | 自己的薄 ABI。Impeller 的 C API 不稳定，也不能当序列化格式 | 把 Impeller 的内部 DisplayList 类型暴露成产品 ABI |

Taffy 只算盒子的位置和大小，含 Grid。文字和图片用它的 measure 回调把尺寸交回去，排版仍走 Impeller。命中测试、手势和绘制仍在 `core/`。

我们自己写的原生代码用 Rust：渲染树、场景格式、用 Impeller 绘制的渲染引擎、macOS 宿主。Rust 编码以 `.agents/skills/rust-best-practice` 为准，调用 `/rust-best-practice`。写本仓库的 Rust 时重点看其中的 trait、强类型、引用和泛型。布局和绘制在渲染线程上同步执行，不放进 async 运行时。异步留给以后的文件、网络和脚本加载。平台线程做窗口、输入换算和 vsync 登记。`Engine` 是 `!Send`。`register_font`、`submit_layers`、`draw_metal`、`draw_vulkan` 和 typography 只在渲染线程上调用，并且在该线程上创建和销毁。`CAMetalLayer` 在主线程创建并挂到视图上；渲染线程只用层指针取 drawable。Metal context 仍用 `MTLCreateSystemDefaultDevice()`。

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
| Solid 2.0 TSX + QuickJS | `frameworks/solid-renderer`、`frameworks/solid-ui`、`frameworks/solid-router` | QuickJS，脚本可下发 | `apps/gallery-solid/dist/counter.js` 是编译产物，由 QuickJS 会话驱动 `core` |
| Kotlin lambda DSL | `frameworks/kotlin-ui` | 目标是 Kotlin/Native AOT | 只有 JVM 上的 DSL 和编码器。信号和原生目标还没接 |

Kotlin 路线的写法参考 KiteUI（`scene { layer { rect(...) } }`）。KiteUI 画的是各端原生控件。这里的 DSL 要重写，输出 `protocol/` 里的场景，不沿用它的 View / UIKit / DOM 后端。

图片增强（降采样、色彩管理、缓存）等框架能在真机上跑起来再做。Flutter 自带的图片解码在 Dart 侧，这条路径用不到。

## 技术路线

进程内的协议是 `DrawList` 和 `LayerTree`，没有场景字节。QuickJS 会话在进程内直接调用 `core::Tree`。Kotlin 路线暂缓，见 `docs/refactor-plan.md`。

绘制不经过 Dart。`embedder.h` / `FlutterEngineRun` 会启动 Dart VM，本进程不使用它。macOS 宿主用 Impeller standalone SDK（`impeller.h`）的 Metal 后端，调用 `Engine::draw_metal`。Android 宿主用同一份 SDK 的 Vulkan 后端，调用 `Engine::create_vulkan` 和 `Engine::draw_vulkan`。运行中的界面由 `core` 交出 `LayerTree`，渲染引擎只提交这份层树。iOS 预构建矩阵没有这一列，宿主还没写。UI 代码继续写 Kotlin 或 JS。

Impeller 的 C API 没有稳定 ABI。钉住的是一份 Flutter 框架提交，不是 engine 仓库的提交。创建 context 时传入该头文件里的 `IMPELLER_VERSION`。升级 pin 时一起改调用代码。

`just host` 打开 360×640 的窗口并加载 `apps/gallery-solid/dist/counter.js`。`just host-js` 是同一条命令。`UIFW_DUMP` 把一帧 Metal 纹理写成 PNG 后退出，用来看画面。宿主把指针换成左上角的场景点放进队列，由会话出队。计数器的加减写在应用里，不写在宿主里。click 的验收是 `core` 里的 `Gesture`，以及 QuickJS 会话里减号把文字改成 `Count: -1`。按下和抬起另外分发 press 与 release。抬起仍在按下区域内时，焦点沿父链找到 `focusable` 节点，发出 `Blur` 再 `Focus`。滑出、取消、以及越过拖动阈值交给 `Scroll` 的手势不改焦点。

计数器加减是第一条应用，用来验收框架，不是把逻辑写进宿主。`count`、`onClick` 和按钮摆放都属于上层。

仍然守住的外部约束：不要把 Flutter engine 源码克隆进本仓库。engine fork 是另一个仓库。预构建矩阵没有 iOS 列时不写 `hosts/ios`；`Session` 和渲染线程约束不按只有 macOS 和 Android 来设计。不做场景 diff，增量绘制走已落地的合成层，见 `docs/architecture.md`。macOS 与 Android 都是平台线程入队，渲染线程绘制。

## 框架分层

测量、布局、命中测试、绘制录制只有一份，放在 `core/`。Solid 和 KiteUI 都不提供这套跨端实现，不要在 TypeScript 或 Kotlin 里再写一套。

Solid 的 `@solidjs/universal` 只有 `createElement`、`insertNode`、`setProperty` 这种树更新。网页上的测量和布局是浏览器做的。KiteUI 的 `RowOrCol` 是 `expect/actual`，分别落到 Android `onMeasure`、UIKit `layoutSubviews` 和 CSS。`ProgrammaticLayoutDelegate.measure` 再去问平台子节点的尺寸。沿用它们会得到多套布局，也画不出同一份 Impeller 画面。

对照是 Flutter 的管线：组件配置和状态在上层，RenderObject 做约束布局、绘制录制和命中测试，DisplayList 交给光栅器。Compose 同样把重组和 `MeasurePolicy` 分开。这里的光栅器是 Impeller，语言运行时是 Solid 或 Kotlin。

| 层 | 落点 | 职责 |
|---|---|---|
| 宿主 | `hosts/macos`、`hosts/android`。iOS 还没有预构建 SDK | 表面、vsync、视图尺寸、缩放、安全区、原始指针、按键和输入法、字体与图片字节、把 DisplayList 交出去并上屏 |
| 运行时会话 | `runtimes/quickjs`，以后 Kotlin 绑定 | 把一种语言接到 `core/`。一帧里的顺序是：处理输入、冲刷信号、改元素树、布局、录制、提交。宿主不关心语言 |
| UI 核心 | `core/` | Rust 渲染树。布局调用 Taffy。测试里的文字测量是等宽 `Shaper`。命中测试走这棵树。`record` 交出 `protocol::DrawList`。QuickJS 在会话 crate 里直接调这棵树。嵌入面走 `runtimes/quickjs/abi.yml` 生成的变更缓冲，不手写一套和 setter 一一对应的 C API |
| 场景协议 | `protocol/` | 进程内的 `DrawList` 与 `LayerTree`（`draw_list.rs`、`layer.rs`）。绝对坐标。没有 flex，没有回调，没有应用状态，没有场景字节 |
| 渲染器 | `frameworks/solid-renderer`，`frameworks/kotlin-ui` | 把 JSX 或 Kotlin DSL 变成对 `core/` 的增删改。只标脏，不算坐标 |
| 响应式 | Solid signal；Kotlin 侧以后的 signal | 应用和控件的状态。写信号会标脏对应元素并请求下一帧 |
| 基础控件 | 布局原语和组合控件内核在 `core/`。语言包是皮：模板、信号和渲染回调 | `Text`、`Row`、`Column`、`Stack`、`Padding`、`Clip`、`Opacity`、`Scroll`、`Transform` 是渲染对象。`Button` 用它们加上手势组成。`Field` 绑定 `core` 的 `TextEditor`。`ListDriver` 做窗口、测量调度和可见集 diff。`ScrollController` 做夹紧、拖动续算和刷新阈值。`createSlot` 仍是语言侧的 owner。编辑按字形簇进行，偏移是 UTF-16 |
| 应用 | `apps/gallery-solid` | 只写状态和控件组合 |

一帧在渲染线程上完成。平台线程把指针、metrics 和生命周期放进命令队列后立即返回。命中测试用上一帧的几何，在渲染线程排空队列时做。监听器只写状态。接着才冲刷、布局、布局回调、录制。`layer_epoch` 未变且缩放相同则跳过 `submit_layers`。`needs_present` 为假时也跳过 `present`；为真时 `PresentCached`，把已安装的显示列表画进新 drawable。`Resumed`、`Invalidate` 和 present 失败会置 `needs_present`。`MemoryWarning` 清掉已安装的列表，下一帧重新 `submit_layers`。没有新的指针、metrics 或表面事件时不再登记下一帧，除非会话申请了恰好下一帧。这个申请在 present 之后被取走。动画要连续播放，就在每一帧里再申请。当前结构见 `docs/architecture.md`。三端差异见 `docs/platform-hosts.md`。迭代规划见 `docs/refactor-plan.md`。

```
pointer → hit-test（上一帧）→ 监听器写信号
key / text / composing → 有焦点才派发到该节点
vsync → 冲刷信号 → layout → driver tick（写了树或可见集则再 layout）→ 布局回调（写了树则再 layout 一次）→ 录成 DisplayList → player → present
present 之后 wants_ime 变化才让平台弹出或收起键盘
```

指针事件是 `core::PointerEvent`：down、move、up、cancel，坐标是场景点，原点在左上。macOS 宿主先把窗口坐标换进内容视图，再把 Y 翻到左上。AppKit 给出的已经是点，不除 `backingScaleFactor`。宿主只入队。命中测试在 `core` 里沿着上一帧的盒子走，不读场景字节。`Text` 和 `Image` 命中自己的盒子。布局容器把命中交给子节点。`click` 由 `Tree::pointer` 合成：按下时记住最近的 `PointerRegion`，移出则取消，抬起时仍是该节点或其子孙就返回该区域的 `NodeId`。同一次抬起才移动焦点：从命中节点沿父链找 `focusable`，先 `Blur` 旧节点，再 `Focus` 新节点。已经聚焦的节点再点一次不重复发 `Focus`。锁定成拖动之后不再发 click，也不改焦点。会话在这次借用结束后分发监听器。监听器只改状态，例如 `set_text`。`set_text` 保留上一帧的几何，到下一次 `layout` 才重算。

按键和输入法的事件类型在 `session::InputEvent`，以 `session/src/lib.rs` 为准。无焦点时会话丢掉事件。编辑状态在 `core` 的 `TextEditor`。有焦点时，借用结束后把事件派发给该节点的监听器。`Tree::measure_text` 用这棵树的 `Shaper`，参数与 `Shaper::measure` 相同。键盘状态由 `Session::take_ime` 交出 `ImeUpdate`（`Show` / `Update` / `Hide`），宿主在 `frame` 之后经 `Services::apply_ime` 投递给平台，`Update` 携带完整 `ImeState`（文本、UTF-16 选区、组合区间和光标矩形）。用户把键盘收起时，宿主投递 `ImeDismissed`，会话失焦。光标闪烁只在聚焦时 `requestFrame`。

运行中的界面由 `core::Tree::record` 组出 `LayerTree`，picture 里的命令坐标相对该 picture 的原点。一段 picture 属于一个 owner：根，或本帧开层的 `Scroll`、`Opacity`、`Transform`、`Clip` 与效果节点。没有开层后代的 `Clip` 把 `PushClip` 留在父 picture 里。`Scroll` 的内容是单独一张 child picture，滚动偏移在 `Offset` 层上。平移、旋转、缩放和半透明记在层上。圆角、裁剪、透明度在层上。文字命令还带段落样式：对齐、字重、方向、最大行数、省略号、行高倍数，以及可选的 family。缺省是左对齐、字重 400、从左到右、不限制行数，行高用字体自己的。family 缺省时用渲染引擎注册的那一份字体。`PushTransform` 把随后的命令平移、顺时针旋转再等比缩放，直到配对的 `Pop`。旋转单位是弧度。命中把查询点按这条变换的逆矩阵送进子节点。圆角裁剪按圆角拒绝角上的点。布局样式留在渲染树上，不进 `DrawList`。

文字测量的实现必须和渲染引擎绘制文字的实现是同一个排版器。`DrawList` 的文字命令带 `font_size`、`max_width` 和颜色。渲染引擎按命令里的字号绘制，不乘 `point_scale`。布局测量走 `Typography::measure_paint`，`sync_paragraphs` 用 `retain_measured` 复用同一带前景色的槽。光标仍走 `Typography::measure_paragraph` 的度量缓存。测试用 `MonoShaper`。不要往渲染引擎里加布局。

布局原语是 `Sized`、`Padding`、`Align`、`Stack`、`Row`、`Column`、`Grid`、`Text`、`Image`、`PointerRegion`、`Decorated`、`Clip`、`Opacity`、`Scroll`、`Transform`。任何节点都可以 `set_focusable`。几何交给 Taffy，含 flex grow 和 grid。`Stack`、`Clip`、`Opacity`、`Transform` 是一格的 grid，子节点放在第 1 行第 1 列。`Scroll` 也是一格 grid，轨道是 `max_content`：视口是边框盒，子节点按完整尺寸布局，可以高出视口。`set_scroll_offset` 不进 Taffy，只让下一帧的录制和命中平移。`Scroll` 的 `ScrollAxis` 缺省 `Both`。`Vertical` 和 `Horizontal` 只在该轴上认领、滚动和回弹；另一轴的 limits、offset、overscroll、fling、snap、滚轮和 `scroll_into_view` 恒为 0。`ScrollAccept` 缺省 `Content`（声明轴上还有余量才认领）。`Always` 到边也认领。`Never` 把这次拖拽让给祖先。`lock_at_edge` 只留给 `Both`。`Gesture::DragBy` 仍带原始两轴增量，跨轴分量在 `set_scroll_offset` 丢掉。`ScrollController` 对单轴滚动丢掉跨轴增量，不交给父级；`Both` 且该轴没有内容时，溢出仍交给父级。`ScrollArea` 缺省 vertical 和 Always，`VirtualList` 固定 vertical，横向分页显式传 horizontal。裸 `scroll()` 仍是 Both 加 Content。细节在 `docs/scroll-axis.md`。扁平列表减去 offset，picture 保留内容局部坐标，由 `Offset` 层平移。命中查询加上 offset，并用 `PushClip` 裁到视口。非单位 `Transform`（含纯平移）是 `Transform` 层，扁平列表里是 `PushTransform` … `Pop`。`set_justify` 和 `set_wrap` 写行或列的主轴对齐和换行。`Text` 和 `Image` 用 measure 回调报告尺寸。`Decorated` 按子节点定尺寸，命中交给子节点，并录一条盖住边框盒的矩形，圆角记在命令上。宽或高为 0 时不录矩形。扁平列表里，开层的节点以及写成命令的 `Clip` 仍在子节点前后成对 `Push` / `Pop`，盒子为 0 也成对。alpha 为 255 和单位 `Transform` 不推这组命令。

当前行为以源码和 `docs/architecture.md` 为准。未落地的目标以 `docs/refactor-plan.md` 为准。进程内的 `protocol`、`core`、`engine`、`hosts/macos` 不手写 C 或 Objective-C。QuickJS 的胶水 C 只在 `runtimes/quickjs`。变更缓冲的 C 布局由 `runtimes/quickjs/abi.yml` 生成，不把现有 setter 逐个包成头文件。计数器的 Solid 产物由 QuickJS 加载。iOS 宿主等预构建矩阵有这一列再写。

不要在 `frameworks/solid-renderer` 或 `frameworks/kotlin-ui` 里实现第二套 measure / layout。

## 必须守住的边界

- 产品代码只在 `third_party/` 以外。`third_party/solid` 和 `third_party/kiteui` 是本地的上游 git 检出，保持它们自己的历史，不提交进本仓库。克隆命令在 `third_party/README.md`。不在里面加功能，不把它们并进产品的构建。Solid 是 `next` 分支（Solid 2.0）。KiteUI 是 `version-8.3`。KiteUI 的 signal 在外部库 `com.lightningkite:reactive`，这里没有带进来。
- `third_party/kiteui/AGENTS.md` 描述的是上游 KiteUI。它和本文件冲突时，产品决策以本文件为准。
- 根目录 `settings.gradle.kts` 只有 `rootProject.name`，没有 `include`。它挡住 Gradle 从仓库根向上找到用户家目录里的工程。Kotlin 工程在 `frameworks/kotlin-ui`。始终用 `frameworks/kotlin-ui/gradlew -p frameworks/kotlin-ui`。不要把 Kotlin 模块写进根 settings，也不要做覆盖所有语言的单一 pnpm 或 Gradle 根。
- 没有 Impeller SDK 时 `Engine::create_metal` 返回 `Status::NotLinked`。没有 C stub。
- 不要提交 `third_party/impeller-sdk/`、`target/`，也不要解压约 1GB 的 `libimpeller.a`。`engine/fetch-impeller-sdk.sh` 按 `engine/engine.pin` 把每个平台解到 `third_party/impeller-sdk/<平台>/`，只留 `include/*`、动态库和 `LICENSE.sdk.md`。darwin 的 install name 是 `@rpath/libimpeller.dylib`。Android 的 SONAME 是 `libimpeller.so`。Cargo 的输出在 `target/`。
- 节点的 `x, y` 是父相对坐标。`parent` 只表示结构，渲染引擎不加父节点变换。`Tree::rect` 返回场景坐标。`LAYER` 不产生绘制调用。
- 宿主不保存 UI 状态，不创建按钮，不解释 `count`，也不做命中测试。它把指针入队，并把 `LayerTree` 交给渲染引擎。`just host` 每帧把队列交给会话。命中、手势和绘制录制在 `core`。场景格式在 `protocol`。Impeller 绘制在 `engine`。布局只调用 Taffy。Solid 渲染器和 Kotlin DSL 只改元素树。不要给 `protocol`、`core`、`engine`、`hosts/macos` 再加 C 或 Objective-C。QuickJS 的胶水 C 只留在 `runtimes/quickjs`。
- `register_font`、`submit_layers`、`draw_metal`、`draw_vulkan` 只在渲染线程上调用。`Engine` 是 `!Send`。Impeller 的 typography context 不能离开这条线程。`CAMetalLayer` 在主线程创建并挂到视图上；渲染线程只用层指针取 drawable。Metal context 仍用 `MTLCreateSystemDefaultDevice()`。Android 的 `ANativeWindow` 由平台线程持有，要等渲染线程应答 `SurfaceDropped` 之后才 `release`。等待在 `uifw-surface-drop` 上阻塞于 `DropWait`，平台线程不等待。

## Monorepo

```
protocol/                  DrawList、LayerTree
core/                      渲染树。Taffy、等宽排版、命中、click
engine/                    渲染引擎。只读 DrawList，调用 impeller.h
session/                   会话 trait。宿主只看见 metrics、pointer、frame、lifecycle
hosts/hostshell/          命令队列、帧调度、渲染线程
hosts/macos/               macOS 窗口。指针入队、CVDisplayLink、UIFW_DUMP
hosts/android/             Android 宿主。`HostActivity`、`HostView`、Choreographer、Vulkan。`just android` 打 APK
hosts/ios                  还没有。预构建矩阵没有 iOS 列之前不写；接口不按两台宿主收死
runtimes/quickjs/          quickjs-ng 会话。包名 `quickjs`。没有源码时返回 NotLinked
frameworks/modules/        宿主对象类型。Linking、FileSystem、AsyncStorage、Permissions
frameworks/solid-renderer/ 接到 core 的运行时。不做布局
frameworks/solid-ui/       Button、Field、Dialog、theme、overlay
frameworks/solid-router/   路径、栈和 RouterView
frameworks/net/            createFetch
frameworks/form/           createForm
frameworks/i18n/           formatNumber、formatDate、plural
frameworks/permissions/    permission_handler 形状的权限 API
frameworks/kotlin-ui/      Kotlin Gradle 根。路线暂缓，见 docs/refactor-plan.md
apps/gallery-solid/        Solid 应用。源码 counter.tsx，产物 dist/counter.js、dist/counter.esm.js
third_party/solid          solidjs/solid，分支 next。本地检出，不进 git
third_party/kiteui         lightningkite/kiteui，分支 version-8.3。本地检出，不进 git
third_party/impeller-sdk/  本地 SDK，不进 git
third_party/quickjs/       quickjs-ng 源码，不进 git
.agents/skills/             跟随仓库的 agent skill。Rust 编码以 rust-best-practice 为准
docs/architecture.md       当前仓库划分和各层结构
docs/platform-hosts.md     macOS、Android、iOS 的平台差异。iOS 宿主还没写
docs/refactor-plan.md      生产级迭代规划。未落地
docs/protocol-v2.md        Picture 字节。未排期，只在 Rust 里做
```

原生构建是 Cargo，输出在 `target/`。根上没有 CMake。`runtimes/quickjs` 在 workspace 里。没有 `third_party/quickjs` 时不编译解释器。`just quickjs` 按 pin 下载源码。

Kotlin：Kotlin 2.3.20，Gradle wrapper 9.2.1，JVM target 21，group `dev.uifw`。`settings.gradle.kts` 里只有 `include(":library")`。界面 DSL 还没写。

Solid：包名 `@uifw/solid-renderer`，`type: module`。`just solid` 先在 `frameworks/solid-renderer` 里 `pnpm install --frozen-lockfile`，再跑 renderer、`net` 和 `i18n` 的测试。`solid-router`、`form`、`solid-ui` 在各自目录 `pnpm install --frozen-lockfile` 后跑单测，并用 `frameworks/register.mjs` 把包名指到仓库里的 TypeScript 源。`hostRendererOptions()` 对齐 `@solidjs/universal` 的 `RendererOptions`。宿主标签是 `row`、`column`、`stack`、`align`、`padding`、`decorated`、`pointer`、`clip`、`opacity`、`scroll`、`transform`、`image`、`sized`。`Text` 是组件。编译在 `apps/gallery-solid/build.mjs`：

```js
solidPlugin({
  moduleName: "@uifw/solid-renderer",
  generate: "universal",
})
```

依赖钉 `solid-js` 和 `@solidjs/universal` 的 `2.0.0-rc.17`。`just solid-app` 安装依赖、重编 `dist/counter.js`，再用 mock host 跑一遍。`just check` 不跑这一步。

## 绘制

渲染引擎是 `engine::Engine`。它接收 `protocol::LayerTree`，不解析场景字节，也不依赖 `core`。

- `Engine::create_metal`：没有 SDK 时返回 `Status::NotLinked`
- `register_font`：拷贝字体字节。宿主默认注册 `/System/Library/Fonts/Supplemental/Arial.ttf`，family 名 `UIFWSans`
- `submit_layers`：`LayerTree`、`point_scale`、图片回调。`point_scale` 把一个场景单位乘成像素。回调的 source 是 `&str`。RGBA 活到 `submit_layers` 返回
- `draw_metal`：系统默认设备上的 `CAMetalLayer` drawable，指针是 `*mut c_void`
- `create_vulkan` / `draw_vulkan`：只在 Android 目标上编译。swapchain 由宿主的 `Surface` 持有，尺寸变化不丢链

成功是 `Result::Ok`。失败是 `Status::NotLinked`、`BadScene`、`NotReady`（还没有 drawable、context 或 display list，或者 present 失败）。空文字、缺图、非正缩放是 `BadScene`。

Impeller 句柄的所有权以刚解压的 `impeller.h` 为准。绘制在 `engine/src/painter.rs` 和 `engine/src/player/ops.rs`，文字在 `engine/src/typography/`，picture 在 `engine/src/pictures.rs`，纹理在 `engine/src/textures.rs`。不要按「C++ 分配的内存都由 Rust 释放」处理。

- 函数名以 `New` 结尾，头文件写明用对应的 `Release` 释放：调用方持有一次引用，用完释放一次。context、typography、paint、path、texture、paragraph、display list、surface、swapchain、glyph info、color source、image filter 属于这一类。
- 取得函数没有 `New`，头文件写明结果缓存在已有对象上：指针是借来的。读出数字后丢掉，不要 `Release`。`ImpellerParagraphGetLineMetrics` 的行高缓存在段落上，段落释放时由 Impeller 释放。`Engine::lines` 把基线、宽高和码元区间抄进 `LineMetric`。同一段再查一次仍用这份缓存。
- `ImpellerMapping.on_release` 在头文件里可空。`register_font` 仍传入非空的 `keep_font_bytes`：创建字型失败时 Impeller 也会调用回调，空指针会崩溃。回调不释放字节。成功时字节留在 `Typography` 的 `FontRegistry`，typography context 在 `Typography::drop` 里释放，先于这些字节。纹理走 `ImpellerTextureCreateWithContentsNew`。没有回调时，头文件允许 Impeller 在调用返回前拷贝像素；像素活到这次调用返回，`on_release` 保持空。不要把调用方的缓冲交给一个会事后释放的回调。

升级 `engine/engine.pin` 时，对每个新返回的句柄再读一遍头文件，按前两条归类，再改绘制和文字模块。

`Fill` 在缩放之前铺满画布。缩放只做一次，放在第一条非填充命令之前。渲染引擎使用命令里的 family、字号、字重、对齐、方向和 `max_width`，字号不乘 `point_scale`。左上角放在命令的 `(x, y)`。绘制用 paint 槽里带前景色的段落。布局测量命中同一槽。光标用度量缓存。相同文字、字号、宽度、颜色和样式跨帧复用，命中不复制整段文字。纹理按 `source` 跨帧复用，资产在进程内不可变。成功提交结束时只释放当前显示列表没有引用的纹理和段落，以及本帧没有 retain 的 `refs == 0` 测量槽。`register_font` 使旧段落在下一次提交释放显示列表之后失效。上屏失败是 `PresentFailed`，宿主保留表面并再要一帧。还没有表面或显示列表是 `NotReady`。图片要调用方给出紧排 RGBA8888。Impeller 不接受 PNG 或 JPEG。`source` 以 `asset:` 开头。macOS 把它映射成启动参数里的资产目录下的 `<name>.png`，默认目录是 `hosts/macos/assets`。Android 映射成 APK 里的 `assets/images/<name>.png`。缺图仍是 `BadScene`。

窗口标题 `uifw counter`，内容默认 360×640，可关闭、可最小化、可缩放。缩放和改尺寸都投递 `Metrics`。`contentsScale` 和 `drawableSize` 来自窗口所在屏幕的 `backingScaleFactor`（取不到时用 2），尺寸变化时按内容视图重算。这一层 `framebufferOnly = NO`，这样 `UIFW_DUMP` 能读回纹理。

SDK pin 在 `engine/engine.pin`：Flutter `77f043bc63196b9979cc09c5b97bb6d9b00be34d`，平台 `darwin-arm64`。换提交前先对 URL 做 HEAD，确认返回 200，再写进 pin。

## 构建和测试

工具：稳定版 rustc（`rust-toolchain.toml`）、Cargo、just、JDK 21、仓库内的 Gradle wrapper、带类型剥离的 Node（`--experimental-strip-types`）。macOS 宿主需要 Xcode 和本机 arm64。

新机器先取 SDK，再跑检查：

```sh
engine/fetch-impeller-sdk.sh
just check
```

`just check` 先 `cargo test --workspace`，再跑 Kotlin 的 `:library:jvmTest`，以及 Solid 的宿主树测试。它不会打开窗口。没有 quickjs-ng 源码时 QuickJS 测试走 `NotLinked`。有源码时会编译解释器并跑编译后的计数器。没有 SDK 时 `Engine` 返回 `NotLinked`，图形测试不编译；Kotlin 和 Solid 不需要 SDK。`just host` 需要 SDK。

GitHub Actions 工作流是 `.github/workflows/check.yml`。它跑 `cargo fmt --all -- --check`、Linux 上用 `RUSTFLAGS=-D warnings` 做 `cargo check --workspace --exclude macos`、Linux 上的 `cargo test --workspace --exclude macos`、macOS 上的 `cargo test -p macos`，以及和 `just kotlin`、`just solid` 相同的检查。`macos` 这个 crate 只在 macOS 上编译。工作流不下载 Impeller SDK，也不下载 quickjs-ng，因此渲染引擎走 `NotLinked`，QuickJS 测试同样走 `NotLinked`。工作流不装 crapper 和 dryer。窗口、`just android` 和 `just solid-app` 不在这套检查里。

| 命令 | 作用 |
|---|---|
| `just check` | `cargo test`，然后 Kotlin 和 Solid。不启动 GUI |
| `just test` | `cargo test --workspace` |
| `just quality` | crapper、dryer，再用 `-D warnings` 做 `cargo check --workspace`。需要本机有 crapper 和 dryer；CRAP 读 `just coverage` 写下的 LCOV |
| `just coverage` | `cargo llvm-cov` 把 LCOV 写到 `target/coverage/rust/lcov.info` |
| `just kotlin` | Kotlin `:library:jvmTest` |
| `just solid` | renderer、net、i18n，以及 solid-router、form、solid-ui 的单测 |
| `just solid-app` | 安装依赖，重编 `dist/counter.js` 和 `dist/counter.esm.js`，再用 mock host 跑渲染 |
| `just host` | 打开 macOS 窗口并加载 `apps/gallery-solid/dist/counter.js`。需要 SDK、quickjs-ng 源码和 `hosts/macos/assets` |
| `just quickjs` | 按 `runtimes/quickjs/quickjs.pin` 下载 quickjs-ng 源码 |
| `just host-js` | 同 `just host` |
| `just android` | 编 `aarch64-linux-android` 的 `libuifw.so`，再打 APK。需要 NDK 和 `android-arm64` SDK |
| `UIFW_DUMP=/tmp/uifw-frame.png just host` | 画出第一帧，写成 PNG，然后进程退出 |

单独跑 Gradle 时也要钉住工程目录：

```sh
frameworks/kotlin-ui/gradlew -p frameworks/kotlin-ui :library:jvmTest
```

看画面用 `UIFW_DUMP` 的读回。窗口进程用完要退出。dump 模式会自己终止；普通 `just host` 会留下窗口，验证完结束掉它。

## 改代码时

- 渲染引擎调用在 `engine/src/painter.rs`、`engine/src/player/`、`engine/src/typography/`、`engine/src/pictures.rs` 和 `engine/src/textures.rs`，对照 `impeller.h`。没有 stub。没有 SDK 时走 `Status::NotLinked`。句柄按「绘制」里的 Impeller 规则释放：`New` 且写明 `Release` 的用完释放一次；缓存在段落上的 `ImpellerParagraphGetLineMetrics` 不要 `Release`；`register_font` 的 `on_release` 必须非空。
- 动到 pin 或 `impeller.h` 的调用，保持 `IMPELLER_VERSION` 来自刚解压的那份头文件，并按「绘制」重新归类新句柄。fetch 脚本继续改 darwin 的 install name，并核对 Android 的 SONAME。`engine` 和 `hosts/macos` 的 `build.rs` 都要把 rpath 指到 `third_party/impeller-sdk/darwin-arm64/lib`。Android 目标链接 `third_party/impeller-sdk/android-arm64/lib`。
- 给宿主加第二种图片时，在解析回调里按 source 区分，并把像素保持到 `submit` 返回。
- 渲染树存储父相对坐标，`Tree::rect` 返回场景坐标。不要为了对齐调试文本去改录制路径。
