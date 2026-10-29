# 三端平台层（macOS / Android / iOS）

macOS 与 Android 已落地：平台线程入队，渲染线程创建 `Engine` 和会话并 present。SDK 按平台放在 `third_party/impeller-sdk/<平台>/`。iOS 预构建矩阵仍然没有这一列（`ios`、`ios-arm64`、`ios-arm64-x86_64-sim` 返回 404），所以还没有 `hosts/ios`。`Session` 和渲染线程约束按第三端也能实现来写，见 `docs/refactor-plan.md` 的暂缓一节。

参照系是 Flutter embedder 和 React Native 的平台层：每个子系统先对上上游模块，再决定复刻接口形状、按发布版本链接，或只实现其中一部分。能链接的链接。对不上边界的只借鉴接口形状。硬边界：无 Dart VM，不引入 `embedder.h`。`protocol`、`core`、`engine`、`hosts/macos` 不手写 C。QuickJS 的胶水 C 只在 `runtimes/quickjs`。变更缓冲的生成头由 `runtimes/quickjs/abi.yml` 生成，不把现有 setter 逐个写成头文件。QuickJS 与 `Engine` 都是 `!Send`，排版上下文不离开持有它的那条线程。

## 现状

macOS 上 `session::Session` 由 QuickJS 实现。`just host` 加载 `counter.js`，每帧调用会话并 `submit_layers`。

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
| 3 | 指针 | `pointer_data_dispatcher`、`PointerDataPacketConverter`；Android `MotionEventTracker` | 换算后入队，vsync 上排空再命中。有 hover、滚轮、按键和修饰键。每根手指一条 `PointerId`。没有 resampling |
| 4 | 按键 / 输入法 | `TextInputPlugin`、`InputConnectionAdaptor` | `Session::input` 收 `InputEvent`。焦点和 `TextEditor` 在 `core`。macOS 用 `NSTextInputClient`。Android 的 `TextInput` 用 `InputConnection`，把 `commitText`、`setComposingText` 和 `deleteSurroundingText` 收成 `Text` / `Composing` |
| 5 | 帧调度 | `Animator::RequestFrame`、`VsyncWaiter`；RN `ReactChoreographer` 槽位 | 指针、metrics、表面事件，或会话申请的下一帧，才登记。列表与上次相同则不 `submit`，除非会话还要这一帧 |
| 6 | 表面 | `AndroidSurface`、`FlutterSurfaceController` | `Surface` 只负责 acquire 与 present。尺寸变化不丢 Vulkan 链；surface 销毁才丢 |
| 7 | 平台能力 | platform channel、`PlatformPlugin` | `Services`。第一批是字体、已解码图片、日志 |
| 8 | 资产 | `FlutterAssetManager` 密度变体 | 密度变体按 scale 桶选择；色彩管理和降采样保持今天的设备 RGB、预乘 |
| 9 | 生命周期 | `AppLifecycleState` 五态 | `session::Lifecycle`：`Detached`、`Resumed`、`Inactive`、`Hidden`、`Paused`。`Hidden` 和 `Paused` 冻结指针队列。inset 写入 `Metrics`，再交给 `set_viewport` |
| 10 | 平台视图 | `PlatformViewsController`、`TextureRegistry` | 无原生控件混排。图片是 `DrawList` 的 `Image` |
| 11 | 可访问性 | `AccessibilityBridge` | `Services::semantics` 把语义树交给平台。Generic 不建元素，子节点改挂到最近的非 Generic 祖先。macOS 的 label 进 `accessibilityLabel`，description 进 `accessibilityHelp`。Android 用并行数组交给 `AccessBridge`。Switch 用 CheckBox。有 ACTIVATE 才可点，有 SET_TEXT 才投递文本 |
| 12 | 多窗口 | `FlutterEngineGroup` | 一进程一窗口一会话 |
| 13 | 入口 | `DartEntrypoint`；RN `AppRegistry.runApplication` | 宿主在渲染线程调用 `quickjs::open(script, shaper)`。`hostshell` 不依赖 `quickjs` |
| 14 | 文本缩放 / 亮度 / locale | `SettingsChannel` | `Session::settings` 收 locale、text_scale、亮度、reduce_motion、bold_text、high_contrast。这些不进 `Metrics`。macOS 的 text_scale 是 preferredFont 字号除以 13，夹在 0.5 到 3；非有限或非正数用 1。bold_text 固定 false |

