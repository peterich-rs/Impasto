# 三端平台层（macOS / Android / iOS）

macOS 与 Android 已落地：平台线程入队，渲染线程创建 `Engine` 和会话并 present。SDK 按平台放在 `third_party/impeller-sdk/<平台>/`。iOS 预构建矩阵仍然没有这一列（`ios`、`ios-arm64`、`ios-arm64-x86_64-sim` 返回 404），这一端不开始。

参照系是 Flutter embedder 和 React Native 的平台层：每个子系统先对上上游模块，再决定复刻接口形状、按发布版本链接，或只实现其中一部分。能链接的链接。对不上边界的只借鉴接口形状。硬边界不变：无 Dart VM，不引入 `embedder.h`，进程内产品代码不新增 C 头文件，QuickJS 与 `Engine` 都是 `!Send`，排版上下文不离开持有它的那条线程。

## 现状

macOS 上 `session::Session` 由 QuickJS 实现。`--js` 每帧调用它并提交 `DrawList`。`just host` 不创建会话：第一帧 `submit` 并 present 之后取消 vsync。

当前 pin `77f043bc63196b9979cc09c5b97bb6d9b00be34d` 作为 URL 里的框架提交，下面这些产物返回 200：

- `darwin-arm64`
- `android-arm64`
- `android-x64`

上游 Impeller interop 的预构建表有 macOS、Linux、Android、Windows。`ios-arm64`、`ios`、`ios-arm64-x86_64-sim` 对当前 pin 返回 404。不再扫候选提交找 iOS 包。iOS 要等上游把平台放进矩阵，或等单独的 engine 仓库提供产物。本仓库不克隆 Flutter engine，也不在树里编一份 Impeller。

stable `abaf9c523780a608bd46686fd5e53740a07077f8` 写在 URL 里时，android 与 iOS 都是 404。这个提交把 `engine.version` 设为 `deb287481e3ce9468f3434937ced4240a70539ca`。用 engine 哈希时，`android-arm64` 与 `darwin-arm64` 为 200，iOS 仍为 404。当前 pin 却是框架哈希就能 200。升级 pin 时对两种哈希各做一次 HEAD，把返回 200 的那种写进 `engine.pin`。

Vulkan 入口与 `example_vk.c` 一致，例子只提供调用顺序，不引入 GLFW：

1. `ImpellerContextCreateVulkanNew`（`proc_address_callback` 从 `libvulkan.so` `dlsym`）
2. `ImpellerContextGetVulkanInfo`，取出 `VkInstance`
3. 用该 instance 创建 `VkSurfaceKHR`（Android 上是 `vkCreateAndroidSurfaceKHR`）
4. `ImpellerVulkanSwapchainCreateNew`
5. 每帧 `ImpellerVulkanSwapchainAcquireNextSurfaceNew`，绘制，present，释放 surface

`AcquireNextSurfaceNew` 可能阻塞。acquire 失败且无法恢复时丢掉 swapchain，用新的 `VkSurfaceKHR` 重建。`ImpellerVulkanSwapchainCreateNew` 把链建成 1×1，Android 上的实现也不会读 `currentExtent` 来改这个尺寸，所以宿主记下窗口像素，`vkCreateSwapchainKHR` 回调把 `imageExtent` 改成这个像素尺寸。尺寸变化仍由这条链在下次 acquire 时重建，不丢 surface。发行包里 `enable_vulkan_validation` 为假。

## 决策

