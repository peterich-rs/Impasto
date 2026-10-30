# 架构

这是当前代码的结构。未落地的迭代规划在 `docs/refactor-plan.md`。这里和代码不一致时，先改已经跑起来的那一边。

## 仓库怎么切

一种语言一个绑定，一套 UI 核心。我们写的原生代码用 Cargo 编成 Rust。Kotlin 只用 `frameworks/kotlin-ui` 里的 Gradle。JS 以后可以有只包含前端包的 `pnpm-workspace.yaml`，不把 Kotlin 和 `third_party/solid` 算进产品构建。根上的 `settings.gradle.kts` 继续保持没有 `include`。

场景格式、渲染树、渲染引擎、会话、`hostshell`、macOS 宿主和 Android 宿主已经是 Rust。不要往 `protocol`、`uicore`、`engine`、`hosts/macos` 加手写的 C 或 Objective-C。Android 的 NDK 调用留在 `hosts/android`。QuickJS 的胶水 C 只在 `runtimes/quickjs`。嵌入用的变更缓冲由 `runtimes/quickjs/abi.yml` 生成。

```
protocol/                     DrawList、LayerTree
uicore/                      渲染树。Taffy、等宽排版、命中、click、drag、焦点
engine/                       渲染引擎。读 LayerTree，调用 impeller.h
session/                      会话 trait。metrics、pointer、frame、wants_frame、lifecycle
hosts/macos/                  macOS 窗口。指针入队、CVDisplayLink、IMPASTO_DUMP
hosts/hostshell/              命令队列、帧调度、渲染线程
hosts/android/                `HostActivity`、`HostView`、Choreographer、Vulkan。库名 `host`
hosts/ios                     还没有。预构建矩阵没有 iOS 列之前不建这个目录
runtimes/quickjs/             quickjs-ng 会话。包名 quickjs
frameworks/clipboard/         @impasto/clipboard。host 由 QuickJS 物化
frameworks/linking/           @impasto/linking
frameworks/app-state/         @impasto/app-state。currentState 是门面上的 getter
frameworks/file-system/       @impasto/file-system
frameworks/async-storage/     @impasto/async-storage
frameworks/solid-renderer/    接到 uicore 的 universal 运行时。directionFor、spring、onBack
frameworks/solid-ui/          Button、Field、Dialog、theme、overlay
frameworks/solid-router/      路径、栈和 RouterView
frameworks/net/               createFetch
frameworks/form/              createForm
frameworks/i18n/              formatNumber、formatDate、plural
frameworks/permissions/       permission_handler 形状的权限 API。五法来自 @impasto/permissions/host
frameworks/kotlin-ui/         Gradle 根。DSL、控件、以后的 cinterop
  library/
apps/gallery-solid/           Solid 应用。源码 counter.tsx；ESM 产物 dist/counter.js 由 `just gallery-dist` 本地生成，不进 git
third_party/                  只读对照和本机 SDK
```

依赖只允许向下：

```text
gallery-solid ──► solid-ui ──► solid-renderer ──► quickjs 会话 ──► uicore ──► protocol
gallery-solid ──► solid-router ──► solid-renderer
gallery-solid ──► solid-router ──► linking
hosts/macos ──► hostshell ──► session ──► uicore ──► protocol
hosts/macos ──► engine ──► protocol
hosts/android ──► hostshell ──► engine ──► protocol
hosts/macos ──► quickjs 会话          `just host` 加载 `counter.js`
hosts/android ──► quickjs 会话        加载 assets 里的 `counter.js`
```

`uicore` 不包含 QuickJS、Kotlin、AppKit 或 Impeller 的头文件。它通过 crates.io 依赖 Taffy。Impeller 排版由宿主实现 `Shaper` 并调用 `engine`，测试构建不链它。`engine` 不包含 `uicore`。语言包不实现测量和布局。选型表在 `AGENTS.md` 的「优先复用已有实现」。

两个产品二进制以后都链同一套 `uicore` 和 `engine`：macOS 上 QuickJS 宿主是 `hosts/macos`，Kotlin/Native 宿主是 `frameworks/kotlin-ui` 编出来的可执行文件。宿主按操作系统写，会话按语言写。一个可执行文件只链一种会话。

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
  | ⑨ submit 层树  |                |                  |
  |──────────────────────────────────────────────────►|
  | ⑩ present      |                |                  |
  |◄──────────────────────────────────────────────────|
