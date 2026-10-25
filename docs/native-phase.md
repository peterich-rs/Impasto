# 原生层

场景格式、渲染树、渲染引擎、会话、`hostshell`、macOS 宿主和 Android 宿主是 Rust。字节格式是版本 1，开头是 `UIFW`。Solid 编译、Kotlin 的 cinterop 和图片管线不在这里。iOS 没有 Impeller 预构建产物，宿主还没写。

`cargo test` 核对 208 字节和布局结果。不加载脚本时，macOS 宿主把 `counter.bin` 画成第一帧。`--js` 走 QuickJS 会话。Kotlin 和 TypeScript 用自己的编码器对这套字节，不调用 Rust。这几层没有对外的 C 头文件。`core` 的 C ABI 留到 Kotlin cinterop 真要链接的时候再写。QuickJS 会话直接调用 `Tree`，胶水 C 只在 `runtimes/quickjs/src/glue.c`。

没有 `third_party/quickjs` 时会话 crate 返回 `NotLinked`。根上没有 CMake。

## 交付

| crate 或目录 | 内容 |
|---|---|
| `protocol/` | 版本 1 的编码、解码、`counter.bin`，以及把它收成 `DrawList` |
| `core/` | Taffy 布局、等宽排版、绘制录制、命中、click、drag 和焦点 |
| `engine/` | 读 `DrawList`，调用 `impeller.h` |
| `session/` | `metrics`、`pointer`、`input`、`frame`、`wants_frame`、`take_ime`、`lifecycle`、`settings` |
| `hosts/hostshell/` | 命令队列、指针过滤、相同列表跳过提交、表面应答 |
| `hosts/macos/` | 窗口、`CVDisplayLink`、指针入队、`UIFW_DUMP`。`--js` 时每帧问会话要列表 |
| `hosts/android/` | `HostActivity`、`HostView`、`Choreographer`、Vulkan、logcat。库名 `uifw` |
| `runtimes/quickjs/` | quickjs-ng 会话。包名 `quickjs` |

`just check` 先 `cargo test --workspace`，再跑 Kotlin 和 Solid 的字节对比。对照文件是 `protocol/tests/fixtures/counter.bin`，环境变量是 `UIFW_COUNTER_BYTES`。

## 仓库

根上有 `Cargo.toml` 和 `rust-toolchain.toml`。toolchain 钉 stable。workspace 成员是 `protocol`、`core`、`engine`、`session`、`hosts/hostshell`、`hosts/macos`、`hosts/android`、`runtimes/quickjs`。包名等于目录名，QuickJS 那个包名是 `quickjs`，Android 动态库名是 `uifw`。

```
protocol/          版本 1 字节，以及把它变成 DrawList 的映射
core/              渲染树。依赖 taffy 和 protocol
engine/            渲染引擎。依赖 protocol 和 Impeller 头文件
session/           会话 trait。依赖 core 和 protocol
hosts/hostshell/   命令队列、帧调度、渲染线程。依赖 core、engine、protocol、session
hosts/macos/       二进制。依赖 hostshell、core、engine、protocol、session、quickjs
hosts/android/     cdylib `uifw`。依赖 hostshell，并在 Android 上打开 QuickJS 会话
runtimes/quickjs/  QuickJS 会话。依赖 core、protocol、session
```

依赖只向下：`hosts` → `session` → `core` → `protocol`，`hosts` → `engine` → `protocol`，`hosts` → `quickjs` → `session`。`engine` 不依赖 `core`。`core` 不依赖 `engine`，也不包含 Impeller 或 AppKit 的头文件。宿主实现 `Shaper` 时调用 `engine`。

Taffy 用 crates.io 上引入时的版本，写进 `Cargo.lock`。不使用它未完成的 C 绑定。

## 一帧

宿主和核心直接用 Rust 调用，不经过 C。帧顺序、依赖方向和会话里的冲刷在 `docs/architecture.md`。三端线程和表面在 `docs/platform-hosts.md`。

`just host` 不经过渲染树。它把 `counter.bin` 收成 `DrawList` 并提交一次，不做 Taffy 布局。`just host-js` 走会话那一帧。

## protocol

模块分成 `encode`、`decode`、`fixture`、`draw_list`。

版本 1 的布局以这一节和 `protocol/tests/fixtures/counter.bin` 为准。

- 开头四字节是 `UIFW`，接着小端 `uint32` 版本 `1` 和 `uint32` 节点数
- 每个节点 36 字节再加字符串：id、parent、kind、3 字节 0、四个小端 `f32`、rgba、`uint32` 长度、UTF-8
- kind 是 `LAYER=1`、`RECT=2`、`TEXT=3`、`IMAGE=4`
- 短缓冲区返回失败且不写出。解码要求魔数、版本、kind 和字节刚好用完。解码结果拥有 `String`，并拒绝非法 UTF-8