| # | 子系统 | 上游参照 | 决策 |
|---|---|---|---|
| 1 | 启动装配 | Flutter `Shell`、Android `FlutterActivityAndFragmentDelegate`；RN `ReactHost` | `hostshell` 编排启动。语言会话由宿主在渲染线程上打开后交进来 |
| 2 | 线程 | `ThreadHost`；RN 无 ui/raster 流水线 | 平台线程 + 渲染线程。测量与绘制仍共用一个 typography，不做 ui/raster 重叠 |
| 3 | 指针 | `pointer_data_dispatcher`、`PointerDataPacketConverter`；Android `MotionEventTracker` | 换算后入队，vsync 上排空再命中。有 hover、滚轮、按键和修饰键。没有 resampling，没有多点 |
| 4 | 按键 / 输入法 | `TextInputPlugin`、`FlutterTextInputPlugin` | `Session::input` 收 `InputEvent`。焦点在 `core`。编辑在组件。macOS 用 `NSTextInputClient`。Android 把 GameActivity 的全量缓冲差分成 `Text` / `Composing` |
| 5 | 帧调度 | `Animator::RequestFrame`、`VsyncWaiter`；RN `ReactChoreographer` 槽位 | 指针、metrics、表面事件，或会话申请的下一帧，才登记。列表与上次相同则不 `submit`，除非会话还要这一帧 |
| 6 | 表面 | `AndroidSurface`、`FlutterSurfaceController` | `Surface` 只负责 acquire 与 present。尺寸变化不丢 Vulkan 链；surface 销毁才丢 |
| 7 | 平台能力 | platform channel、`PlatformPlugin` | `Services`。第一批是字体、已解码图片、日志 |
| 8 | 资产 | `FlutterAssetManager` 密度变体 | 固定路径，启动时解码一次。无变体、无色彩管理 |
| 9 | 生命周期 | `AppLifecycleState` 五态 | `Pause` / `Resume`。inset 写入 `Metrics`，再交给 `set_viewport` |
| 10 | 平台视图 | `PlatformViewsController`、`TextureRegistry` | 无原生控件混排。图片是 `DrawList` 的 `Image` |
| 11 | 可访问性 | `AccessibilityBridge` | `frame` 里的语义遍历保持空位，不建桥 |
| 12 | 多窗口 | `FlutterEngineGroup` | 一进程一窗口一会话 |
| 13 | 入口 | `DartEntrypoint`；RN `AppRegistry.runApplication` | 宿主在渲染线程调用 `quickjs::open(script, shaper)`。`hostshell` 不依赖 `quickjs` |
| 14 | 文本缩放 / 亮度 / locale | `SettingsChannel` | `Metrics` 先不加这些字段 |

`hostshell` 不调用 `quickjs::open`。一个可执行文件只链一种会话；以后 Kotlin 宿主替换打开会话的闭包，这些 trait 不动。

## 分层

```text
apps/gallery-solid/dist/counter.js
        │
        ▼
runtimes/quickjs          宿主在渲染线程上打开，得到 Box<dyn Session>
        │
session::Session          metrics · pointer · input · frame · wants_frame · wants_ime · lifecycle
        ▲
hostshell                 命令队列 · 指针过滤 · 帧调度 · 相同列表不提交 · boot
        │
        ├── hosts/macos        AppKit、CAMetalLayer、CVDisplayLink
        ├── hosts/android      GameActivity、AChoreographer、Vulkan surface
        └── hosts/ios          UIKit、CAMetalLayer、CADisplayLink。预构建矩阵没有 iOS，这一端还没写
        │
        ▼
engine::Engine            submit 在 `hostshell` 里；draw_metal / draw_vulkan 在各端 Surface 里
core::Tree                会话持有。宿主与 `hostshell` 都不直接改树
```

依赖向下：`hosts/*` → `hostshell` → `session`、`core`、`protocol`、`engine`。`PointerEvent` 定义在 `core`，`hostshell` 会依赖整个 `core` crate。`hostshell` 不依赖 `quickjs`。`engine` 继续不依赖 `core`。

宿主实现三个 trait，并交给 `hostshell` 一个 `Send` 闭包：

| 能力 | 谁实现 | 在哪条线程调用 |
|---|---|---|
| `Services` | 各端宿主 | 渲染线程 |
| `Surface` | 各端宿主 | 渲染线程 |
| `Vsync` | 各端宿主 | `request_frame` / `cancel` 随时可调，立即返回 |
| 打开会话的闭包 | 各端宿主 | 渲染线程，此时 `Engine` 已经创建 |

