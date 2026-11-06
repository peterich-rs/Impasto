# 平台宿主

macOS 与 Android 已落地：平台线程入队，渲染线程创建 `Engine` 和会话并 present。SDK 在 `third_party/impeller-sdk/<平台>/`。iOS 预构建矩阵没有这一列，所以没有 `crates/ios`。线程模型不按只有两台宿主来收死。

`hostshell` 不依赖 `quickjs`。一个可执行文件只链一种会话。`protocol`、`uicore`、`engine`、`crates/macos` 不手写 C。QuickJS 的 C 只在 `crates/quickjs`。`Engine` 与 QuickJS 都是 `!Send`。

## 分层

```text
quotes.js
  → quickjs 会话（渲染线程打开）
  → session::Session
  → hostshell（队列、帧调度、submit）
  → macos / android
  → engine::Engine
```

宿主实现 `Services`、`Surface`、`Vsync`，并交给 `hostshell` 一个 `Send` 闭包，在渲染线程上打开会话。`Engine` 的所有权留在 `hostshell`。`Surface` 的公开接口不出现 `CAMetalLayer` 或 `VulkanSwapchain`。

## 启动

平台线程准备脚本、字体、图片和窗口，然后回到事件循环。渲染线程创建 `Engine`、`register_font`、调用会话闭包。平台线程不在启动函数里等待表面。

启动 URL 在 `hostshell::start` 之前写入 Shared `launch_url`。macOS 来自 argv `--url`，Android 来自显式 Intent 的 `getData()`。当前宿主没有注册 URL scheme。

退出码：`0` 正常，`2` 引擎不可用，`3` 解释器 `NotLinked` 或 `Runtime`，`4` 启动期脚本失败。运行期脚本错误只记日志。

## 线程

| 对象 | 线程 |
|---|---|
| 窗口、`CAMetalLayer` 的创建和挂载、`ANativeWindow` 的获取 | 平台线程 |
| `ANativeWindow_release` | 平台侧 `surface-drop`，必须在渲染线程应答 `SurfaceDropped` 之后 |
| `Engine`、会话、`Shaper`、`Surface::present` | 渲染线程 |
| vsync 回调 | 平台自己的线程，只入队 `Frame` |

Android 的 vsync 是主线程 `Choreographer`。macOS 是 `CVDisplayLink`。渲染线程是同步循环，不进 async 运行时。相邻的 `Pointer(Move)` 合并成队尾那一个。`Down`、`Up`、`Cancel` 不合并。

## 指针

入队前换成场景点，原点在左上。

| 端 | 换算 |
|---|---|
| macOS | 窗口坐标进内容视图，Y 翻转。AppKit 已是点，不除 `backingScaleFactor` |
| Android | 物理像素除以 `density`。density 不是有限正数就丢掉 |
| iOS | `location(in:)` 已是点，不翻转 |

`Zoom` 的 `zoom` 是这次事件的相对缩放。macOS 来自 `magnifyWithEvent:`，Android 来自 `ScaleGestureDetector`。没有轨迹重采样。一个 `Scroll` 同时只锁定一根手指。

## 输入法

`Session::input` 只在有焦点时派发。`take_ime` 在 `frame` 之后交给平台。用户收起键盘时宿主投递 `ImeDismissed`，会话失焦。

Android 的 `TextEvent` 下标是 UTF-16，宿主先换成字节下标。键盘类型和回车动作两端共用同一套序号，Android 再映射到 `InputType` 和 `IME_ACTION`。macOS 没有这套 `KeyboardType` 映射。`windowSoftInputMode` 是 `adjustNothing`。键盘高度进 `Metrics.inset_bottom`。inset 从非零变成 0 视为用户收起键盘。

## 帧

```text
队列里的 Lifecycle / Metrics / Surface
→ 指针，再 Input
→ 没有 Surface 或本帧取消则不 present
→ session.frame
→ epoch 未变且不需要 present 则 Skip；需要 present 则 PresentCached；否则 submit 再 present
→ 没有新事件且会话不要下一帧，就不再 request_frame
```

`hostshell` 只看 `wants_frame`，不读树。会话在这一位里并上未布局、未绘制、未排空的 job，以及还没结束的运动。脚本错误清掉这一次的需求。

`Hidden`、`Paused`、`MemoryWarning` 调用 `Engine::trim` 进入 `Flush`。连续 `PresentFailed`、`NotReady` 或 `AcquireMiss` 从第 3 次起退避，16ms 起翻倍，上限 1000ms。`BadScene` 不计入。同一表面打开 `NotReady` 最多再试 120 次。

## 表面

`SurfaceChanged` 只更新尺寸，Vulkan 链保留。`SurfaceDestroyed` 由渲染线程丢掉 `Surface` 并应答。Android 的 `surface-drop` 等到应答后才 `ANativeWindow_release`。macOS 不投递 `SurfaceDestroyed`。尺寸变化不丢链。空 acquire 是 `AcquireMiss`；Vulkan 连续 3 次后用同一窗口重建。

## 资产

| 键 | macOS | Android |
|---|---|---|
| 字体 `Sans` | `/System/Library/Fonts/Supplemental/Arial.ttf` | `assets/fonts/Sans.ttf` |
| `asset:<name>` | 资产目录按 scale 桶找 png/jpg/webp，ImageIO | APK `assets/images/`，`AImageDecoder` |

`SansCJK` 是第二张启动脸。脚本 `registerFont` 最多再加 8 个名字。必选缺图这一帧不提交。可选缺图跳过该命令。`https:` 图片在 `io` 线程下载，正文最多 4 MiB。

## 生命周期

`session::Lifecycle`：`Detached`、`Resumed`、`Inactive`、`Hidden`、`Paused`。`Hidden` 和 `Paused` 冻结指针，并给未结束的按下补 `Cancel`。`Resumed` 丢掉卡住的 vsync token 再武装一次。定时器 `Wake` 在已有 vsync token 时不抢跑这一帧。

| | macOS | Android |
|---|---|---|
| 隐藏 | `applicationDidHide`、最小化 → `Hidden`。不投递 `Paused` | `onStop` → `Hidden`，`onPause` → `Inactive` |
| 恢复 | `didBecomeActive`、还原 → `Resumed`。遮挡恢复 → `Invalidate` | `onResume` → `Resumed` |
| 内存 | `applicationDidReceiveMemoryWarning` | `onTrimMemory` / `onLowMemory` |

`Metrics` 的 inset 交给 `Tree::set_viewport`。macOS 的 inset 是 0。默认内容 360×640。

## 还没有

无原生控件混排，无外接纹理，无多窗口，无 iOS 宿主，无 GL 后备，无轨迹重采样，无命令级显示列表 diff。