计数器场景是这棵手写的绝对坐标树，编码长度 208。文件是 `protocol/tests/fixtures/counter.bin`。Rust 测试对这个文件逐字节比较，不用编码器再算一遍「应该是 208」，也不重新生成它。解码测试覆盖：截断、错误魔数、错误版本、非法 kind、尾部多余字节、文件里 `"Count"` 的坐标和 `"asset:logo"` 的宽度。

`draw_list` 把解码后的版本 1 节点收成绘制命令：

- 跳过 `LAYER`
- 先铺 rgba `(17, 17, 17, 255)` 的全屏矩形
- `RECT` 和 `IMAGE` 用节点自己的盒子
- `TEXT` 记下字符串、颜色、位置、字号和段落宽。版本 1 字节没有样式，映射填字号 28、段落宽 280。这两个常数在 `protocol`

```rust
pub enum Command {
    Fill { color: [u8; 4] },
    Rect { x: f32, y: f32, w: f32, h: f32, corner_radius: f32, color: [u8; 4] },
    Text { x: f32, y: f32, max_width: f32, font_size: f32, color: [u8; 4], utf8: String },
    Image { x: f32, y: f32, w: f32, h: f32, source: String },
    PushClip { x: f32, y: f32, w: f32, h: f32, corner_radius: f32 },
    PushOpacity { x: f32, y: f32, w: f32, h: f32, alpha: u8 },
    PushTransform { translate_x: f32, translate_y: f32, rotate_rad: f32, scale: f32 },
    Pop,
}

pub struct DrawList {
    pub commands: Vec<Command>,
}
```

`core::Tree::record` 交出带节点自己的字号和测量约束的同一份列表。渲染引擎只认 `DrawList`，不自己解析场景字节，也没有默认字号。

## core

渲染树是 Rust 结构，不导出 C。节点 id 是槽位加代数，原始值是 `u64`。根是显式字段。`remove` 释放子树的 Taffy 节点，场景节点留着，可以再插回去。空文字不录。树脏了才重新 `layout`。`layout` 调用 `TaffyTree::compute_layout_with_measure`，把算出来的位置和尺寸写回节点。坐标相对父节点。录制时加上祖先原点，`DrawList` 里只有绝对坐标。

| 节点 | 样式 | 绘制 |
|---|---|---|
| `Row`、`Column` | `Display::Flex`，方向和 gap | 不绘制 |
| `Grid` | `Display::Grid`，行轨和列轨 | 不绘制 |
| `Padding`、`Align`、`Sized` | padding、对齐、宽高 | 不绘制 |
| `Stack` | 一格的 grid。子节点放在第 1 行第 1 列，叠在同一原点 | 子节点从底到顶 |
| `Clip` | 与 `Stack` 相同 | `PushClip` … 子节点 … `Pop`。盒子为 0 也成对 |
| `Opacity` | 与 `Stack` 相同 | `PushOpacity` … 子节点 … `Pop`。子树离屏后按 alpha 回贴 |
| `Scroll` | 一格 `max_content` grid。边框盒是视口 | `PushClip` … 子树减去 offset … `Pop`。offset 不进 Taffy |
| `Transform` | 与 `Stack` 相同 | `PushTransform` … 子节点 … `Pop`。命中使用逆变换 |
| `Text` | measure 回调返回段落尺寸 | `Text` 命令，带字号、行宽约束和颜色 |
| `Image` | measure 回调返回固有尺寸 | `Image` 命令 |
| `Decorated` | 一列子节点，间距 0 | 边框盒一条矩形，圆角记在命令上。命中交给子节点 |
| `PointerRegion` | 尺寸等于子节点 | 不绘制 |

文字测量是 trait。测试实现按 Unicode 标量计宽：一个标量的前进宽度是 `font_size * 0.5`，高度是 `font_size`。放不下就换行。宿主的实现调用 `Engine::measure_paragraph`，测量和绘制共用同一次排版，颜色在测量时写进前景。`core` 不链接 Impeller。

命中测试用布局之后的盒子，从最后一个子节点往前。宽或高为 0 的节点跳过。`opaque` 命中盒子就收下，`defer` 只看子节点。点必须落在每个祖先盒子内。圆角不改变命中。`PointerRegion` 把自己放进命中结果。子节点的 `set_grow` 写 `flex_grow`，负值拒绝。flex 父节点有确定主轴尺寸时，正的 grow 在下一次 `layout` 分到剩余空间。