```

`frame` 的顺序是排空 job、`onFrame(targetMs, intervalMs)`、再排空一轮、扫掉本轮摘下且没有再挂上的子树、布局。布局机会之后，在 Rust 里跑已注册的 `ListDriver` 和 `ScrollController` 的 `tick`，把可见集变更经 `onDriver` 交给脚本并冲刷。tick 或这次回调把树弄脏，就再布局。然后才调用 `onLayout`。`onLayout` 再冲刷它写进来的变更，需要的话再布局一次，然后录制。`boot` 登记的 `onFrame` 先 `flush`，再跑监听。`onLayout` 先跑监听，再 `flush`。树不脏、也没有可见集变更时，沿用上一份层树，并且不调用 `onLayout`。driver 还有未测完的行高时，会话自己再要一帧。job 轮数有上限，超过则记入 `script_error` 并打日志。`Session::semantics` 在布局之后交出语义树；没有变化时宿主不重复投递。

## 绘制命令

进程内的绘制结果是 `protocol::DrawList` 和 `protocol::LayerTree`。没有场景字节。命令是 `Fill`、`Rect`、`Text`、`Image`、`PushClip`、`PushOpacity`、`PushTransform`、`Pop`。`Rect` 带 `corner_radius`，0 是直角。`PushClip`、`PushOpacity`、`PushTransform` 和 `Pop` 成对嵌套，三组开命令共用同一个 `Pop`。渲染引擎校验这套栈，不平衡的列表是 `BadScene`。`protocol` 不校验。picture 里的坐标相对该 picture 的原点。文字命令带 `font_size`、`max_width` 和颜色。布局用的约束、flex、回调不进命令。`max_width` 是测量时用的行宽，不是排版之后的行宽。文字命令还带段落样式，缺省是左对齐、字重 400、从左到右、不限制行数。`PushTransform` 的顺序是平移、顺时针旋转、等比缩放。旋转单位是弧度，渲染引擎换成度再交给 Impeller。缩放和旋转绕当前原点。命中测试按同样的顺序做逆变换。`Clip` 的圆角会拒绝圆角外的点。

`uicore::Tree::record` 交出扁平命令，同时组出 `LayerTree`。宿主提交层树。扁平命令是场景坐标。picture 里的命令相对该 picture 的 `origin`。

一段 picture 属于一个 owner。owner 是根，以及本帧开层的节点：`Scroll`、alpha 在 1 到 254 的 `Opacity`、非单位 `Transform`（含纯平移）、子树里还有开层节点的 `Clip`，以及 blur、backdrop 或非 `SrcOver` 的 blend。没有开层后代、自身也没有这些效果的 `Clip`，把 `PushClip` / `Pop` 写进父 picture，不单独开层。alpha 为 0 的 `Opacity` 跳过子树。alpha 为 255 的 `Opacity` 和单位 `Transform` 并入父 picture。`Scroll` 另外缓存一张 child picture，滚动偏移在 `Offset` 层上。平移、旋转、缩放和半透明记在层上。层上的数值变化不重建 picture。节点内容变了，脏标记停在最近的 owner 上，只重建那个 owner 的 picture。

## UI 核心

内部是 Rust 渲染对象，类型是 `uicore::Tree`。节点 id 是槽位加代数，`to_raw` 是 `u64`，高位代数从 1 起、不超过 `2^21 - 1`，这样 JS 的 Number 还能精确表示。删掉的槽位代数对不上，旧 id 得到 `UnknownNode`。代数到顶的槽位不再复用。根是显式的：第一棵挂上的树，或第一个创建的节点；`layout`、`record`、命中都读这个根。`remove` 摘掉子树，Taffy 节点、段落槽和 picture 都留着，并把 id 放进待销毁名单。同一轮里再 `insert` 会把 id 从名单拿掉，Taffy 还在，所以不 `revive`。轮末仍没有父节点的，`discard_detached` 才释放 Taffy 并退役 id。`detach` 同样摘链，但不进名单，壳可以跨帧再挂上。`discard` 要求节点没有父节点，后序退掉子树槽位，旧 id 变成 `UnknownNode`。摘链时，落在这棵子树里的焦点范围会从栈里去掉。空字符串不进 `DrawList`。`layout` 调用 `TaffyTree::compute_layout_with_measure`。Taffy 的 `location` 相对父节点，`place` 把它累加成场景绝对坐标写进 `x, y`。`record` 用这些绝对坐标。扁平列表减去滚动偏移。picture 里的命令相对 `origin`，滚动由 `Offset` 层平移。渲染引擎不解释父节点变换。`record` 同时交出扁平 `DrawList` 和 `LayerTree`。`set_viewport` 和样式、文字的 setter 只标脏，不丢掉已经算好的几何。挂上或摘下节点会让几何失效。摘掉子树只取消落在该子树里的指针路由。

`Row`、`Column`、`Grid`、`Padding`、`Align`、`Sized`、`Stack`、`Clip`、`Opacity`、`Scroll`、`Transform` 只是 `taffy::Style` 的不同设值：`Display::Flex`、`Display::Grid`、padding、对齐、宽高、gap、flex grow、grid template。`Stack`、`Clip`、`Opacity`、`Transform` 是一格的 grid，子节点放在第 1 行第 1 列。`Scroll` 的轨道是 `max_content`，`overflow` 是 hidden，最小尺寸是 0，这样父约束可以把视口压得比内容矮，子节点仍按完整尺寸布局。`set_scroll_offset` 只标绘制，不进入 Taffy。`layout` 在只有绘制变更时直接返回。`record` 和命中应用 offset。松手后的惯性由 `Tree::tick_motions` 采样 `Friction`，并在 offset 变化时发出 `Gesture::Scroll`。每个 `Scroll` 有一份 `ScrollPhysics`（衰减 `Decay`：`Tau` 时间常数或 `Rate` 减速率，`Rate(0.998)` 等价约 500ms；拉伸 `Band`：`Linear` 线性系数或 `Curve` 渐近带 `D(1 − 1/(cx/D + 1))`，拉伸永远到不了 `D/c`；回弹或夹紧；分轴吸附；吸附弹簧 `snap_stiffness` / `snap_damping` 独立于回弹弹簧；`accept`）和 `chain`，以及 `ScrollAxis`（`Vertical`、`Horizontal`、`Both`）。`ScrollPhysics::IOS` 是一个预设：`Rate(0.998)`、`Curve(0.55)`、偏硬的分页弹簧。轴缺省 `Both`，`accept` 缺省 `Content`。缺省值与原先的 180ms、线性系数 0.55 相同。速度环只取松手前 120ms 内的样本，早前的甩动不参与。`set_pixel_scale` 记录设备像素比，滚动 motion 结束时把 offset 吸附到整设备像素；会话从 `Metrics.device_pixel_ratio` 转发。`scroll_limits` 把未声明的轴收成 0。`set_scroll_offset` 丢掉该轴的偏移和 overscroll。fling、snap、滚轮和 `scroll_into_view` 只动声明轴。`chain` 缺省为真：惯性只沿同一轴交给还能吃下这段速度的祖先；竖向 fling 不会推动横向祖先。`chain` 为假时停在自己的边上。认领在 `Tree::scroll_for`：声明轴只看自己的分量，`Content` 要有余量，`Always` 到边也认领，`Never` 跳过。`Both` 仍用较大的分量当主轴，无人能消费时 `lock_at_edge` 锁最内层溢出者。单轴不用这个兜底。取舍见 `docs/scroll-axis.md`。中途顶到边的剩余位移由 `consumeScrollOverflow` 交给父控制器，父级吃不下时内层继续回弹。吸附在摩擦结束后用弹簧落到最近一格，这颗弹簧自己的结束不再吸附。相位只在 `Idle`、`Dragging`、`Coasting`、`Settling` 切换时发 `Gesture::ScrollPhaseChanged`。`set_justify` 写行或列的 `justify_content`，`set_wrap` 写 `flex_wrap`。`set_fill` 把宽高设成父级或视口的 100%，最小尺寸为 0。`set_grow` 写子节点的 `flex_grow`，值必须非负，并标脏。flex 父节点有确定的主轴尺寸时，正的 grow 分到剩余空间。仅有 viewport 时，宽度为 auto 的 flex 根仍按内容收缩，不会出现剩余空间。`Text` 和 `Image` 走 measure 回调。文字回调调用 `Shaper`；`AvailableSpace::MinContent` 时改调 `Shaper::min_content_width`，按最宽不可断词计宽，grid 的 `min_content` 轨道和滚动内容下限因此是真值。图片回调返回固有尺寸。`set_min_axis` / `set_max_axis` 接受 `AxisSize`，`Fraction` 写百分比的最小或最大尺寸；百分比解析要父容器在该轴有确定尺寸，内容自适应的列解析不了子项的百分比最小高度。`set_panel` 把节点标成独立重排边界，`measure_panel` 用与 `layout` 同一条 Taffy 路径在 `SubtreeSpace`（自然尺寸、带帽自然尺寸、确定宽度）下单独排子树并返回内容盒，弹层先量后摆，不再渲染一帧再读 rect。Taffy 不绘制，也不做命中测试。

| 对象 | Taffy 负责 | 绘制 |
|---|---|---|
| `Sized`、`Padding`、`Align`、`Stack`、`Row`、`Column`、`Grid` | 样式和子节点的框 | 容器本身不绘制。`Stack` 的子节点从底到顶画 |
| `Decorated` | 一列子节点，间距 0 | 边框盒一条矩形，圆角记在命令上。命中交给子节点 |
| `Clip` | 与 `Stack` 相同的一格 grid | 子树开层时是 `Clip` 层，否则 `PushClip` … 子节点 … `Pop` 在父 picture 里。盒子为 0 也成对 |
| `Opacity` | 与 `Stack` 相同的一格 grid | alpha 在 1 到 254 时是 `Opacity` 层。255 并入父 picture。0 跳过子树 |
| `Scroll` | 一格 `max_content` grid。边框盒是视口 | `Clip` 层、`Offset` 层，和一张 child picture。扁平列表是 `PushClip` … 按 offset 平移的子命令 … `Pop`。offset 不进布局 |
| `Transform` | 与 `Stack` 相同的一格 grid | 非单位变换（含纯平移）是 `Transform` 层。单位变换并入父 picture。命中把查询点按平移、顺时针旋转、等比缩放的逆变换送进子节点 |
| `Text` | measure 回调给出段落宽高 | `DrawParagraph`。命令带字号、行宽约束、颜色和段落样式 |
| `Image` | measure 回调给出固有尺寸 | `DrawImage` |
| `PointerRegion` | 尺寸等于子节点 | 不绘制。命中后接收手势 |

文字测量是 `Shaper`。测试实现是 `MonoShaper`：一个 Unicode 标量前进 `font_size * 0.5`，行高是 `font_size`，放不下就换行，一行至少一个标量。布局测试因此不链 Impeller。布局测量调用 `Shaper::measure_paint`，前景色写进段落。`sync_paragraphs` 只处理 `TEXT_DIRTY` 的节点，命中时 `retain_measured` 复用这次建的槽，不再 shape。光标和选区仍走 `Shaper::metrics`，对应 `Typography::measure_paragraph` 的度量缓存。键里有颜色。`set_scroll_offset` 不改变 `layout_point`。这一帧若没有 `layout`、也没有语义结构变化，`semantics_patch` 只更新滚动节点的偏移。`subtree_layers` 记的是子孙有没有开层；节点自己开层只体现在递归的返回值里。

命中测试按绘制顺序从最后一个子节点往前。宽或高为 0 的节点跳过。`Text` 和 `Image` 是 `Opaque`，命中盒子就收下。布局容器、`Decorated`、`Clip`、`Opacity`、`Scroll`、`Transform` 是 `Defer`，只看子节点。点必须落在每个祖先盒子内，所以裁剪框外的子节点不可命中。`Clip`、`Text`、`Image`、`PointerRegion` 的圆角会拒绝角上的点。`Scroll` 先用视口盒子接纳这个点，再把查询点加上 offset 交给子树。`Transform` 把查询点按逆变换送进子树。`PointerRegion` 是 `Region`，没有子节点接住时收下自己。

点击和拖拽都在 `uicore` 的 `Tree::pointer`。按下时做命中测试，记住最近的 `PointerRegion`，并记住最近的 `Scroll` 祖先。`PointerEvent.time_ms` 是宿主事件时间，毫秒。位移不超过 8 点时，按下状态保持，抬起仍在区域内就先 `Release` 再 `Click`。位移超过 8 点时，声明了拖动的 `PointerRegion` 先锁定成选区拖动。否则沿父链找声明轴上还能滚动的 `Scroll`。`Both` 用较大的分量当主轴，贴边且无人能消费时锁最内层溢出者。单轴只看自己的分量，纯跨轴拖动不认领，到边是否认领由 `ScrollAccept` 决定。一个 `Scroll` 同时只锁定一根手指。锁定后先 `Release` 再 `DragBegin`，随后每次移动是 `DragBy`（场景点增量，两轴都保留）。跨轴分量在 `set_scroll_offset` 丢掉，不进偏移，也不进 overscroll。抬起是 `DragEnd`（场景点/毫秒，取最近两次 `Move`；不足两次则为 0）。没有可滚动的祖先时，超过 8 点仍走原来的按下语义。`Cancel` 在拖拽锁定后补 `DragEnd(0, 0)`。树上不存 JS 函数或 Kotlin lambda。会话在借用结束后分发自己的监听器。监听器里不调用 `layout`。插入节点不会清掉已有的指针路由。摘掉一棵子树时，只取消目标落在这棵子树里的路由。

```text
Down 命中
  ├─ Move ≤ 8pt ─────────────── 积累，不转移
  ├─ Move > 8pt 且有 Scroll ─── Release，DragBegin，DragBy…，Up → DragEnd
  ├─ Move > 8pt 且无 Scroll ─── 保持 Press，移出则 Release
  └─ Up 仍在 Press 内 ───────── Release，Click