`hostshell` 不调用 `quickjs::open`。一个可执行文件只链一种会话；以后 Kotlin 宿主替换打开会话的闭包，这些 trait 不动。

## 分层

```text
apps/gallery-solid/dist/counter.js
        │
        ▼
runtimes/quickjs          宿主在渲染线程上打开，得到 Box<dyn Session>
        │
session::Session          metrics · pointer · input · frame · wants_frame · take_ime · lifecycle · settings
        ▲
hostshell                 命令队列 · 指针过滤 · 帧调度 · 相同列表不提交 · boot
        │
        ├── hosts/macos        AppKit、CAMetalLayer、CVDisplayLink
        ├── hosts/android      HostActivity、HostView、Choreographer、Vulkan surface
        └── hosts/ios          还没写。预构建有 iOS 列之后接 UIKit、CAMetalLayer、CADisplayLink
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
    /// 已解码的紧排 RGBA。借用活到 `Engine::submit_layers` 返回。
    fn image<'a>(&'a self, source: &str) -> Option<engine::ImagePixels<'a>>;
    fn log(&self, level: LogLevel, message: &str);
}

pub trait Surface {
    /// acquire、绘制、present。acquire 失败且表面丢失时返回 `Status::NotReady`。
    fn present(&mut self, engine: &mut engine::Engine) -> Result<(), engine::Status>;
}

pub trait Vsync: Send + Sync {
    fn bind(&self, sink: Arc<dyn Fn(FrameToken, FrameTime) + Send + Sync>);
    fn request_frame(&self, token: FrameToken);
    fn cancel(&self);
}
```

`Surface` 的公开接口不出现 `VulkanSwapchain` 或 `CAMetalLayer`，否则没装对应 SDK 的目标编不过测试。`hostshell` 调用 `Engine::submit_layers`，再把 `&mut Engine` 交给 `present`。Metal 实现调用 `draw_metal`，Android 实现调用 `draw_vulkan`。

```rust
pub enum Launch {
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
读脚本 / 字体 / 图片字节 / `--url` 或 Intent
创建窗口，主线程创建并挂上 CAMetalLayer
把已知启动 URL 写入 Shared launch_url
Shell::start(Boot) ──────────────────────►  创建 Engine，register_font(UIFWSans)
返回 looper                                  调用闭包（含 Shaper 与 quickjs::open）
                                             Linking.getInitialURL 读 Shared
表面就绪，投递 SurfaceReady ──────────────►  创建 Surface，下发 metrics，强制出首帧
运行中的 openURLs 投递 LaunchUrl ─────────►  Session::set_launch_url（emit url）
```

启动 URL 在 `hostshell::start` 之前写入 Shared `launch_url`，脚本求值时 `Linking.getInitialURL()` 就能读到。macOS 来自 argv `--url`，Android 来自显式 Intent 的 `getData()`。没有 URL 时槽保持 `None`，不写入空字符串。进程已在跑时，macOS 的 `application:openURLs:` 投递 `LaunchUrl`。冷启动时若 Apple Event 与 `--url` 相同，只在 `applicationDidFinishLaunching` 之前跳过那一次；之后同一条 URL 再打开也会投递。平台线程不调用 `Session`。当前宿主没有注册 URL scheme（macOS 无 `CFBundleURLTypes`，Android 无 `ACTION_VIEW`），系统不会把隐式打开交给本进程。`--url` 和显式 Intent data 仍然有效。

字体缺失、`create_metal` / `create_vulkan` 失败、会话闭包失败，都在渲染线程记日志并把退出码交回平台线程。不留下只建了一半的 `Engine`。macOS 随进程退出。Android 在 `onDestroy` 里等渲染线程结束再返回。`std::process::exit` 跳过这一步，`onDestroy` 仍停在等待上。

退出码：

| 码 | 条件 |
|---|---|
| 0 | 正常退出 |
| 2 | 引擎不可用：`Status::NotLinked` 或 context 创建失败 |
| 3 | 解释器 `NotLinked` 或 `Runtime` |
| 4 | 启动期 `qjs_eval` 失败，`SessionError::Script`。运行期 `console` 和监听器错误只记日志，进程继续 |

