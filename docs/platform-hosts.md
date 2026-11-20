# 平台宿主

macOS 与 Android 的平台线程将事件入队，渲染线程创建 `Engine` 和会话并上屏。两个宿主各自链接对应目标的 Impeller SDK。[Impeller SDK](../crates/impeller-sys/README.md)

## 目录与 Cargo 包

平台宿主统一放在 `crates/embedder/`。公共 crate 负责启动与关闭、队列、帧调度和宿主模块。各平台 crate 依赖公共 crate，并在自己的目录中维护窗口、输入、原生表面和平台工程。

| 目录（相对 `crates/embedder/`） | Cargo 包 | 状态 |
|---|---|---|
| `common/` | `embedder` | 公共宿主 |
| `android/` | `embedder-android` | 已接入，Gradle 工程位于 `apk/` |
| `macos/` | `embedder-macos` | 已接入 |
| `ios/` | `embedder-ios` | 待接入 |
| `windows/` | `embedder-windows` | 待接入 |
| `linux/` | `embedder-linux` | 待接入 |

平台目录随实现加入 Cargo workspace。Android 的 JNI 库名为 `libimpasto.so`，macOS 的 Cargo 可执行目标名为 `macos`。

## 分层

`embedder` 通过 `session` 使用帧和输入类型。平台宿主启用 `javascript/ui` 打开应用会话。`protocol`、`uicore`、`engine`、`crates/embedder/macos` 不手写 C。QuickJS 使用 Cargo 构建的 quickjs-jit 0.14.2，Rust 绑定在 `crates/javascript/src/ui/player/vm`。`Engine` 与 QuickJS 都是 `!Send`。

```text
平台适配器 → 窗口事件、输入、vsync → embedder
embedder → 驱动 session::Session（JavaScript UI 会话）
embedder → 提交 LayerTree → engine::Engine → 平台表面
```

宿主实现 `Services`、`Surface`、`Vsync`，并交给 `embedder` 一个 `Send` 闭包，在渲染线程上打开会话。`Engine` 的所有权留在 `embedder`。`Surface` 的公开接口不出现 `CAMetalLayer` 或 `VulkanSwapchain`。

## Android 宿主

Android 使用 Kotlin 和 AndroidX，最低 API 为 24。默认入口是 `dev.impasto.host.ImpastoActivity`。`ImpastoHost.attach` 接收现有 Activity 和 `LifecycleOwner`。Fragment 传入 `viewLifecycleOwner`，并将 `host.view` 放入容器。关闭宿主会释放它的 View 会话。单个 View 会话迁移期间先调用 `retainSession()`，再将返回的句柄交给 `ImpastoHost.attach(session = ...)`。放弃迁移时，主线程调用 `ImpastoSession.close()`。 [独立应用与原生 Fragment 示例](../apps/android-acceptance/README.md)