```rust
pub trait Services: Send {
    /// 注册名到字体字节。`UIFWSans` 是注册名，不是文件里的 name id。
    fn font(&self, family: &str) -> Option<&[u8]>;
    /// 已解码的紧排 RGBA。借用活到 `Engine::submit` 返回。
    fn image<'a>(&'a self, source: &str) -> Option<engine::ImagePixels<'a>>;
    fn log(&self, level: LogLevel, message: &str);
}

pub trait Surface {
    /// acquire、绘制、present。acquire 失败且表面丢失时返回 `Status::NotReady`。
    fn present(&mut self, engine: &mut engine::Engine) -> Result<(), engine::Status>;
}

pub trait Vsync: Send + Sync {
    fn request_frame(&self);
    fn cancel(&self);
}
```

`Surface` 的公开接口不出现 `VulkanSwapchain` 或 `CAMetalLayer`，否则没装对应 SDK 的目标编不过测试。`hostshell` 调用 `Engine::submit`，再把 `&mut Engine` 交给 `present`。Metal 实现调用 `draw_metal`，Android 实现调用 `draw_vulkan`。

```rust
pub enum Launch {
    /// 一份现成的 DrawList。渲染线程上 submit 一次，之后只 present。不排空指针。
    Fixture(protocol::DrawList),
    /// 在渲染线程上调用。闭包只捕获脚本等 `Send` 数据，不捕获 `Engine`。
    Session(Box<dyn FnOnce(&EngineSlot) -> Result<Box<dyn session::Session>, SessionError> + Send>),
}

pub enum SessionError {
    NotLinked,
    Runtime,
    Script(String),
}
```

`EngineSlot` 只提供 `shaper()`。`Engine` 的所有权留在 `hostshell`。macOS 的闭包调用 `quickjs::open(&script, slot.shaper())`，并把 `quickjs::Error` 映射成 `SessionError`。`ImpellerShaper` 在 `hostshell`，各端用同一台 `Engine` 测量。`Rc` 在闭包里面创建，不经过命令队列。

## 1. 启动

平台线程准备 `Send` 的字节和窗口，然后返回事件循环。渲染线程创建 `!Send` 的对象。平台线程不在启动函数里等待表面，否则会占住 looper，表面回调进不来。

```text
平台线程                                      渲染线程
读脚本 / 字体 / 图片字节
创建窗口，主线程创建并挂上 CAMetalLayer
Shell::start(Boot) ──────────────────────►  创建 Engine，register_font(UIFWSans)
返回 looper                                  Fixture → submit 一次
                                             Session → 调用闭包（含 Shaper 与 quickjs::open）
表面就绪，投递 SurfaceReady ──────────────►  创建 Surface，下发 metrics，强制出首帧
```

字体缺失、`create_metal` / `create_vulkan` 失败、会话闭包失败，都在渲染线程记日志并把退出码交回主线程，进程退出。不留下只建了一半的 `Engine`。

退出码：

| 码 | 条件 |
|---|---|
| 0 | 正常退出 |
| 2 | 引擎不可用：`Status::NotLinked` 或 context 创建失败 |
| 3 | 解释器 `NotLinked` 或 `Runtime` |
| 4 | 启动期 `qjs_eval` 失败，`SessionError::Script`。运行期 `console` 和监听器错误只记日志，进程继续 |

`Launch::Fixture` 画出第一帧后取消 vsync 登记。内容是静态的。`UIFW_DUMP` 在这一帧读回像素，再让主线程结束进程。系统把窗口重新暴露出来时，不因此再登记一帧。

## 2. 线程

渲染线程用来接住会阻塞的取表面。`ImpellerVulkanSwapchainAcquireNextSurfaceNew` 可能等到下一张图像；macOS 上 `nextDrawable` 今天在主线程取，窗口事件会一起停。平台线程只做窗口、输入换算和 vsync 源。测量和绘制仍是同一条渲染线程上的同一个 typography，不各持一份排版上下文。渲染线程是 `std::thread` 上的同步循环，不进 async 运行时。