`UIFW_DUMP` 在一帧画出后读回像素，再让主线程结束进程。它用来看画面。窗口重新变为可见时，macOS 投递 `Invalidate`，置 `needs_present`。列表和缩放都没变则 `PresentCached`。

## 2. 线程

渲染线程用来接住会阻塞的取表面。`draw_vulkan` 里的 `ImpellerVulkanSwapchainAcquireNextSurfaceNew` 可能等到下一张图像。macOS 的 `nextDrawable` 在渲染线程的 `MetalSurface::present` 里取。平台线程只做窗口、输入换算和 vsync 源。测量和绘制仍是同一条渲染线程上的同一个 typography，不各持一份排版上下文。渲染线程是 `std::thread` 上的同步循环，不进 async 运行时。

平台线程到渲染线程是 `hosts/hostshell/src/command.rs` 里的 `Command` 队列。平台线程入队后立刻返回。渲染线程把本帧要用的指针拷出来之后，在锁外跑会话和 `Engine`。`SurfaceDestroyed` 由渲染线程立刻处理，先丢掉尚未打开的 `SurfaceTarget`，然后 `Acks::signal`。Android 的 `destroy_surface` 和 `open_surface` 把 `NativeWindow` 交给 `uifw-surface-drop`。该线程在 `DropWait` 上等待这条应答，然后 `ANativeWindow_release`。平台线程在 `spawn` 之后返回。渲染线程处理销毁时不再等平台线程。入队顺序仍是 `SurfaceDestroyed` 先于下一次 `SurfaceReady`。

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

- Android：`Choreographer` 挂在 Java 主线程。API 33 及以上用 `postVsyncCallback` 的帧开始时刻和预计上屏时刻；更低版本用 `frameTimeNanos` 加上刷新间隔。
- iOS：`CADisplayLink` 挂在主线程 runloop。
- macOS：`CVDisplayLink` 的回调线程。主线程上不常开 `NSTimer`。

三端的回调都只入队 `Frame { token, time }`。`FrameTime` 有 `vsync_ms`、`target_ms` 和 `interval_ms`，都是单调毫秒，原点由平台自定。动画采样 `target_ms`。macOS 的 `inNow` 是 `vsync_ms`，`inOutputTime` 是 `target_ms`。Android API 33 及以上用 `Choreographer.postVsyncCallback` 的帧开始时刻和预计上屏时刻；更低版本用 `frameTimeNanos` 加上刷新间隔。`Session::frame` 把 `target_ms` 和 `interval_ms` 交给 `onFrame`。

`Vsync::request_frame` 由渲染线程调用。macOS 在该线程启动 `CVDisplayLink`，Android 在该线程经 JNI 调 `scheduleFrame`。`cancel` 同样由渲染线程在 `end_frame` 里执行。UI 线程不调用 `FrameClock::cancel`。渲染线程阻塞在命令队列上。`Pointer`、`Metrics`、`Lifecycle` 和表面尺寸变化只写入渲染线程的本地状态；调用 `session.pointer` 与 `frame` 的是随后的 `Frame`。当时若没有在途的帧，就 `request_frame` 一次。`SurfaceDestroyed` 与 `Shutdown` 收到即处理，不等下一帧。

`UIFW_DUMP` 在渲染线程读回刚 present 的纹理并写 PNG，然后向主线程投递退出。不从渲染线程调用 `NSApplication::terminate`。

## 3. 指针

`core::PointerEvent` 带 `pointer`、`kind`、`button` 和 `buttons`。相位是 `Down`、`Move`、`Up`、`Cancel`、`Wheel`、`Zoom`。`Zoom` 的 `zoom` 是这次事件的相对缩放，其它相位为 0。macOS 来自 `magnifyWithEvent:`，Android 来自 `ScaleGestureDetector.onScale`。`Tree` 按 `PointerId` 各记一条按下和拖动。Android 按 `MotionEvent` 的 action index 展开每根手指。历史采样不入队。

入队前各端把坐标换成场景点，原点在左上：