`PointerEvent` 是 `phase`、`x`、`y` 和 `time_ms`。`time_ms` 是宿主事件时间，毫秒，用来算拖拽速度。相邻 `Move` 在队列里合并时保留队尾那一笔的时间。点击记在 `PointerRegion` 上。按下返回 `Press`。位移不超过 8 点时，移出或 `Cancel` 返回 `Release`，抬起仍在区域内时先 `Release` 再 `Click`。位移超过 8 点且按下点有 `Scroll` 祖先时，先 `Release`，再 `DragBegin`、`DragBy`，抬起是 `DragEnd(vx, vy)`，单位是场景点/毫秒。没有 `Scroll` 时超过 8 点仍走按下语义。拖拽中的 `Cancel` 补 `DragEnd(0, 0)`。树上不存闭包。`set_scroll_offset` 只标绘制，不改 Taffy，也不让下一次 `layout` 进入 Taffy；录制把子树坐标减去 offset，命中把查询点加上 offset。测试用合成的 down、move、up、cancel，不打开窗口。监听器里不调用 `layout`。`set_text`、字号、间距和内边距标布局脏，几何留到下一次 `layout`。颜色、圆角、透明度和变换只标绘制。插入节点保留当前按下和拖拽。摘掉子树时只取消目标在这棵子树里的按下和拖拽。`set_viewport(x, y, w, h)` 把根放到 `(x, y)`，可用空间是 `(w, h)`。

无窗口测试至少锁住这些结果：

- `Column` 里两个高度为 20 的子节点，第二个的 y 是 `20 + gap`
- `Padding` 四边为 8 时，子节点原点是 `(8, 8)`，父尺寸是子尺寸加 16
- `Grid` 两列，两个固定尺寸的子节点分列左右
- 等宽排版下，字号 16 的 `"Hi"` 宽 16、高 16
- 在按钮范围内 down 再 up 会触发一次 click。移出后再 up 不触发
- `Clip` 包住 `Image` 时，命令序是 `PushClip`、`Image`、`Pop`。裁剪盒外的点击不落到子节点
- 位移不超过 8 点仍会 `Click`。超过 8 点且在 `Scroll` 里会 `Release`、`DragBegin`、`DragBy`，抬起的 `DragEnd` 带上两次 `Move` 的速度
- `Scroll` 的内容高于视口时，offset 平移子命令，视口外的点不命中，未滚动节点的布局坐标不变
- `Clip` 包住 `Transform` 时命令成对。变换后的命中仍落在布局盒子上
- 宽 100 的 flex 容器里，`set_grow(1)` 在下一次 `layout` 后把子节点拉过固有宽度。负的 grow 被拒绝，并且不标脏

## engine

渲染引擎用 `bindgen` 包含当前目标平台目录下的 `impeller.h`，例如 `third_party/impeller-sdk/darwin-arm64/include/impeller.h`。创建 Metal context 时传入该头文件里的 `IMPELLER_VERSION`。没有 SDK 时 `engine` 的图形测试跳过，不另放一个 C stub。

`submit` 接收 `DrawList`、`point_scale` 和图片回调。回调里的 source 是 `&str`。RGBA 只要活到 `submit` 返回。`draw_metal` 接收系统默认设备上的 `CAMetalLayer` drawable。Android 目标另外提供 `create_vulkan` 和 `draw_vulkan`。成功是 `Result::Ok`。失败是 `Status::NotLinked`、`BadScene`、`NotReady`。

第一条非 `Fill` 命令之前做一次缩放。`PushClip`、`PushOpacity` 和 `PushTransform` 也走这次缩放，因此 `Save` 和 `Restore` 之间的矩阵都含 `point_scale`。半径不大于 0 时走直角接口，否则钳到短边的一半再画圆角或圆角裁剪。透明度用单独的 layer paint，白底加上命令里的 alpha，`Restore` 之后释放。`PushTransform` 在 `Save` 之后是 `Translate`、`Rotate`、`Scale`，`Pop` 复用 `Restore`。多一个 `Pop`，或结束时栈还没排空，都是 `BadScene`，并且不创建显示列表。失败时先把残留的 save 恢复掉，再释放 builder。没有 SDK 时这些测试早退，不另放 C stub。

绘制时坐标乘 `point_scale`，文字用已注册字体和命令里的字号、字重、对齐、方向，字号不再乘缩放。缺省样式不额外调用最大行数、省略号和行高接口。图片必须是紧排 RGBA8888。版本 1 映射带来的文字已经带上字号 28、段落宽 280 和缺省样式。纹理按 `source` 跨帧保留，段落按文字、字号、宽度、颜色和样式跨帧保留。`measure_paragraph` 和绘制共用这张表。成功提交结束时只释放当前显示列表没有引用的条目。`register_font` 使段落在下一次提交释放显示列表之后失效。上屏失败是 `PresentFailed`。还没有表面或显示列表是 `NotReady`。这两种都不丢掉 `Surface`。`submit` 失败时已经在屏幕上的显示列表还在。