平台线程到渲染线程是一条命令队列。渲染线程到平台线程是一条应答队列，只放四件事：登记下一帧、取消登记、`SurfaceDropped`、退出。平台线程入队后立刻返回。锁只包住入队和出队。渲染线程把本帧要用的指针拷出来之后，在锁外跑会话和 `Engine`。平台线程只在 `SurfaceDestroyed` 里等待 `SurfaceDropped`；渲染线程处理销毁时不再等平台线程。

```rust
enum Command {
    Pointer(core::PointerEvent),
    Metrics(session::Metrics),
    Lifecycle(session::Lifecycle),
    SurfaceReady(SurfaceTarget),
    SurfaceChanged(SurfaceTarget),
    SurfaceDestroyed,
    Frame { time_ms: f64 },
    Shutdown,
}
```

相邻的 `Pointer(Move)` 合并成队尾那一个，避免渲染线程停住时 move 无限堆积。`Down`、`Up`、`Cancel` 不合并、不丢弃。metrics 不从指针中间抽走，避免重排。

`*mut c_void` 本身不是 `Send`。窗口指针放进 newtype，在一处 `unsafe impl Send`。约定：平台线程保住对象活到收到 `SurfaceDropped`；只有渲染线程解引用。`NSWindow`、`NSView`、`UIView` 以及 objc2 的 `MainThreadOnly` 类型不进这个队列。

| 对象 | 线程 |
|---|---|
| `NSWindow`、`NSView`、`UIView`、窗口事件 | 平台线程 |
| `CAMetalLayer` 的创建、挂到视图、`drawableSize` | 平台线程 |
| `ANativeWindow` 的获取与最终 `release` | 平台线程，`release` 必须在 `SurfaceDropped` 之后 |
| `Engine`、typography、会话、`Shaper`、`Surface::present` | 渲染线程。在这里创建、使用、销毁 |
| vsync 回调 | 平台自己的线程。回调只入队 `Frame` |

vsync 源不在渲染线程上跑帧：

- Android：`AChoreographer_postFrameCallback64` 挂在 `android_main` 的 `ALooper` 上。那不是 Java 的 main looper。
- iOS：`CADisplayLink` 挂在主线程 runloop。
- macOS：`CVDisplayLink` 的回调线程。主线程上不常开 `NSTimer`。

三端的回调都只入队 `Frame { time_ms }`。`time_ms` 是单调毫秒，原点由平台自定。Android 用 `frameTimeNanos / 1e6`，Apple 用 vsync 时间戳乘 1000。QuickJS 今天忽略这个参数。

`Vsync::request_frame` 只向应答队列投递「登记下一帧」，不等待登记完成。`cancel` 同样立即返回。渲染线程阻塞在命令队列上。`Pointer`、`Metrics`、`Lifecycle` 和表面尺寸变化只写入渲染线程的本地状态；调用 `session.pointer` 与 `frame` 的是随后的 `Frame`。当时若没有在途的帧，就 `request_frame` 一次。`SurfaceDestroyed` 与 `Shutdown` 收到即处理，不等下一帧，否则平台线程会停在销毁回调上等一个不会再来的 `Frame`。

`UIFW_DUMP` 在渲染线程读回刚 present 的纹理并写 PNG，然后向主线程投递退出。不从渲染线程调用 `NSApplication::terminate`。

## 3. 指针

`core::PointerEvent` 的相位只有 `Down`、`Move`、`Up`、`Cancel`，另有 `time_ms`，没有 pointer id。多指过滤放在各端入队之前：只保留 pointer index 0，丢掉 `ACTION_POINTER_DOWN` / `ACTION_POINTER_UP`，不展开历史采样。第二指不进入队列，因此不会覆盖 `Tree` 里记住的按下目标。

入队前各端把坐标换成场景点，原点在左上：

| 端 | 换算 |
|---|---|
| macOS | 窗口坐标转进内容视图，Y 翻转。AppKit 已是点，不除 `backingScaleFactor` |
| Android | 物理像素除以 density。density 来自 `AConfiguration`，读不到就失败，不猜 |
| iOS | `location(in:)` 已是点，原点在视图左上，不翻转、不除 scale |