| 端 | 换算 |
|---|---|
| macOS | 窗口坐标转进内容视图，Y 翻转。AppKit 已是点，不除 `backingScaleFactor` |
| Android | 物理像素除以 `DisplayMetrics.density`。density 不是有限正数就丢掉这次采样 |
| iOS | `location(in:)` 已是点，原点在视图左上，不翻转、不除 scale |

指针先进入 `hostshell` 的队列，渲染线程再交给 `core` 做命中和手势。三端共用这一份队列：

- 出队顺序与入队顺序一致。
- 未按下时的 `Move` 保留，用来做 hover。相邻的 `Move` 合并成最后一笔。滚轮事件保持原顺序。
- `Cancel` 原样送到 `Session::pointer`。`Tree::pointer` 会清掉按下的区域。macOS 没有系统 cancel。

还没有轨迹重采样。Android 按 pointer id 送出每一根手指。一个 `Scroll` 同时只锁定一根手指。

## 4. 按键与输入法

文本焦点在 `core::Tree`。点击（抬起且没有锁成拖动）才移动焦点。`Session::input` 把 `session::InputEvent` 交给当前焦点；没有焦点就丢掉。字符串、UTF-16 选区和组合区间在 `core` 的 `TextEditor`。复制、剪切、粘贴和方向键都走这里。声明了 `DragClaim` 的区域越过拖动阈值后锁成选区拖动。选区高亮和组合下划线由文本节点录进自己的 picture。

平台线程把 `Command::Input` 入队，和指针一样在下一帧排空，并登记这一帧。暂停时先留在队列里。`take_ime` 在输入派发之后和 `session.frame` 之后各读一次。`Show` 弹出键盘，`Update` 写入选区、组合区间和光标，`Hide` 收起键盘。组合过程中的 `InputEvent::Composing` 不另写一份缓冲。

macOS：带 Command、Control 或 Option 的 `keyDown:` 投递 `Key::Char`，不再走 `insertText:`。其余 `keyDown:` 调用 `interpretKeyEvents:`。`magnifyWithEvent:` 投递 `Phase::Zoom`，`zoom` 是这次的相对缩放。`AppleInterfaceThemeChangedNotification` 和 `NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification` 再读外观、减弱动态效果、提高对比度和粗体。`insertText:` 是 `Text`，`setMarkedText:` / `unmarkText` 是 `Composing`，`doCommandBySelector:` 把回车、退格和方向键收成 `Key`，其余 selector 吞掉并打印。`selectedRange` 和 `markedRange` 读主线程上的镜像。`firstRectForCharacterRange:` 返回内容视图 bounds 换到屏幕坐标后的矩形。`set_ime(true)` 在主线程 `makeFirstResponder`。

Android：`TextEvent` 的下标是 UTF-16。宿主先换成字节下标，再拆成已提交串和组合串。已提交串在末尾变长时用 `Text` 追加后缀。中间编辑取公共前缀，再 `DeleteRange` 加 `Insert`，映射成 `core::Edit`。组合区间变化用 `Composing`。

键盘类型和回车动作两端共用同一套序号。Android 再换成 `InputType` 和 `IME_ACTION`。

| keyboard | 值 | Android `InputType` |
|---|---|---|
| Text | 0 | `TYPE_CLASS_TEXT` |
| Multiline | 1 | 文本 + 多行 |
| Number | 2 | `TYPE_CLASS_NUMBER` |
| Phone | 3 | `TYPE_CLASS_PHONE` |
| Email | 4 | 文本 + email |
| Url | 5 | 文本 + uri |
| Name | 6 | 文本 + 人名 |
| Password | 7 | 文本 + password |

| action | 值 | Android `IME_ACTION` |
|---|---|---|
| None | 0 | 1 |
| Done | 1 | 6 |
| Next | 2 | 5 |
| Search | 3 | 3 |
| Go | 4 | 2 |
| Send | 5 | 4 |
| Previous | 6 | 7 |
| Newline | 7 | 1 |