```

图片固有尺寸在创建 `Image` 节点时传入。宿主解码 RGBA，活到 `submit` 返回。降采样和色彩管理以后换宿主的解码，不改布局对象。

## 渲染引擎

渲染引擎只做光栅，用 Rust 调用 `impeller.h`。会话帧交给它 `protocol::LayerTree`。它不读渲染树，也不处理指针。没有 SDK 时返回 `NotLinked`。命令以 `protocol/src/draw_list.rs` 为准，层树以 `protocol/src/layer.rs` 为准，失败码以 `engine/src/status.rs` 为准。`Engine` 编排提交。`Typography` 持有字体、度量缓存和真色 paint 槽。`PictureCache` 缓存子显示列表，`TextureStore` 缓存图片。一次 `submit_layers` 里的 builder 和 save 栈在 `Painter` 上，函数返回就结束。宿主壳通过 `engine::Surface` 调用 `register_font`、`submit_layers`、`draw_metal` 和统计；构造和 `draw_vulkan` 仍是 `Engine` 的固有方法。

文字命令自带字号、行宽、颜色和段落样式。渲染引擎按这些字段画，不另设默认字号，也不把字号乘 `point_scale`。布局测量和绘制共用带前景色的 paint 槽。度量缓存另给光标和行框，键里仍有颜色。相同文字、字号、宽度、颜色和样式跨帧保留。查找用栈上的字段比较，命中不复制整段文字。成功提交结束时丢掉当前列表没有引用、且这一代没有再用的段落和纹理。`refs == 0` 的测量槽若本帧没有被 retain，也在这次提交之后经 `retire_paragraph` 释放。`register_font` 在下一次提交释放旧显示列表之后清掉段落缓存。上屏失败（`PresentFailed`）保留表面并再要一帧。`NotReady` 表示还没有表面或显示列表，壳层留着 `Surface` 再要一帧。`submit_layers` 在新的显示列表建完之前不放掉已经在屏幕上的那一份，并按 picture 缓存子显示列表。`point_scale` 只乘在根 builder 上。

第一条非 `Fill` 命令之前做一次 `point_scale`。`PushClip`、`PushOpacity` 和 `PushTransform` 也算这类命令，所以 `Save` 记下的矩阵已经含缩放，`Restore` 回到的仍是这份矩阵。`Pop` 不再缩放。半径不大于 0 时矩形走 `DrawRect`、裁剪走 `ClipRect`；更大的半径钳到短边的一半，再走圆角接口。`PushOpacity` 为这一层单独建 paint，颜色是白、alpha 来自命令，`Restore` 之后才释放。`PushTransform` 在 `Save` 之后依次 `Translate`、`Rotate`、`Scale`，`Pop` 走同一个 `Restore`。渲染引擎只用自己的 save 栈：多出来的 `Pop` 不调用 `Restore`；结束时栈非空则整帧 `BadScene`，并且不创建显示列表。失败收尾在释放 builder 之前先 `Restore` 再释放残留的 layer paint。

## 宿主

宿主做三件事：平台事件入队、vsync 上问会话要一帧、把 `LayerTree` 交给渲染引擎并上屏。会话边界是 Rust trait `session::Session`：`metrics`、`pointer`、`frame`、`wants_frame`、`lifecycle`。`wants_frame` 取走「再要一帧」的标志。`frame` 的顺序是处理已经入队的指针、`onFrame(targetMs, intervalMs)`、冲刷指针期间写过的状态、布局、driver tick、必要时再布局、`onLayout`、必要时再布局一次、录制，然后 `semantics`。

macOS 宿主把 `NSEvent` 换成 `uicore::PointerEvent` 放进队列。先把窗口坐标换进内容视图，再把原点改到左上。AppKit 给出的已经是点，不除 `backingScaleFactor`。`just host` 和 `just host-js` 都加载 `apps/gallery-solid/dist/counter.js`，每帧出队并调用会话。宿主不做 click。字体文件字节和 PNG 解码在宿主里，图片像素活到 `submit` 返回。宿主实现 `uicore::Shaper`，测量时借用 `Engine` 交出去的 `Typography`。借用在 `session.frame()` 返回前结束。

Android 宿主的平台线程是 Java 主线程。`HostActivity` 转发生命周期，`HostView` 接收指针、按键和 `InputConnection`，`HostSurfaceView` 把 `Surface` 交给渲染线程。坐标是物理像素除以 `DisplayMetrics.density`。每根手指一条指针。`onPause` 对应 `Lifecycle::Inactive`，`onStop` 对应 `Hidden`，`onResume` 对应 `Resumed`。UI 线程不调用 `FrameClock::cancel`。系统栏写进 `Metrics` 的 padding，键盘写进 `view_insets`。会话把它交给 `Tree::set_viewport`，根从左上 inset 排，可用空间扣掉四边。`destroy_surface` 与替换旧窗口的 `open_surface` 把 `NativeWindow` 交给 `surface-drop`。该线程在 `DropWait` 上等到渲染线程 `Acks::signal` 之后才 `ANativeWindow_release`。平台线程在 `spawn` 之后返回。

宿主不知道 `count`、`Button` 或信号。窗口尺寸变化只更新 metrics 并请求下一帧。`layer_epoch` 未变、缩放与 `installed` 相同、且 `needs_present` 为假时，跳过 `submit_layers` 和 `present`。`Resumed` 与 `Invalidate` 置 `needs_present` 并保留 `installed`，这一帧走 `PresentCached`：不重建显示列表，由 `draw_metal` 或 `draw_vulkan` 把已安装的列表画进新 drawable。present 返回 `PresentFailed`、`NotReady` 或 `BadScene` 时同样置 `needs_present`。连续失败未满 3 次时下一帧仍是 vsync；从第 3 次起记一条 `present failed; backing off`，改由 `Wake` 延迟重试，间隔从 16ms 翻倍，上限 1000ms，成功后计数归零。`MemoryWarning` 清掉 `installed` 并丢掉 `ImageCache` 里的像素，下一帧 `submit_layers`。会话申请了下一帧时，画面没变也再登记恰好一帧；`needs_present` 为假时这一帧仍可 `Skip`。

macOS 的窗口和 `CAMetalLayer` 在主线程，`Engine`、会话和 present 在渲染线程。Android 的 `ANativeWindow` 由平台线程持有，Vulkan swapchain 在渲染线程创建和释放。iOS 宿主还没写，预构建矩阵没有这一列；线程模型不按两台宿主收死，见 `docs/refactor-plan.md` 的暂缓一节。

## 运行时会话

quickjs-ng 会话拥有 `JSRuntime`、一棵 `uicore::Tree` 和一份区域 id 到 JS 函数的表。`glue.c` 注册全局 `core`。句柄是 `u64`，失败用返回码，`0` 只在这层表示没有节点。解释器按 `quickjs.pin` 的 tag 下载到 `third_party/quickjs`，不抄进 `runtimes/quickjs`。没有源码时 `open` 返回 `NotLinked`。

指针到达时，`Tree::pointer` 先返回被激活的区域，点击时再附上焦点转换。借用结束后会话调用全部匹配的 JS 函数。`Session::input` 只在 `focused()` 有值时派发到那个节点。监听器抛错时，文本留在 `take_script_error`，并经宿主日志打出，带上异常的 `stack`。页面 `Errored` 接住的渲染错误同样 `console.error` 到这条日志。`insert` 失败时宿主再记一行 `insert failed:` 和 `uicore` 的原因。这一帧不再派发其余区域。进程不退出。监听器只改树。会话不在这个回调里 `layout`。JS `remove` 先摘下子树，焦点若在这棵子树里先派发 `Blur`。同一轮脚本里这个 id 还能再插回去。脚本返回后，会话对仍没有父节点、且没有调用过 `detach` 的子树调用 `Tree::discard`，并按退掉的 id 释放监听。`detach` 把 id 移出这份名单，监听留着。`discard` 立刻退役，并立刻释放监听。

vsync 时先排空 QuickJS job，再触发到期的 `setTimeout` / `setInterval`，再跑 `requestAnimationFrame`，然后跑 `core.onFrame(targetMs, intervalMs)`。更晚的计时器由 `time` 投递 `Wake`，不空转 vsync。`fetch` 整包下载，`fetchStream` 分块：`net` 先回 `HttpHead`（状态和响应头），再逐块回 `HttpChunk`，末块带 `done`，出错带 `HttpError`（`kind` 是 `Network` / `Timeout` / `Canceled` / `TooLarge` / `BadUrl` / `BodyNotUtf8`，reject 值是 DOMException 名）。prelude 把流接成最小 `ReadableStream`（`getReader` / `read` / `cancel`）。`WebSocket`、`fetch` 和框架模块的结果从 `net` 回到渲染线程，在下一帧排空。HTTP 走宿主注入的 `Boot.http`；macOS 与 Android 注入 openwire `Client`，没有注入时错误文本是 `network is not linked`。`boot` 注册的回调先 `flush`，再跑通过 `onFrame` 登记的监听。回调失败留在 `take_script_error` 并记日志，绘制仍用上一帧，并且这次失败不再登记下一帧。需要布局时才 `layout`。这次 `layout` 跑完才调用 `core.onLayout`，监听写进树之后同一帧再布局一次。需要绘制时才 `record`。`FramePhases.layout_cb_ms` 是这次回调的时间，不并进 `script_ms`。`requestFrame` 让宿主在 present 之后再登记恰好一帧。全局还有 `console`、`performance.now`、`URL`、`TextEncoder`、`fetch`、`fetchStream`。`@impasto/<pkg>/host` 由 `glue.c` 用 `JS_NewCModule` 物化；查找走 `ModuleRegistry::by_specifier`。未注册抛 `module '%s' is not linked`。其它 `@impasto/*` 必须打进 bundle。一方模块的方法名和 kind 由 `cargo run -p hostshell --bin host-stubs` 写成各包的 `host.gen.ts`，hostshell 测试防漂移。公开参数和返回类型写在各包 `src/index.ts`。插件返回的大字节（≥4KB）零拷贝进 JS：字节留在会话的 slab，quickjs 的 realloc 钩子以 size 0 释放时回调 `release_bytes` 回收。

会话加宿主的一帧超过 vsync 间隔时，宿主打一行 `frame`。`script` 再拆成 `motion`（采样动画并取出滚动通知）、`dispatch`（把通知交给脚本并排空由此排队的 job）、`timer`、`raf`、`callback`（`onFrame`）、`flush`（变更缓冲）。`layoutcb` 是布局之后的 `onLayout`。`laywork` 是这一帧每一次 `layout` 的合计，含脚本里 `rect` 触发的那次；后面的 `layout` 只计 `frame` 末尾那一次。`x` 是进入 Taffy 的次数，紧接着的词是第一次把树标脏的原因：`none`、`viewport`、`style`、`text`、`structure`。`reuse` 是直接复用已有行框的测量，`shaped` 是重新调用排版器的次数。`ptr`、`key`、`edit`、`ime`、`scroll`、`focus`、`blur`、`gesture` 是这一帧的指针、按键、文本编辑、输入法收起、滚动通知、焦点、失焦和手势总数。这些计数在打出日志后清零。

`uicore` 上的函数——JS 全局名仍是 `core`：`column`、`row`、`stack`、`align`、`padding`、`pointerRegion`、`decorated`、`clip`、`opacity`、`scroll`、`transform`、`image`、`text`、`append`、`insert`、`remove`、`detach`、`discard`、`parent`、`firstChild`、`nextSibling`、`isText`、`setText`、`setFontSize`、`setColor`、`setGap`、`setPadding`、`setGrow`、`setRadius`、`setAlpha`、`setAlign`、`setCross`、`metrics`、`setSize`、`setImage`、`setMaxLines`、`setEllipsis`、`setTextAlign`、`setFontWeight`、`setTextDirection`、`setLineHeight`、`setFamily`、`setScrollOffset`、`scrollOffset`、`rect`、`setTransform`、`setJustify`、`setWrap`、`setFill`、`requestFrame`、`onClick`、`onPress`、`onRelease`、`onDragBegin`、`onDragBy`、`onDragEnd`、`onFrame`、`onLayout`。`decorated` 的半径可以省略，省略时是 0。内建 `image` 的 `source`、宽和高在创建时读入。`setImage` 把这三项一起改掉。`onDragBy` 和 `onDragEnd` 收到两个数，分别是位移增量和速度。`onKey` 收到 `"Backspace"` 这类名字，`onText` 收到已提交串，`onComposing` 收到组合串或 `null`。`measureText` 返回 `{w, h}`，走 `Tree::measure_text`。`setImeEnabled` 和 `setImeText` 只改会话里的两个标志，由宿主在帧末投递给平台。同一张 tap 表保存 click、press、release、drag、focus、blur、key、text、composing，摘掉节点时一起释放。

Kotlin 会话还没接上原生树。这条路线暂缓，见 `docs/refactor-plan.md`；重启时走 `runtimes/quickjs/abi.yml` 的变更缓冲，信号用已发布的 `com.lightningkite:reactive`。会话代码放在 `frameworks/kotlin-ui`，不放到 `runtimes/`，这样 Gradle 仍然只有一个根。

## 渲染器

`src/tree.ts` 的 `hostRendererOptions` 不依赖 Solid，把 universal 的方法接到 JS 全局 `core`。宿主标签是 `row`、`column`、`stack`、`align`、`padding`、`decorated`、`pointer`、`clip`、`opacity`、`scroll`、`transform`、`image`、`sized`。`align` 的对齐在创建时读入，之后 `setProperty` 走 `setAlign`。`cross` 走 `setCross`。`pointer` 的 `focusable` 走 `setFocusable`。`radius` 和 `alpha` 创建时读入，之后 `setProperty` 走 `setRadius`、`setAlpha`。内建 `image` 的 `source`、宽和高只在创建时读入。`Image` 组件把这三项一起交给 `setImage`。`grow` 走 `setGrow`。`justify`、`wrap`、`fill` 走 `setJustify`、`setWrap`、`setFill`。`transform` 的 `x`、`y`、`rotate`、`scale` 合成一次 `setTransform`。`onPress`、`onRelease`、`onFocus`、`onBlur`、`onKey`、`onText` 和 `onComposing` 跟 `onClick` 一样登记回调。`onDragBegin`、`onDragBy`、`onDragEnd` 登记在滚动节点上。`Text` 不是标签：universal 会把字符串子节点插到父节点下面，而 `uicore` 的文字是叶子。`Text` 自己建叶子，再用 effect 调 `setText`、`setFontSize`、`setColor`，以及最大行数、省略号、对齐、字重、方向、行高和 family。`boot` 独占 `core.onFrame`：先 `flush`，再调用 `onFrame` 返回的监听。`boot` 也独占 `core.onLayout`：先跑 `onLayout` 的监听，再 `flush`。`VirtualList` 在这次回调里读行高。传了 `recycle` 时，窗口里的行由 `Recycler` 复用 `Slot`：离窗 `detach`，超上限或卸载才 `discard`。`createSlot` 的 root 不挂在调用方的 effect 上。`render` 只跑一次，换内容写 `item` 信号。`onFrame` 返回取消函数，组件用 `onCleanup` 摘掉。抛错的监听会从这张表里移除。`requestFrame` 向宿主再要恰好一帧。

编译产物的 `moduleName` 是包入口 `runtime/index.ts`。它用 `createRenderer` 装配 `hostRendererOptions`。渲染器节点是 `{ id }`，不是裸数字。universal 把数字当成文本。`insertNode` 调 `insert`，移动由 Rust 处理。`onClick` 不进显示列表，闭包留在会话那张表里。

渲染器不保存 `x, y`，也不在 TS 里做命中测试。无窗口测试在 `quickjs` crate 里用 `MonoShaper` 断言编译产物上的点击。不要为了测试再写一套 TS 布局。

Kotlin DSL 还没写。接上以后保持 KiteUI 那种 lambda，落到同一套树操作：

```kotlin
column {
  text(count().toString())
  row {
    button("-") { count.set(count() - 1) }
    button("+") { count.set(count() + 1) }
  }
}
```

## 基础控件

布局原语只有 `uicore` 里那一批。语言包给它们薄薄的函数或组件，参数是样式和子节点，不写尺寸计算。

`Button` 写在 `frameworks/solid-ui`。结构是：

```text
PointerRegion      press / release / click
  Padding
    Decorated      抬起是按钮底色，按下换成按压色，圆角 6
      Text         标签
```

按下时控件把 `Decorated` 的填充色改成按压色。抬起时先恢复底色，再调用应用传入的 `onClick`。`pressed` 是控件自己的信号，不是渲染对象的字段。

`Decorated` 按子节点定尺寸，并录一条盖住边框盒的矩形。圆角记在 `Rect` 命令上，不改变命中。按钮外层内边距是 8。`Tree::pointer` 交出 `Press`、`Release`、`Click`，以及滚动锁定后的 `DragBegin`、`DragBy`、`DragEnd`。`set_viewport(x, y, w, h)` 把根放到 `(x, y)`，并把 `(w, h)` 交给 Taffy。命中和录制用的已经是绝对坐标。

`ScrollArea` 包一个 `Scroll` 和一列内容，缺省 `axis=vertical`、`accept=always`。横向分页传 `horizontal`。`VirtualList` 内部的 `Scroll` 固定 vertical。裸 `<scroll>` 仍是 `Both`。`Field` 的单行滚动是 horizontal，多行是 vertical。拖动、滚轮和夹紧在 `uicore` 的 `ScrollController` 上：拖动从上一笔的未夹紧位置继续减，不从夹紧后的 offset 重算；滚轮仍夹紧 offset。声明了单轴时，控制器丢掉跨轴增量，不把它当成给父级的溢出。`Both` 且该轴没有内容时，溢出仍交给父级。`physics.overscroll` 为 `rubber` 时，越界的位移交给 `Tree::set_scroll_offset` 记成 overscroll。松手后的弹簧在 `uicore` 里回到 0，相位回到 idle。`phase` 和 `overscroll` 只在相位变化和拖拽采样时写信号。`offset()` 现读树。`createScrollLink` 挂在 `onOffset` 上，映射函数留在语言侧。`RefreshControl` 把阈值交给同一个 controller：拖动中按 overscroll 越过阈值就武装，相位回到 idle 时才调用 `onRefresh`。皮只把这段距离画成一根占位高度。

`VirtualList` 的窗口、Fenwick 行高、滑块、吸顶和可见集 diff 在 `uicore` 的 `ListDriver`。应用 props 不变。语言侧在条目变化时推入 key、类型和是否吸顶，不在 tick 里逐行回调。行节点用 `noteRow` 登记一次，之后 tick 直接读 `Tree::rect`。可见集没变时不调用脚本。变了才经 `onDriver` 交出 `Detach` / `Adopt` / `Set` / `Place`，由 `bindListRenderer` 交给 `createSlot`。`restoreKey` 仍是语言侧的一张表，卸载时读 `offset()`。空白、错误和骨架仍是 JSX。独立的 `Recycler` 仍在语言侧对调用方已经切好的 `each` 做池化，不走这台 driver。驱动的类边界和 ops 在 `docs/driver.md`。

QuickJS 上这两个对象是 class：`core.listDriver` 和 `core.scrollController`。opaque 是序号和代，Rust 不保存 `JSValue`。事件是宿主上的一个 `onDriver` 回调，按 id 分发。终结器释放 driver。C 只在 `runtimes/quickjs/src/glue.c`。

```text
Scroll                 视口，PushClip
  Column               内容，高度可以超过视口
    …行
```

`Field` 也写在 `frameworks/solid-ui`。字符串、选择区和组合文本在 `uicore` 的 `TextEditor` 里。偏移是 UTF-16，移动和删除按字形簇。选择区记下 `base` 和 `extent`：方向键移动 `extent`，按着 Shift 则保持 `base`。软换行的同一个偏移有两个位置，`Affinity` 的 `Upstream` 画在上一行末尾，`Downstream` 画在下一行开头。硬换行是文本里的 `\n`，多行字段的回车插入它。

`TextStyle` 带 `soft_wrap` 和 `overflow`。`soft_wrap` 为真时，超过段落宽度的文字换到下一行，并尽量在空白后断开。为假时只认硬换行。`Clip` 丢掉超过 `max_lines` 的行。`Ellipsis` 在最后一行末尾换成省略号，缺省是 `…`。`Visible` 保留每一行，不换行时一行可以宽过约束。测试里的 `MonoShaper` 用同一套规则量尺寸和光标。宿主的排版器把这些字段交给 Impeller 的段落。

光标矩形来自这段落的 `caret_for`，不靠量前缀宽度。选择区是段落空间里的若干矩形。单行字段不换行，自己的 `Scroll` 把光标留在视口里。多行字段里上下键按行移动，并记住这一次横向位置。单行字段的上下键把焦点交给相邻控件。

`uicore` 记焦点，并把按键和文本送到当前焦点。聚焦时打开输入法。`KeyboardType` 决定平台键盘种类，`TextInputAction` 决定回车键的动作。边框换成焦点色。聚焦期间每帧 `requestFrame`，折叠的光标按时间闪烁。若包在外层 `Scroll` 里，聚焦时把视口滚到这块输入框。

`Switch` 是一个 `pointer` 加 `decorated`。传入 `checked` 时由调用方控制；省略时组件自己留一个布尔信号。`Dialog` 是铺满的 `stack`：遮罩 `pointer` 的点击调用 `onClose`，内容列叠在遮罩上面，所以点内容不会关掉。`Dialog` 不拦截焦点。

## 应用

`apps/gallery-solid/counter.tsx` 用 `@impasto/solid-ui` 的 Button、Field、Dialog、VirtualList，并用 `@impasto/solid-router` 的 `createRouter` 与 `RouterView`。`/list` 的 200 行打开 `VirtualList` 的 `recycle`。页底色由 gallery 把 `theme().color.background` 传进 `RouterView`。路由是 `/schedule`、`/nav/:depth`、`/list`。`boot` 把根列撑满视口，登记 `flush` 并 `render`。`build.mjs` 用 `@solidjs/babel-plugin`（`moduleName: "@impasto/solid-renderer"`、`generate: "universal"`）再交给 esbuild 打成 ESM `dist/counter.js`。门面打进 bundle，`@impasto/<pkg>/host` 按包枚举为 external，运行时由 QuickJS 物化。`util.js` 打进同一份产物。依赖是 npm 上的 `2.0.0-rc.17`。产物不包含 `third_party/solid`，也不进 git：`just gallery-dist` 在源码更新后重编，`just test`、`just host` 和 `just android` 先走它。quickjs 测试在运行时读产物文件，缺失时直接失败并提示 `just gallery-dist`。`cargo test` 不跑 node。

应用代码里不出现约束、命中测试、Metal 或窗口。Kotlin 界面 DSL 还没写。信号准备用已发布的 `com.lightningkite:reactive`。