指针先进入 `hostshell` 的队列，渲染线程再交给 `core` 做命中和手势。三端共用这一份队列：

- 出队顺序与入队顺序一致。
- 未按下时的 `Move` 保留，用来做 hover。相邻的 `Move` 合并成最后一笔。滚轮事件保持原顺序。
- `Cancel` 原样送到 `Session::pointer`。`Tree::pointer` 会清掉按下的区域。macOS 没有系统 cancel。

还没有轨迹重采样，也没有多点。Android 只取 pointer index 0。

## 4. 按键与输入法

文本焦点在 `core::Tree`。点击（抬起且没有锁成拖动）才移动焦点。`Session::input` 把 `Key`、`Text`、`Composing` 交给当前焦点；没有焦点就丢掉。组件保存字符串、光标和组合文本。`core` 不编辑字符串。

平台线程把 `Command::Input` 入队，和指针一样在下一帧排空，并登记这一帧。暂停时先留在队列里。`wants_ime` 在 `session.frame` 之后读取，和上次投递不同才 `Services::set_ime`。相同显示列表提前返回的那条路也要读。`take_ime_text` 有值时，平台线程 `set_text_input_state`（Android）或替换 macOS 的协议镜像。组合过程中不回写。

macOS：`keyDown:` 调用 `interpretKeyEvents:`。`insertText:` 是 `Text`，`setMarkedText:` / `unmarkText` 是 `Composing`，`doCommandBySelector:` 把回车、退格和方向键收成 `Key`，其余 selector 吞掉并打印。`selectedRange` 和 `markedRange` 读主线程上的镜像。`firstRectForCharacterRange:` 返回内容视图 bounds 换到屏幕坐标后的矩形。`set_ime(true)` 在主线程 `makeFirstResponder`。

Android：`TextEvent` 的下标是 UTF-16。宿主先换成字节下标，再拆成已提交串和组合串。变长用 `Text`，变短用相应次数的 `Backspace`，组合区间变化用 `Composing`。`KeyAction::Up` 不映射。软键盘可见时忽略 `KEYCODE_DEL`。`KEYCODE_BACK` 在键盘可见时投递 `ImeDismissed`，不删字符。聚焦和本地按键修改之后 `set_text_input_state`，光标在末尾。键盘高度来自 `WindowInsets.Type.ime()`（8），并进 `Metrics.inset_bottom`。`targetSdk` 35 不靠 `adjustResize` 缩小 Surface。inset 从非零变成 0 视为用户收起键盘，会话失焦。

还没有的能力：选区、复制粘贴、点按把光标放到任意偏移、修饰键和快捷键、组合文本下划线、键盘方向键移动焦点。窗口失焦见 §9，和文本焦点不同名。

## 5. 帧

`Frame` 在渲染线程上执行。暂停期间到达的指针仍入队，恢复时再排空。

```text
① 处理队列里的 Lifecycle / Metrics / Surface
   Pause 且仍有未结束的按下 → 先补一个 Cancel
② 排空指针队列，调用 session.pointer；再排空 Input，调用 session.input
   命中用上一帧几何。无焦点的输入丢掉。监听器只改状态
③ 已 Pause，或还没有 Surface → 不 submit。Pause 不再 request_frame
④ session.frame(time_ms) → DrawList
   会话内部先排空 QuickJS job，再 flush，再按树的脏标记 layout / record
⑤ 列表与上一次成功 submit 的列表相同 → 跳过 submit 和 present
   不同 → engine.submit，成功后记住这份列表，再 surface.present
⑥ 本帧不再 request_frame
```

下一帧只在这些命令到达、且当时没有在途帧时登记：指针、metrics 发生变化、`Resume`、`SurfaceReady`，以及启动后的第一帧。`Session` 不增加 `poll_dirty`。树的脏标记在 `layout` 成功时清掉，`frame` 返回之后 `hostshell` 看不到它。`hostshell` 也不能在调用 `frame` 之前猜树是干净的，否则 `flush` 不会跑。