macOS 没有这套 `KeyboardType` 映射。粘贴同时读 `public.utf8-plain-text` 和 `public.html`。Android 粘贴的 `html` 是 `None`。`KeyAction::Up` 不映射。软键盘可见时，同一轮已经有缓冲变化就不再映射 `KEYCODE_DEL`；没有缓冲变化才投递 `Backspace`。读缓冲在写回 `set_text_input_state` 之前。用户刚改过的串还没被会话确认时，丢掉文本不同的 `Update`，避免把刚删掉的字写回去。`KEYCODE_BACK` 在键盘可见时投递 `ImeDismissed`，不删字符。聚焦和本地按键修改之后 `set_text_input_state`，光标在末尾。键盘高度来自 `WindowInsets.Type.ime()`（8），并进 `Metrics.inset_bottom`。Manifest 的 `windowSoftInputMode` 是 `adjustNothing`，Surface 不跟着键盘缩小。inset 从非零变成 0 视为用户收起键盘，会话失焦。

窗口失焦见 §9，和文本焦点不同名。`Field` 用 `caretAt` 把插入符放到点击处。选区高亮和组合下划线由文本节点录进自己的 picture。

## 5. 帧

`Frame` 在渲染线程上执行。暂停期间到达的指针仍入队，恢复时再排空。

```text
① 处理队列里的 Lifecycle / Metrics / Surface
   Pause 且仍有未结束的按下 → 先补一个 Cancel
② 排空指针队列，调用 session.pointer；再排空 Input，调用 session.input
   命中用上一帧几何。无焦点的输入丢掉。监听器只改状态
③ FrameIn.paused，或还没有 Surface → 不 present。
   Cancel 丢掉本帧的 again。finish 仍发出 pending 里已经记下的下一次 token
④ session.frame → FrameOutput
   会话内部先排空 QuickJS job，再 flush，再按树的脏标记 layout / record
⑤ layer_epoch 未变、缩放与 installed 相同、且 needs_present 为假 → Skip
   不 submit_layers，也不 present
   needs_present 为真 → PresentCached：draw_metal / draw_vulkan 画已安装的显示列表
   否则 engine.submit_layers，成功后把 installed 记为这一档缩放，再 surface.present
⑥ 没有新的指针、metrics 或表面事件，且会话没有申请下一帧时，不再 request_frame
```

下一帧在这些命令到达、且当时没有在途帧时登记：指针、metrics 发生变化、`Resumed`、`Invalidate`、`SurfaceReady`，以及启动后的第一帧。`Input::resume` 固定返回真，所以 `Resumed` 一定会 `arm`，并置 `needs_present`，保留 `installed`。`Invalidate` 同样置 `needs_present`。成功 present 之后清掉 `needs_present`，静止画面回到 `Skip`。`hostshell` 仍然只看 `wants_frame`，不读树。会话在这一位里并上还没布局的变更、还没绘制的变更、没排空的 job，以及仍在衰减的滚动。脚本错误会清掉这一次的需求，避免用失败回调自己再要帧。

`set_viewport` 会把树标脏。metrics 与上一份 `PartialEq` 相同则不再调用 `session.metrics`，否则静止画面每帧都重新布局。窗口第一次就绪时一定下发。`installed` 在 `SurfaceDestroyed`、`MemoryWarning`，以及 `note_scale` 发现缩放变化时清掉。成功的 `SurfaceReady` 不清 `installed`。打开得到 `NotReady` 也不清。Android 先销毁再打开时，`installed` 已经是 `None`，下一帧走 `Submit`。表面还在、列表未变时，`Resumed` 和 `Invalidate` 走 `PresentCached`。

连续的 `PresentFailed`、`NotReady` 或 `BadScene` 未满 3 次时，下一帧仍登记 vsync。从第 3 次起记一条 `present failed; backing off`，并用 `Wake` 延迟下一次尝试：16ms 起翻倍，上限 1000ms。成功 present 后计数归零。`MemoryWarning` 清 `installed` 并丢掉图片缓存里的像素，下一帧 `submit_layers`。

`Session::frame` 返回 `FrameOutput`。`layers` 为 `None` 表示 epoch 未变。trait 的方法以 `session/src/lib.rs` 为准。布局失败时会话留着上一份成功的层树。

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

Android 的 `uifw-surface-drop` 在 `DropWait` 上等到 `Acks::signal`，然后才 `ANativeWindow_release`。平台线程不等待。渲染线程处理 `SurfaceDestroyed` 时不反过来等平台线程。macOS 不投递 `SurfaceDestroyed`。