## 宿主

macOS 宿主用 `objc2`、`objc2-app-kit` 和 `objc2-metal` 调用 AppKit。窗口标题 `uifw counter`，内容默认 360×640，可关闭、可最小化、可缩放。缩放取窗口所在屏幕的 `backingScaleFactor`，取不到时用 2。改尺寸或换屏幕时按内容视图重算 `drawableSize`，并投递 `Metrics`。`framebufferOnly = NO`。

环境变量仍叫 `UIFW_DUMP`。设置之后画出第一帧，写成 PNG，然后进程退出。字体仍读 `/System/Library/Fonts/Supplemental/Arial.ttf`。图片按 `asset:<name>` 到资产目录里找 `<name>.png`，目录由启动参数给出，默认是 `hosts/macos/assets`。Android 用同一约定读 `assets/images/<name>.png`。

指针从 `NSEvent` 换成场景点放进队列。先把窗口坐标换进内容视图，再把原点改到左上。`timestamp` 乘 1000 写进 `time_ms`。AppKit 给出的已经是点，不除 `backingScaleFactor`。Android 的 `event_time` 是纳秒，除以 1_000_000 得到毫秒。队列在渲染线程排空。`just host` 提交一次绘制列表后只 present 第一帧。`--js` 由会话出队。没有新的指针或 metrics，并且会话没有申请下一帧时，不再登记 `CVDisplayLink`。列表与上次提交相同时不调用 `submit`。会话申请了下一帧时，列表相同也再登记恰好一帧。计数器的加减写在应用里。

Android 宿主是 `hosts/android` 的 cdylib，库名 `uifw`。Java 主线程入队。`present` 里 acquire、`draw_vulkan`、present。`ANativeWindow` 要等 `SurfaceDropped` 之后才释放。字体是 `assets/fonts/UIFWSans.ttf`（Roboto Regular，注册名仍是 `UIFWSans`）。图片是 `assets/images/logo.png`，用 `AImageDecoder` 解成 RGBA。`just android` 先编 cdylib，再打 APK。

文本输入：`Command::Input` 和 `ImeDismissed` 与指针一起入队。会话用 `take_ime` 交出 `ImeUpdate`。macOS 在 `Show` 之后把内容视图设成 first responder，`keyDown:` 只调用 `interpretKeyEvents:`，`NSTextInputClient` 读主线程上的那份 `ImeState`。Android 的 `TextInput` 实现 `InputConnection`。`commitText`、`setComposingText`、`finishComposingText` 和 `deleteSurroundingText` 收成 `Text`、`Composing`，以及用 `SelectAll` 加 `Text` 换上的新串。`Update` 写回 `Editable` 的选区和组合区间，单位是 UTF-16，并用插入符矩形更新 `CursorAnchorInfo`。键盘避让把 IME inset 放进 `view_insets`。系统返回键在键盘可见时只失焦。

## 验收

1. `protocol` 的测试与 `counter.bin` 逐字节相同，解码拒绝损坏输入。
2. `core` 的布局和 click 测试通过，且不链 Impeller。
3. `UIFW_DUMP` 配合 `just host` 画出计数器场景：深色底、白色 `Count`、绿色 `0`、右上角绿色斜线标志，这些像素在场景坐标里。窗口默认是 360×640，画面比场景里的 320×180 大，多出来的区域是清空色。`just host-js` 的第一帧还有圆角按钮和 `asset:logo`。按钮角上是窗口底色，内部是按钮填充色。
4. `just check` 仍跑通 Kotlin 和 Solid 的字节对比。QuickJS 测试读的是提交进仓库的 `dist/counter.js`，改宿主树之后要先 `just solid-app` 重编再跑 `cargo test`。真机验收：首帧避开状态栏，点减号得到 `Count: -1`，按下变色、抬起恢复，旋转或分屏后仍避让，Home 再回来不崩。没有指针、也没有 `requestFrame` 时不再登记帧。`console` 和运行期脚本错误出现在宿主日志里，进程继续。
5. `protocol`、`core`、`engine`、`hosts/macos` 没有 C 或 Objective-C 源码，根上也没有原生 CMake。QuickJS 的 `glue.c` 由 `cc` 编进 `quickjs` crate。没有解释器源码时这一步不编译 C。Android 宿主的 NDK 调用留在 `hosts/android`。