`set_viewport` 会把树标脏。metrics 与上一份 `PartialEq` 相同则不再调用 `session.metrics`，否则静止画面每帧都重新布局。窗口第一次就绪时一定下发。`SurfaceDestroyed` 丢掉 `hostshell` 记住的那份列表，下一次表面就绪必须重新 `submit`。

`Session::frame` 返回拥有的 `DrawList`。`DrawList` 实现了 `PartialEq`，`hostshell` 用它判断要不要提交。渲染引擎里的显示列表留到下一次不同的 `submit`。列表与上次相同时仍会进一帧，但不会重建 Impeller 对象。

```rust
pub enum Lifecycle {
    Pause,
    Resume,
}

pub trait Session {
    fn metrics(&mut self, metrics: &Metrics);
    fn pointer(&mut self, event: core::PointerEvent);
    fn frame(&mut self, time_ms: f64) -> DrawList;
    fn wants_frame(&mut self) -> bool { false }
    fn lifecycle(&mut self, _event: Lifecycle) {}
    fn input(&mut self, _event: InputEvent) {}
    fn wants_ime(&self) -> bool { false }
    fn take_ime_text(&mut self) -> Option<String> { None }
    fn ime_dismissed(&mut self) {}
}
```

`Launch::Fixture` 没有会话。第一帧 present 成功后 `cancel`。布局失败时会话返回上一份成功的列表；空列表表示树上没有命令。

`record` 会跳过空文字。渲染引擎对空文字返回 `BadScene`。`hostshell` 不用再滤。

## 6. 表面

```text
无 Surface
  │ SurfaceReady
  ▼
有 Surface ── present
  │ SurfaceChanged：平台线程更新 drawableSize，`hostshell` 更新 metrics，链保留
  │ SurfaceDestroyed：渲染线程丢掉 Surface，应答 SurfaceDropped
  ▼
无 Surface（Engine、会话、typography 仍在）
```

平台线程只在 `SurfaceDestroyed` 里等待 `SurfaceDropped`，然后才 `ANativeWindow_release`。渲染线程处理这条命令时不反过来等平台线程。除此之外平台线程不等待渲染线程。

`present` 返回 `NotReady` 时同样丢掉 `Surface` 并应答，等下一次 `SurfaceReady`。重建或再次就绪之后，先下发 metrics，再出帧。

尺寸变化不调用 `ImpellerVulkanSwapchain` 的丢弃。只有表面本身失效才重建，顺序仍是 GetVulkanInfo 的 instance、`vkCreateAndroidSurfaceKHR`、`ImpellerVulkanSwapchainCreateNew`。

## 7. 平台能力

`Services` 替代 platform channel。三端各一份实现，由 `hostshell` 在渲染线程调用。

字体按注册名取字节，启动时交给 `register_font`。图片在 `Services` 实现里解码一次并留着，`image` 返回对这份缓存的借用。未知 `source` 返回 `None`，`submit` 按缺图处理。日志从渲染线程调用：macOS 写 stderr，Android 写 logcat，iOS 写 `os_log`。

以后若要剪贴板、触觉、系统音或打开 URL，逐个加方法。不加字符串消息通道。

## 8. 资产

| 键 | macOS | Android | iOS |
|---|---|---|---|
| 注册名 `UIFWSans` | `/System/Library/Fonts/Supplemental/Arial.ttf` | `assets/fonts/UIFWSans.ttf` | Bundle 内同名文件 |
| `asset:<name>` | 启动参数给出的资产目录下 `<name>.png`，ImageIO | `assets/images/<name>.png`，`AImageDecoder` | Bundle 内同名文件，ImageIO |

Android 与 iOS 的字体是可再分发的文件，注册名仍叫 `UIFWSans`。系统 Arial 不打进 APK 或 Bundle。因此 Android 首帧不和 macOS 的 `UIFW_DUMP` 逐像素对比：字形度量会不同。三端要同一份字体字节之后，文字位置才有可比性。