已经打开的表面上，`present` 返回 `NotReady`、`PresentFailed` 或 `BadScene` 时留着 `Surface` 和 swapchain，置 `needs_present`。打开表面得到 `NotReady` 时留下 `SurfaceTarget` 并再登记一帧，不取消调度，也不清 `needs_present`。同一目标最多尝试 120 次，到顶记一条 `surface open failed; stopping retry` 并停止自动登记。之后的 `Resumed`，或该 id 的 `SurfaceChanged`，把次数清零再试一轮。`SurfaceDestroyed` 在应答前丢掉这次未打开的目标。其它打开失败结束渲染线程。`Surface` 只在匹配的 `SurfaceDestroyed` 时丢掉。销毁按 `SurfaceId` 回 ack。

尺寸变化不调用 `ImpellerVulkanSwapchain` 的丢弃。只有表面本身失效才重建，顺序仍是 GetVulkanInfo 的 instance、`vkCreateAndroidSurfaceKHR`、`ImpellerVulkanSwapchainCreateNew`。

## 7. 平台能力

`Services` 替代 platform channel。三端各一份实现，由 `hostshell` 在渲染线程调用。

字体按注册名取字节，启动时交给 `register_font`。`UIFWSans` 和 `UIFWSansCJK` 是启动时的两张脸。脚本 `registerFont` 把字节放进会话队列，渲染线程在 `submit` 之前 `take_fonts` 再 `register_font`。同名替换字节。新名字加上已经交出的名字最多 8 个，那两张启动脸不占名额。失败只记 family 和状态，这一帧继续，缺的 family 退回第一次成功登记的脸。图片由 `ImageDecoder` 在 `uifw-io` 线程解码成紧排 RGBA，结果经 `Command::ImageReady` 回到渲染线程的 `ImageCache`。`submit` 只读这份缓存。必选缺图记一次日志并且这一帧不提交，已安装的显示列表留在引擎里。可选缺图跳过该命令。`https:` 图片在这条线程下载，正文最多 4 MiB，日志只记 scheme 和 host。脚本 `reload(source)` 把名字排进会话，渲染线程在读缓存之前排空。`reload_image` 删掉该键并再要一帧。日志从渲染线程调用：macOS 写 stderr，Android 写 logcat，iOS 写 `os_log`。`Services::semantics` 收到语义树后，宿主把它交给平台辅助功能。

编辑器 Copy / Cut 经 `take_clipboard` 交出文字，`hostshell` 在渲染线程上 `registry.call("Clipboard", "setString", …)`。Clipboard 未注册时记 `not linked`，不 panic。触觉、系统音再各加方法。不加字符串消息通道。

## 8. 资产

| 键 | macOS | Android | iOS |
|---|---|---|---|
| 注册名 `UIFWSans` | `/System/Library/Fonts/Supplemental/Arial.ttf` | `assets/fonts/UIFWSans.ttf` | Bundle 内同名文件 |
| `asset:<name>` | 资产目录按 scale 桶找 `name@3x`、`name@2x`、`name`，扩展名 `.png` `.jpg` `.jpeg` `.webp`，ImageIO | `assets/images/` 下同一组候选，`AImageDecoder` | 宿主还没写 |

Android 与 iOS 的字体是可再分发的文件，注册名仍叫 `UIFWSans`。系统 Arial 不打进 APK 或 Bundle，字形度量和 macOS 上的 Arial 不同。

`AssetManager` 在 `HostActivity` 启动时从 Java 取到。渲染线程第一次要字体或图片时读取并解码，结果留在 `Services` 里。

密度变体按 scale 桶选择；色彩管理和降采样保持今天的设备 RGB、预乘。

## 9. 生命周期与 inset

`session::Lifecycle` 是 `Detached`、`Resumed`、`Inactive`、`Hidden`、`Paused`。`Hidden` 和 `Paused` 冻结指针。`Inactive` 不冻结。Android 的 `onPause` 对应 `Inactive`，`onStop` 对应 `Hidden`，`onResume` 对应 `Resumed`。iOS 进后台与回前台同样走 `Paused` / `Resumed`。macOS 的 `applicationDidBecomeActive` 与 `windowDidDeminiaturize` 投递 `Resumed`，`applicationWillResignActive` 投递 `Inactive`，`applicationDidHide` 与 `windowDidMiniaturize` 投递 `Hidden`。不投递 `Paused`。窗口关闭则退出。启动时 `activate` 会走到 `didBecomeActive`。`windowDidChangeOcclusionState:` 在 `NSWindowOcclusionState::Visible` 置上时投递 `Invalidate`。`applicationDidReceiveMemoryWarning:` 与 Android 的 `onTrimMemory` / `onLowMemory` 投递 `MemoryWarning`。