- 权限弹窗由 AndroidX Activity Result 管理。默认 Activity 在等待权限结果时保留会话。嵌入宿主可用 `retainSessionForRecreation()` 接续同一个逻辑 Activity 重建后的结果。主动迁移到另一 Activity 或关闭时，旧窗口的请求结束。 [Activity Result 生命周期](https://developer.android.com/training/basics/intents/result)
- 多窗口宿主创建一个 `ImpastoWindowSession`，给每次 `ImpastoHost.attach(windowSession = ...)` 传入同一对象。每个窗口有自己的脚本环境、视口、时钟、焦点、语义树和绘制表面。脚本使用 `@impasto/app-state` 的 `WindowState.get/set` 显式共享字符串，通过 `WindowState.addEventListener` 订阅变更键。数据在应用会话结束时释放，跨进程数据仍走持久化。关闭应用会话会同时关闭它管理的宿主。
- Android API 26 以上，`Field` 的 `autofill` 可声明 `username`、`password`、`newUsername`、`newPassword` 或 `emailAddress`。Android 自动填充服务读取虚拟字段。填充结果使用控件的文本替换逻辑。密码不进入 TalkBack 的可读文本。 [Android 虚拟字段接口](https://developer.android.com/reference/android/view/ViewStructure)
- `PlatformView` 的 `kind` 对应 Android 宿主的 `platformViewFactories` 注册项。UI 树提供矩形位置、祖先滚动和裁剪，主线程负责创建、输入、焦点和释放。原生 View 位于 raster 表面之上，支持平移和矩形裁剪。带缩放、旋转、透明度分组、圆角裁剪或滤镜的路径会略过该原生槽位。
- `ImpastoView.openExternalTexture` 返回可传给生产者的 Android Surface，同名 `ExternalTexture` 图元消费帧。每个宿主最多四路，单帧上限 2,073,600 像素，队列保留最新帧。当前支持 RGBA 和 BT.601 limited-range YUV 4:2:0 输入，GLES 和 Vulkan 走现有图像上传。关闭前应先停止生产者。GPU 直接导入、同步 fence 与受保护内容仍待实现。

### 与 Flutter 的职责对照

源码基线为本地 `build/impeller-source/flutter` 的 `77f043bc63196b9979cc09c5b97bb6d9b00be34d`。Android embedder 位于 `engine/src/flutter/shell/platform/android/`，Dart 框架位于 `packages/flutter/lib/src/`。Flutter 的 Activity 委托管理引擎接入，View 单独连接 Renderer，输入法和无障碍各有自己的对象。

| Flutter Android embedder | Impasto Android embedder |
|---|---|
| `FlutterActivity`、`FlutterActivityAndFragmentDelegate`、`LifecycleChannel` | `ImpastoActivity` 是 AndroidX `ComponentActivity`。`ImpastoHost` 结合宿主生命周期、View 接入和窗口焦点发布状态。返回键经 `OnBackPressedDispatcher`。 |
| `FlutterEngine`、`FlutterJNI`、`PlatformViewAndroid` | `NativeBridge` 统一 JNI 声明。Rust 给每个接入分配独立会话句柄。关闭后，迟到的回调按旧句柄查询会失败。平台主线程通过队列送事件。 |
| `FlutterView`、`FlutterSurfaceView`、`FlutterRenderer` | `ImpastoView` 组织输入和表面。会话就绪、窗口和视口齐备后打开绘制表面。当前表面首次成功提交后调用 `reportFullyDrawn`。实际呈现时间由设备侧 SurfaceFlinger 验证。 |
| `AndroidTouchProcessor`、`KeyboardManager`、`VsyncWaiter` | `PointerInput` 映射指针、设备、滚轮与缩放。`TextInput` 优先处理编辑键。`FrameScheduler` 每次安排一个 Choreographer 回调，Rust 只接受匹配的帧 token。 |
| `ImeSyncDeferringInsetsCallback`、`TextInputPlugin`、`InputConnectionAdaptor` | `ViewportObserver` 用 AndroidX 读取系统栏、刘海、系统手势区和 IME，动画期间逐帧报告。编辑连接按当前字段更换，旧连接失效。批量编辑退出最外层才向会话提交快照。物理键在选区插入，先结束当前组合标记。 |
| `AccessibilityBridge` | AndroidX 节点 provider 将语义 ID 映射到稳定的虚拟节点，处理触摸探索、输入焦点、无障碍焦点和滚动事件。点击、滚动、设置文本和修改焦点字段的选区由会话执行。文本和 UTF-16 选区的变化通过 Android 无障碍事件发布。 |
| `PlatformPlugin` | 宿主实现剪贴板、外链、文件目录和系统设置读取。AndroidX Activity Result 驱动系统文档选择。`FileSystem.pick()` 返回应用私有目录中的相对路径，可直接用于 `FileSystem.read()`。取消选择返回 `null`，宿主关闭时释放本次请求和 Activity Result 注册。 |

Android 启动前从 Intent 读取初始 URL，运行中由 `onNewIntent` 发布 `linking` 的 `url` 事件。`singleTop` 使位于栈顶的 Activity 接收新 Intent。重建 Activity 时，非空的有效任务快照会抑制先前已交付的启动 Intent。退到后台时先投递 `Hidden`，再隐藏 View，使系统可以回收表面。恢复时重新显示 View，原会话接续。Activity 销毁时释放其会话所有权。应用可在启动时用 `@impasto/app-state` 的 `Restoration.get(key)` 读取系统任务快照，通过 `Restoration.set(key, value)` 更新和通过 `null` 删除条目。快照总量以编码后的 32 KiB 为上限。默认 Activity 提供稳定键，嵌入宿主可向实现 `SavedStateRegistryOwner` 的生命周期所有者传入稳定的 `restorationId`。大型草稿使用应用持久化。

应用可用 `scene.windowMetrics()` 读取宿主 View 的完整逻辑尺寸、像素尺寸、安全区、IME inset 和系统手势区。Android 另提供 `displayId`、`refreshRate` 和 `displayFeatures`。特征包含折痕、铰链和切口，边界以当前 View 的逻辑坐标表示，折叠特征还含姿态和是否分隔窗口。用 `scene.onWindowMetrics()` 订阅变化，并在组件销毁时用 `scene.offWindowMetrics()` 移除监听。`scene.metrics()` 表示扣除遮挡后的布局区域。macOS 和无窗口会话的系统手势区默认为零，显示特征列表默认为空。

`just android-device-test` 使用独立验收包，结束后卸载该包及仪器测试包。连接多台设备时，通过 `ANDROID_SERIAL` 选择本次测试设备。手动安装的性能和 Flutter 对照包在采集结束后也应卸载。横竖屏测试读取 SurfaceFlinger 的 active buffer 尺寸，按缓冲区变换换算到窗口方向，再用 PixelCopy 确认新表面可以读取到图像。[SurfaceFlinger LayerProto](https://android.googlesource.com/platform/frameworks/native/+/04d24da36e/services/surfaceflinger/layerproto/layers.proto) · [缓冲区方向编码](https://android.googlesource.com/platform/frameworks/native/+/27c20ff12d67b4667edb01160289afc8a4e23373/services/surfaceflinger/LayerProtoHelper.cpp)

传给渲染线程的表面包含 `SurfaceId`、Android 像素尺寸、逻辑视口和 `ANativeWindow`。`surfaceDestroyed` 等待渲染线程丢弃该 ID 的待打开目标和交换链，再释放原生窗口引用并返回 Android。渲染线程提前退出时，在释放表面资源后完成等待。View 更新由主线程处理，释放后的 View 丢弃迟到的更新。[Android SurfaceHolder.Callback](https://developer.android.com/reference/android/view/SurfaceHolder.Callback#surfaceDestroyed(android.view.SurfaceHolder))

### 兼容性差距与验收

完整的框架能力范围、架构契约、实施顺序与发布条件见 [Impasto Android 框架能力建设方案](design/android-framework-plan.md)。

| 能力 | Flutter 源码与行为 | Impasto 待完成与验收 |
|---|---|---|
| 启动 | `FlutterLoader` 在后台准备运行时和资源，并提供异步等待入口。Activity、Engine 和首帧回调各自有明确的接入时机。 | `nativeAttach` 返回会话句柄，渲染线程准备 Engine 和脚本。会话准备期间可取消，尚未交给渲染线程的窗口可立即释放。仍需定标真实设备首帧与恢复预算。 |
| 表面和 GPU | `PlatformViewAndroid::NotifyDestroyed` 等待光栅线程拆除表面。`AndroidContextDynamicImpeller` 在探测 Vulkan 以及检查设备和驱动后选择 Vulkan 或 GLES。`FlutterSurfaceView` 通过 `RenderSurface` 和 Renderer 连接，另有 TextureView 路径。 | 具备 Vulkan 关闭修复的 SDK 上优先尝试 Vulkan。当前预构建归档默认走 GLES。APK 支持 `arm64-v8a` 和 `x86_64`。两条路径已在 API 34 arm64 模拟器上验证旋转与恢复，仍需覆盖实际 GPU 驱动丢失和更广的设备矩阵。 |
| 生命周期和恢复 | `LifecycleChannel` 合并 Activity 和焦点。Dart `services/binding.dart` 补齐状态迁移，`scheduler/binding.dart` 按状态开关帧。`RestorationChannel` 在启动时交付恢复数据。 | 窗口焦点和 Activity 状态合并。AndroidX `SavedStateRegistry` 提供任务快照，已在 Activity 重建和 16 KB 模拟器进程回收后验证。分屏、外接屏和反复迁移仍需验证。 |
| 视口与时钟 | `FlutterView` 分开报告系统栏、IME、手势区、瀑布屏边缘和折叠特征。`VsyncWaiterAndroid` 提供 NDK 与 Java 路径。 | 视口含安全区、IME inset、系统手势区和显示特征。仍需实测折叠屏姿态、API 24–29 的 IME inset 及动画、API 33 以上的帧时间线和 60/90/120 Hz 动态切换。 |
| 帧流水线 | `shell/common/animator.cc` 通过有容量上限的 `FramePipeline` 投递帧，`Shell::OnAnimatorDraw` 在光栅任务运行器上调用 `Rasterizer`。Dart 调度器负责框架帧回调。 | Impasto 在同一渲染线程内同步推进脚本、布局、录制和呈现。用帧阶段埋点区分 CPU 工作、GPU 等待和排队延迟，再评估分离各阶段的收益与同步成本。 |
| 指针和键盘 | `AndroidTouchProcessor` 处理多指变化及触控板平移缩放，底层 `PointerDataPacketConverter` 合法化事件序列，Dart `gestures/binding.dart` 提供可选重采样。`KeyboardManager` 根据框架处理结果走编辑或系统重派发。 | 补齐跨设备的指针事件序列、触控板和键盘处理结果。验收覆盖鼠标按钮、滚轮、触控板、手写笔、硬件键盘、手势取消与失焦期间的按键恢复。 |
| IME 和无障碍 | `TextInputPlugin` 按输入目标维护连接、批量编辑、增量更新和自动填充。`AccessibilityBridge` 连接语义树、平台无障碍动作和原生 View。 | 已接入虚拟字段自动填充。富内容与无障碍遍历仍需扩展。用多个输入法验证组合文本、UTF-16 选区、编辑器切换与键盘显隐，用 TalkBack 验证焦点、朗读、滚动和编辑。 |
| 插件和嵌入 | `FlutterEngineConnectionRegistry` 转发 Activity 接入、配置变更、权限与 Activity result。`PlatformViewsController` 与 `FlutterRenderer` 协作混排原生 View 和外部纹理。 | Android 支持权限、文档选择、系统文本分享和预测返回。保留会话可跨逻辑 Activity 重建接续权限请求。原生 View 覆盖层和媒体纹理已接入。待验证系统弹窗、更多原生 View 效果及 GPU 直接导入。 |

发布验证应覆盖最低 API 24、旧版 IME 路径、API 33–34 帧时间线和 API 35 以上的边到边窗口。模拟器用于回归生命周期和输入协议，设备用于验证 GPU 驱动、TalkBack、输入法和刷新率切换。性能对照使用相同设备、刷新率和脚本，分别记录冷启动首屏、前台恢复后的首帧、实际呈现间隔及输入到呈现延迟。现有 Pixel 9 回放的 CPU 分段将行情详情的响应式处理与布局列为待优化瓶颈，呈现线程改动仍需在同一回放下重新量测。

连接支持 Vulkan 和 GLES 的 Android 设备或模拟器后运行下列检查。Android 测试覆盖焦点与宿主事件的不同顺序、IME 组合区间与 UTF-16 选区。两种后端分别运行 Gallery 冷启动、前后台恢复、三轮旋转和同 Activity 独立会话测试。仪器测试使用 `PixelCopy` 验证缓冲区可读，旋转用 SurfaceFlinger 活跃缓冲区核对实际像素尺寸。调试 APK 可通过 Intent extra `impasto.renderer` 选择 `vulkan` 或 `gles`。

GLES 包装 EGL 窗口的默认帧缓冲。窗口配置提供 24 位深度和 8 位模板附件，供 Impeller 的裁剪和路径覆盖使用。GLES 的 Canvas 像素回归检查曲线描边在包围盒中的覆盖率。 [Flutter 的 EGL 窗口配置](https://github.com/flutter/flutter/blob/77f043bc63196b9979cc09c5b97bb6d9b00be34d/engine/src/flutter/shell/platform/android/android_context_gl_impeller.cc#L122-L127)

```sh
just android-build-check
just android-device-test
```

## 启动

Android 平台线程创建 View、读取启动参数、提交准备任务后回到事件循环。渲染线程选择 Engine、加载字体和脚本并报告会话就绪。Android 把准备期间收到的最新视口和表面交给已就绪的渲染线程。macOS 使用同步启动适配入口 `embedder::start`，同样在渲染线程准备 Engine 并打开脚本。

启动 URL 在应用脚本执行前写入 Shared `launch_url`。macOS 来自 argv `--url`，Android 来自 Intent 的 `getData()`。CLI 根据 `app.toml` 的 `[[android.deep_links]]` 生成 Intent filter，暖启动通过 `Linking` 事件交付。

退出码：`0` 正常，`2` 引擎不可用，`3` 解释器 `Runtime`，`4` 启动期脚本失败。运行期脚本错误只记日志。

## 线程

| 对象 | 线程 |
|---|---|
| 窗口、`CAMetalLayer` 的创建和挂载、`ANativeWindow` 的获取 | 平台线程 |
| `ANativeWindow_release` | Android 主线程，收到渲染线程的表面销毁应答后 |
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

Android 的编辑区间使用 UTF-16，UI 会话转换到 Rust 字符串的有效边界。Android 将键盘类型和回车动作映射到 `InputType` 与 `IME_ACTION`。Activity 使用 `adjustResize`，支持 AndroidX 在低版本 Android 上读取 IME inset。键盘高度进 `Metrics.inset_bottom`。inset 从非零变成 0 时投递 `ImeDismissed`。

## 帧

```text
队列里的 Lifecycle / Metrics / Surface
→ 指针，再 Input
→ 没有 Surface 或本帧取消则不 present
→ session.frame
→ epoch 未变且不需要 present 则 Skip；需要 present 则 PresentCached；否则 submit 再 present
→ 没有新事件且会话不要下一帧，就不再 request_frame
```

`embedder` 根据会话的 `wants_frame` 调度，覆盖未布局、未绘制、未排空的 job 和未结束的运动。公共 `FrameScheduler` 在提交或呈现前即可登记下一次 vsync，回调入队后由渲染线程串行处理。失败进入退避时作废这次登记。Android 的调度器按当前显示器的刷新率计算帧预算，呈现时间取自 `Choreographer` 的首选 timeline（API 33 及以上）。脚本错误清掉这一次的需求。

`Hidden`、`Paused`、`MemoryWarning` 调用 `Engine::trim` 进入 `Flush`。连续 `PresentFailed`、`NotReady` 或 `AcquireMiss` 从第 3 次起退避，16ms 起翻倍，上限 1000ms。`BadScene` 不计入。同一表面打开 `NotReady` 最多再试 120 次。

### Android 帧性能

Gallery 的 release 构建可通过 `-PimpastoProfile=true` 生成可由 adb 采样的 `dev.impasto.host.profile`。它有独立的应用数据。回放使用竖屏，性能面板关闭，详情固定为 00700。脚本采集冷启动、详情入场、首页行情、分时、日 K、行情开关各一组的光标拖动和详情页纵向滚动，以及列表纵向拖动。

```sh
just gallery-dist
crates/embedder/android/apk/gradlew -p crates/embedder/android/apk -PimpastoProfile=true :app:assembleRelease
adb install -r crates/embedder/android/apk/app/build/outputs/apk/release/app-release.apk
adb shell am start -W -n dev.impasto.host.profile/dev.impasto.host.ImpastoActivity
FRAME_REFRESH=120 tools/android-frame-replay.sh target/android-perf/120 3
FRAME_REFRESH=60 tools/android-frame-replay.sh target/android-perf/60 3
python3 tools/android-frame-report.py target/android-perf/120 --hz 120
python3 tools/android-frame-report.py target/android-perf/60 --hz 60
```

报告脚本使用 Python 的 `perfetto` 包。安装后等 Gallery 首页显示再开始采集。原始记录附设备指纹、分辨率、应用路径、框架提交和 SDK 信息。回放结束恢复原刷新率设置。单条手动采集可用 `tools/android-frame-trace.sh OUTPUT`。只查列表拖动时设置 `FRAME_SUITE=list`。只查详情页时，使用 `FRAME_SUITE=detail FRAME_REFRESH=120 tools/android-frame-replay.sh target/android-perf/detail 3 7`。该回放包含日 K 静置、暂停行情拖光标、行情开关各一组纵向滚动。纵向手势从盘口前开始，滑过逐笔和深度区块，再滑回图表。

Flutter 参考从 Gallery 的同一行情引擎导出确定性股票列表、分时、日 K 和盘口，以 200 ms 周期播放快照。用同一台设备采集两个应用的列表滚动、图表光标和详情滚动。参考工程记录 Flutter SDK 版本。其快照在构建时生成，运行测量范围是 Flutter 的布局、绘制和呈现。Impasto Gallery 仍在每帧执行行情仿真，整体耗时还包含这项工作。[Flutter 行情参考](../tools/android-flutter-reference/README.md)

用 `FRAME_SUITE=daily` 只跑日 K 静置。用 `pnpm -C apps/gallery-solid run build --freeze-stock=book` 或 `--freeze-stock=chart` 生成冻结对应区块行情输入的 bundle，再打包并运行同一回放。冻结主图时仍可切换周期。完成对照后，用无参数的 `pnpm -C apps/gallery-solid run build` 恢复实时行情的 bundle。

`Impasto frame` 覆盖渲染线程处理一帧的完整区间。报告按发起呈现的帧计算 P50、P95、峰值和超预算比例，另计跳过呈现与新层树提交次数。`Impasto work` 记录根布局、局部布局、同输出文字、输入、响应式更新和 Canvas 次数；`Impasto Canvas commit` 记录节点、内容盒高度和绘制命令数。Pixel 9 竖屏详情页的主图高度是 204 点。Vulkan 的 acquire、draw 和 present 各有独立区间。

`Impasto Android surface destroy` 记录 Android 主线程等待渲染线程拆除表面的时长，可与同一时间段的渲染线程区间对照。

SurfaceFlinger 的 `--latency` 记录提供实际呈现时间。报告将对应的 `queueBuffer` 与送达应用的触摸事件配对，保留配对样本数和覆盖率。首屏统计从启动到首页首个走势 Canvas 实际呈现，详情入场统计从点击到详情主图实际呈现。提交帧 CPU 耗时、实际呈现间隔与输入到呈现延迟分别给出 P50、P95、P99。滚动中的相邻配对帧用于估算错过的显示周期。设备报告的刷新率需与指定周期匹配，报告才输出这一项。

#### 2026-11-20 Pixel 9 回放

Pixel 9，Android 17 API 37，1080 × 2424，arm64 Vulkan。60 Hz 和 120 Hz 各采集三轮，实际显示周期分别为 16.67 ms 和 8.33 ms。回放使用 standalone Vulkan 关闭修复前的 Gallery profile APK。表中错过周期按两个呈现都配对的相邻滚动帧计算。

| 场景 | 刷新率 | 提交帧 CPU P95 | 输入到实际呈现 P95 | 错过周期 | 呈现配对覆盖率 |
|---|---:|---:|---:|---:|---:|
| 列表拖动 | 60 Hz | 21.93 ms | 57.78 ms（180 次） | 6.04% | 78.86% |
| 实时详情滚动 | 60 Hz | 20.26 ms | 58.06 ms（166 次） | 3.10% | 71.75% |
| 暂停行情后详情滚动 | 60 Hz | 21.44 ms | 58.32 ms（153 次） | 3.09% | 73.02% |
| 列表拖动 | 120 Hz | 20.48 ms | 41.11 ms（440 次） | 9.71% | 94.32% |
| 实时详情滚动 | 120 Hz | 14.12 ms | 38.12 ms（480 次） | 5.23% | 83.47% |
| 暂停行情后详情滚动 | 120 Hz | 15.80 ms | 40.69 ms（442 次） | 6.16% | 82.84% |

六组输入到实际呈现的 P95 均超过三个显示周期。60 Hz 的两组详情滚动达到错过周期低于 5% 的初始目标。冷启动首个走势 Canvas 呈现 P95 为 509.42 ms（60 Hz，三次配对）和 488.18 ms（120 Hz，两次配对）。这一组记录保存在 `target/android-perf/pixel9-framework-20261120/`。

同一台 Pixel 9 也运行了 Flutter 3.44.0 参考，各刷新率采集一轮。列表拖动的 UI 帧 P95 为 11.75 ms（60 Hz）和 10.36 ms（120 Hz），光栅帧 P95 为 5.03 ms 和 4.04 ms。Flutter 报告保留完整采样窗口内的呈现间隔，包含滑动之间的空闲。定标仍需要对应连续手势区间的更多对照样本。参考记录保存在 `target/android-perf/pixel9-flutter-20261120/`。

#### 修复前的 GLES 列表回放

该旧构建的 EGL 窗口配置缺少深度和模板缓冲，Canvas 轨迹会画成实心方块。这组数据保留作历史记录，修复后需重新采集实际呈现性能。

同一台 Pixel 9 使用预构建 SDK 自动选用 GLES，60 Hz 和 120 Hz 各运行三轮列表拖动。表中的 CPU 耗时统计滚动更新提交帧，输入延迟来自触摸事件与 SurfaceFlinger 实际呈现的配对。此次使用 GLES，上表使用 Vulkan，测试后端与构建均有变化。原始数据与 JSON 报告位于 `target/android-perf/pixel9-gallery-gles-followup/`。

| 刷新率 | 滚动帧 CPU P95 | 输入到实际呈现 P95 | 错过呈现周期 | 呈现配对覆盖率 |
|---|---:|---:|---:|---:|
| 60 Hz | 13.18 ms | 57.78 ms（578 次配对） | 0.42% | 99.78% |
| 120 Hz | 7.48 ms | 40.31 ms（740 次配对） | 2.80% | 73.69% |

错过呈现周期比例均低于 5%。输入到实际呈现的 P95 仍超过 60 Hz 的 50 ms 和 120 Hz 的 25 ms 门槛。Flutter 参考另补采每档两轮。列表的 UI 帧 P95 为 15.12 ms 和 8.37 ms，光栅帧 P95 为 6.08 ms 和 4.75 ms。Flutter 报告当前提供帧阶段与实际呈现间隔，输入配对和启动、恢复绝对门槛仍待补齐。补采数据位于 `target/android-perf/pixel9-flutter-followup/`。

在 Perfetto 内展开一条慢的 `Impasto frame`，用帧 ID 对照计数和渲染线程的调度状态。以下埋点按帧汇总，解析结果在报告的 `diagnostics` 和 `scrolling` 中。

| 埋点 | 排查内容 |
|---|---|
| `Impasto script work` | 响应式回调次数、最外层回调累计时间、原生 mutation 次数和应用时间，以及实际应用的 `SetText` 次数。mutation 应用可能发生在回调内，相等值计数来自原生入口的值比较。 |
| `Impasto root`、`Impasto layout work` | 首个到达根的失效原因、节点 ID 和布局盒。Taffy 的耗时包含其测量回调，另记 Taffy 缓存命中、普通文字测量次数和段落度量同步时间。 |
| `Impasto text probes` | 文字复用检查的总耗时与测量耗时。同一节点的查询按有效宽度和测量模式共享结果。`repeat` 记录共享次数。`hit`、`miss` 是检查期间 Impeller 段落测量缓存的底层查询数，一次文字测量可能查多次。 |
| `Impasto record work`、`Impasto pictures` | 组装层树时访问的节点数、写入 picture 的命令数，以及引擎准备 picture 和构建顶层 DisplayList 各自的耗时。 |
| `Impasto scroll work` | 滚动或回弹使绘制失效。报告按帧区分根布局、仅有 Canvas 绘制、及两者均未发生的滚动。实际呈现间隔使用两端都匹配到屏幕时间的连续滚动提交。 |

需要查函数级 CPU 热点时，可另开一轮，用 NDK 的 Simpleperf 按 `cpu-clock:u` 采日 K 静置调用栈，符号从同一 APK 对应的 Cargo 输出读取。

```sh
python3 "$ANDROID_NDK_HOME/simpleperf/app_profiler.py" --disable_adb_root \
  -p dev.impasto.host.profile -lib "$PWD/target/aarch64-linux-android/release" \
  -r '-e cpu-clock:u -f 499 -g --duration 10' -o daily-simpleperf.data
```

#### Pixel 9 回放

同一 release profile APK 在 120 Hz 复跑三轮，60 Hz 回归一轮。下表为 120 Hz，单位是毫秒。基线取同一回放脚本的优化前 APK。预算为 8.33 ms。

| 场景 | 基线 P95 | 提交帧 | P50 | P95 | 峰值 | 超预算 | 实际呈现间隔 P95 | 输入到呈现 P95 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 首页行情 | 18.93 | 729 | 5.83 | 18.43 | 26.00 | 20.58% | 141.80 | — |
| 00700 分时 | 58.02 | 84 | 42.24 | 49.16 | 50.47 | 100% | 216.97 | — |
| 00700 日 K | 62.64 | 87 | 45.39 | 50.71 | 53.52 | 100% | 225.22 | — |
| 暂停行情拖光标 | 17.47 | 665 | 11.85 | 15.27 | 20.89 | 95.94% | 25.07 | 39.08 |
| 行情开启拖光标 | 31.10 | 718 | 10.91 | 27.20 | 50.75 | 85.10% | 41.73 | 42.63 |
| 列表纵向拖动 | 8.85 | 1628 | 4.88 | 8.25 | 34.19 | 4.85% | 16.73 | 34.24 |

冷启动到首屏走势呈现的 P50 为 432.63 ms，配对两轮。点入 00700 到主图呈现的 P50 为 116.17 ms，配对三轮。60 Hz 下列表 P95 为 14.02 ms，超预算比例 2.54%。首页 P95 为 17.15 ms，分时和日 K 的 P95 分别为 45.91 和 51.39 ms。

暂停行情的 665 次提交中，Canvas 回调、准备和提交均为 0。日 K 静置只录制主图和页头行情标尺。首页记录 2950 次同输出文字复用。固定盒零根布局由自动化对照验证，详情页回放仍在行情更新和光标拖动帧进入根布局。

日 K 提交帧的响应式冲刷、布局和 Vulkan draw 的 P95 分别为 18.97、15.71、7.36 ms。Vulkan acquire 和 present 的 P95 分别为 0.28、1.06 ms。完整呈现策略继续以这些分段数据与 SurfaceFlinger 记录为依据。

#### 详情页纵向滚动基线

Pixel 9 的 120 Hz 三轮回放按滚动或回弹更新了绘制的帧计算。两组实际呈现间隔的 P50 都约为 8.34 ms。

| 行情 | 滚动帧数 | 渲染线程帧耗时 P95 | 实际呈现间隔 P95 |
|---|---:|---:|---:|
| 开启 | 1340 | 14.44 ms | 16.73 ms |
| 暂停 | 1257 | 9.48 ms | 16.71 ms |

行情开启时，67 个伴随根布局的滚动帧 P95 为 30.96 ms。1255 个没有根布局和 Canvas 绘制的滚动帧 P95 为 8.30 ms。暂停行情时，18 个 Canvas 补画且未触发根布局的滚动帧 P95 为 20.40 ms。60 Hz 单轮回归中，行情开启和暂停各自的滚动帧耗时 P95 为 29.58 和 13.77 ms。

#### 数值槽和按区块订阅行情后的回放

同一台 Pixel 9 在 120 Hz 复跑三轮。暂停行情拖光标的 675 个提交帧主图重录为 0。三次根布局都发生在提示框边界输出变化时。日 K 静置的根布局由基线的 101 次降为 0，布局区间 P95 从 15.29 ms 降到 10.79 ms。

| 场景 | 渲染线程帧耗时 P95（基线 → 优化后） | 实际滚动呈现间隔 P95（基线 → 优化后） |
|---|---:|---:|
| 日 K 静置 | 53.07 → 47.65 ms | — |
| 暂停行情拖光标 | 15.44 → 14.41 ms | — |
| 行情开启，仅滚动帧 | 14.44 → 13.92 ms | 16.73 → 16.68 ms |
| 行情暂停，仅滚动帧 | 9.48 → 10.01 ms | 16.71 → 16.71 ms |

行情开启时，包含滚动前后行情帧的整段回放 P95 从 26.28 ms 降到 21.72 ms。实际滚动帧中伴随根布局的样本由 67 个降为 29 个。60 Hz 单轮的日 K 静置、行情开启滚动和暂停滚动的 P95 分别为 47.40、27.06 和 13.59 ms。

## 表面

`SurfaceChanged` 更新逻辑视口和缓冲区尺寸。缓冲区尺寸变化会请求重绘，由下一帧为新尺寸重建交换链。视口和 IME inset 变化沿用现有交换链。空 acquire 是 `AcquireMiss`，连续 3 次后丢弃交换链并在下一帧用同一窗口重建。重建暂时失败时，后续呈现按退避策略继续尝试。`SurfaceDestroyed` 使渲染线程丢掉 `Surface` 并应答，Android 主线程随后释放对应窗口引用。

交换链、后台纹理上传器和已上传纹理都持有 Vulkan 加载器。纹理先释放 GPU 资源，再释放对图形上下文的引用，加载器在最后一个持有者退出时关闭。GLES 的 EGL context、离屏 Pbuffer、窗口表面、绘制和纹理上传都归渲染线程，后台只解码图像。识别到设备丢失后，公共帧循环有限次重建设备，保留层树与排版上下文，丢弃旧 GPU 资源和迟到的上传结果。

## 资产

| 键 | macOS | Android |
|---|---|---|
| 字体 `Sans` | `/System/Library/Fonts/Supplemental/Arial.ttf` | `assets/fonts/Sans.ttf` |
| `asset:<name>` | 资产目录按 scale 桶找 png/jpg/webp，ImageIO | APK `assets/images/`，`AImageDecoder` |

`SansCJK` 是第二张启动脸。脚本 `registerFont` 最多再加 8 个名字。必选缺图这一帧不提交。可选缺图跳过该命令。`https:` 图片在 `io` 线程下载，正文最多 4 MiB。

## 生命周期

`session::Lifecycle` 包含 `Detached`、`Resumed`、`Inactive`、`Hidden` 和 `Paused`。`Hidden` 和 `Paused` 冻结指针，并给未结束的按下补 `Cancel`。进入 `Resumed` 或由冻结状态进入 `Inactive` 时，丢弃旧 vsync token 并登记新的回调。已有 vsync token 时，定时器 `Wake` 让该回调推进下一帧。

| | macOS | Android |
|---|---|---|
| 隐藏 | `applicationDidHide`、最小化 → `Hidden` | `onStop` → `Hidden`，`onPause` → `Inactive` |
| 恢复 | `didBecomeActive`、还原 → `Resumed`。遮挡恢复 → `Invalidate` | 启动脚本时为 `Inactive`。Activity 已恢复且窗口有焦点时为 `Resumed`，已恢复但窗口无焦点时为 `Inactive`。 |
| 内存 | `applicationDidReceiveMemoryWarning` | `onTrimMemory` / `onLowMemory` |

`Metrics` 的 inset 交给 `Tree::set_viewport`。macOS 的 inset 是 0。默认内容 360×640。

## 还没有

原生控件与 raster 任意交错、硬件缓冲导入与 fence、单个 JS 运行时的多 View、轨迹重采样和命令级显示列表 diff。