`AssetManager` 在 `android_main` 上取得。渲染线程第一次要字体或图片时读取并解码，结果留在 `Services` 里。

密度变体、降采样、色彩管理和图片缓存层仍停着，不在平台层先做。

## 9. 生命周期与 inset

`Pause` 与 `Resume` 两态。Android 对应 `onPause` / `onResume`，iOS 对应进后台与回前台。macOS 随窗口关闭退出，不发这两态。初始状态就是在跑，启动时不另发 `Resume`。

`Pause` 到达时，若按下还没 `Up` 或 `Cancel`，`hostshell` 先补一个 `Cancel` 再停帧。这样切走应用不会把 `Tree` 的按下状态留到下一次 `Down`。窗口失焦以后若要同样补 `Cancel`，另加事件。那个事件不是文本焦点。

路由恢复不做。

`Metrics` 的四边 inset 交给 `Tree::set_viewport(x, y, w, h)`。根从 `(inset_left, inset_top)` 排，Taffy 的可用空间是扣掉四边之后的宽高。命中和录制用绝对坐标，不再另做换算。macOS 的 inset 是 0。窗口缩放和换屏幕都投递同一份 `Metrics`。

## 10–12. 继续不做的部分

- 无原生控件混排，无外接纹理。以后若要视频帧，再对照 `TextureRegistry` 的接口形状。
- 不建可访问性桥。语义遍历的空位留在 `session.frame` 里。
- 一进程一窗口一会话。QuickJS 是单实例，不做多引擎共享。

## 三端差异

| | macOS | Android | iOS |
|---|---|---|---|
| 窗口 | `NSWindow` + `CAMetalLayer` | GameActivity + `ANativeWindow` | `UIView` + `CAMetalLayer` |
| 平台线程 | AppKit 主线程 | `android_main` 的 looper | `UIApplicationMain` |
| vsync | `CVDisplayLink`，回调只入队 | `AChoreographer_postFrameCallback64` | 主线程 `CADisplayLink` |
| 光栅 | Metal | Vulkan | Metal，调用同一 `draw_metal` |
| 指针 | `mouseDown` / `Dragged` / `Up` | `MotionEvent`，含 `ACTION_CANCEL` | `touches*`，含 `touchesCancelled` |
| 坐标 | 内容视图 + Y 翻转 | 像素 ÷ density | 已是点 |
| metrics | 默认 360×640，可缩放，inset 0 | 窗口尺寸 ÷ density。inset 变成根偏移并缩小可用空间 | bounds 与 scale。inset 同样只采集 |
| 生命周期 | 关闭即退出 | `onPause` / `onResume` | 前后台 |
| 像素 | `UIFW_DUMP` 读纹理 | 有渲染目标读回之后再比像素。`adb screencap` 只作人工查看 | 与 macOS 相同的纹理读回 |

## 实现约束

- crate 均 `publish = false`。`Shell` 含有 `Engine` 和会话，因此不是 `Send`。跨线程的只有命令队列、`Vsync`，以及有意标记为 `Send` 的窗口指针 newtype。
- `unsafe` 集中在 Vulkan 句柄、Android 窗口与 NDK 事件、objc2，以及那个 `Send` 指针。每处按 `engine/src/player.rs` 写 SAFETY 注释。workspace 的 `unsafe_code = deny` 仍有效。
- `hostshell` 的队列、过滤、相同列表跳过提交，以及表面应答，用无窗口单测。各端坐标换算沿用 `hosts/macos/src/pointer.rs` 这种单测。端上验收是出帧、点击改字，以及有读回时的像素。
- 场景格式保持版本 1。不做 GL 后备、resampling、多点、选区、复制粘贴、点按定位光标、组合下划线、平台视图、可访问性桥和多窗口。显示列表的命令级 diff 不做；相同列表直接跳过 `submit`。
- 新依赖在引入时钉版本。`android-activity` 与 `ndk` 已钉住。它们不带 UI 框架或 Dart VM。要裁 Impeller 后端时放到以后的 engine 仓库，不在本仓库编 engine。