`Hidden` 或 `Paused` 到达时，若还有按下的指针，`hostshell` 把当时的指针队列冻结，并为每个指针补一个 `Cancel`，这一帧只送这份冻结队列。之后再来的指针、按键和 metrics 留到 `Resumed`。窗口焦点是单独的事件，不是文本焦点。

路由恢复不做。当前没有多窗口，也没有把原生控件嵌进这棵树。

`Metrics` 的四边 inset 交给 `Tree::set_viewport(x, y, w, h)`。根从 `(inset_left, inset_top)` 排，Taffy 的可用空间是扣掉四边之后的宽高。命中和录制用绝对坐标，不再另做换算。macOS 的 inset 是 0。窗口缩放和换屏幕都投递同一份 `Metrics`。

## 当前没有的宿主能力

- 无原生控件混排，无外接纹理。视频帧再对照 `TextureRegistry` 的接口形状。
- 一进程一窗口一会话。QuickJS 是单实例。

## 三端差异

| | macOS | Android | iOS |
|---|---|---|---|
| 窗口 | `NSWindow` + `CAMetalLayer` | `HostActivity` + `HostSurfaceView` | `UIView` + `CAMetalLayer` |
| 平台线程 | AppKit 主线程 | Java 主线程 | `UIApplicationMain` |
| vsync | `CVDisplayLink`，回调只入队 | 主线程 `Choreographer` | 主线程 `CADisplayLink` |
| 光栅 | Metal | Vulkan | Metal，调用同一 `draw_metal` |
| 指针 | `mouseDown` / `Dragged` / `Up` | `MotionEvent`，含 `ACTION_CANCEL` | `touches*`，含 `touchesCancelled` |
| 坐标 | 内容视图 + Y 翻转 | 像素 ÷ density | 已是点 |
| metrics | 默认 360×640，可缩放，inset 0 | 窗口尺寸 ÷ density。inset 变成根偏移并缩小可用空间 | bounds 与 scale。inset 同样只采集 |
| 生命周期 | `Inactive` / `Hidden` / `Resumed`；遮挡恢复投递 `Invalidate`；关闭即退出 | `onPause` → `Inactive`，`onStop` → `Hidden`，`onResume` → `Resumed` | 前后台 |
| 读回 | `UIFW_DUMP` 把纹理写成 PNG | `adb screencap` 只作人工查看 | 与 macOS 相同的纹理读回 |

## 实现约束

- crate 均 `publish = false`。`Shell` 含有 `Engine` 和会话，因此不是 `Send`。跨线程的只有命令队列、`Vsync`，以及有意标记为 `Send` 的窗口指针 newtype。
- `unsafe` 集中在 Vulkan 句柄、Android 窗口与 NDK 事件、objc2，以及那个 `Send` 指针。每处按 `engine/src/player/`、`engine/src/typography/` 和 `engine/src/vulkan.rs` 写 SAFETY 注释。workspace 的 `unsafe_code = deny` 仍有效。
- `hostshell` 的队列、过滤、相同列表跳过提交，以及表面应答，用无窗口单测。各端坐标换算沿用 `hosts/macos/src/pointer.rs` 这种单测。端上验收是出帧和点击改字。
- 场景格式保持版本 1。不做 GL 后备、轨迹重采样、平台视图和多窗口。不做显示列表的命令级 diff。`layer_epoch` 未变、缩放相同且 `needs_present` 为假则 `Skip`；`needs_present` 为真则 `PresentCached`。`Resumed` 与 `Invalidate` 置 `needs_present`。增量绘制走已落地的合成层，见 `docs/architecture.md`，不在这里另写一套禁令。
- 新依赖在引入时钉版本。`ndk` 0.9.0 与 `jni` 0.22.4 已钉住。它们不带 UI 框架或 Dart VM。要裁 Impeller 后端时放到以后的 engine 仓库，不在本仓库编 engine。
