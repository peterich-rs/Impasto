# 平台宿主

macOS 与 Android 的平台线程将事件入队，渲染线程创建 `Engine` 和会话并上屏。两个宿主各自链接对应目标的 Impeller SDK。[Impeller SDK](../crates/impeller-sys/README.md)

`shell` 通过 `session` 使用帧和输入类型。平台宿主启用 `javascript/ui` 打开应用会话。`protocol`、`uicore`、`engine`、`crates/macos` 不手写 C。QuickJS 使用 Cargo 构建的 quickjs-jit 0.14.2，Rust 绑定在 `crates/javascript/src/ui/player/vm`。`Engine` 与 QuickJS 都是 `!Send`。

## 分层

```text
quotes.js
  → JavaScript UI 会话（渲染线程打开）
  → session::Session
  → shell（队列、帧调度、submit）
  → macos / android
  → engine::Engine
```

宿主实现 `Services`、`Surface`、`Vsync`，并交给 `shell` 一个 `Send` 闭包，在渲染线程上打开会话。`Engine` 的所有权留在 `shell`。`Surface` 的公开接口不出现 `CAMetalLayer` 或 `VulkanSwapchain`。

## 启动

平台线程准备脚本、字体、图片和窗口，然后回到事件循环。渲染线程创建 `Engine`、`register_font`、调用会话闭包。平台线程不在启动函数里等待表面。

启动 URL 在 `shell::start` 之前写入 Shared `launch_url`。macOS 来自 argv `--url`，Android 来自显式 Intent 的 `getData()`。当前宿主没有注册 URL scheme。

退出码：`0` 正常，`2` 引擎不可用，`3` 解释器 `Runtime`，`4` 启动期脚本失败。运行期脚本错误只记日志。

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

`shell` 只看 `wants_frame`，不读树。会话在这一位里并上未布局、未绘制、未排空的 job，以及还没结束的运动。`FrameScheduler` 在提交或呈现前即可登记会话请求的下一次 vsync，回调入队后由渲染线程串行处理。失败进入退避时作废这次登记。Android 的 `VsyncWaiter` 用当前显示器的刷新率计算帧预算，呈现时间取自 `Choreographer` 的首选 timeline（API 33 及以上）。脚本错误清掉这一次的需求。

`Hidden`、`Paused`、`MemoryWarning` 调用 `Engine::trim` 进入 `Flush`。连续 `PresentFailed`、`NotReady` 或 `AcquireMiss` 从第 3 次起退避，16ms 起翻倍，上限 1000ms。`BadScene` 不计入。同一表面打开 `NotReady` 最多再试 120 次。

### Android 帧性能

Gallery 的 release 构建可通过 `-PimpastoProfile=true` 生成可由 adb 采样的 `dev.impasto.host.profile`。它有独立的应用数据。回放使用竖屏，性能面板关闭，详情固定为 00700。脚本采集冷启动、详情入场、首页行情、分时、日 K、行情开关各一组的光标拖动和详情页纵向滚动，以及列表纵向拖动。

```sh
just gallery-dist
crates/android/apk/gradlew -p crates/android/apk -PimpastoProfile=true assembleRelease
adb install -r crates/android/apk/app/build/outputs/apk/release/app-release.apk
adb shell am start -W -n dev.impasto.host.profile/dev.impasto.host.HostActivity
FRAME_REFRESH=120 tools/android-frame-replay.sh target/android-perf/120 3
FRAME_REFRESH=60 tools/android-frame-replay.sh target/android-perf/60 1
python3 tools/android-frame-report.py target/android-perf/120 --hz 120
python3 tools/android-frame-report.py target/android-perf/60 --hz 60
```

报告脚本使用 Python 的 `perfetto` 包。安装后等 Gallery 首页显示再开始采集。回放结束恢复原刷新率设置。单条手动采集可用 `tools/android-frame-trace.sh OUTPUT`。只查详情页时，使用 `FRAME_SUITE=detail FRAME_REFRESH=120 tools/android-frame-replay.sh target/android-perf/detail 3 7`。该回放包含日 K 静置、暂停行情拖光标、行情开关各一组纵向滚动。纵向手势从盘口前开始，滑过逐笔和深度区块，再滑回图表。

用 `FRAME_SUITE=daily` 只跑日 K 静置。用 `pnpm -C apps/gallery-solid run build --freeze-stock=book` 或 `--freeze-stock=chart` 生成冻结对应区块行情输入的 bundle，再打包并运行同一回放。冻结主图时仍可切换周期。完成对照后，用无参数的 `pnpm -C apps/gallery-solid run build` 恢复实时行情的 bundle。

`Impasto frame` 覆盖渲染线程处理一帧的完整区间。报告按发起呈现的帧计算 P50、P95、峰值和超预算比例，另计跳过呈现与新层树提交次数。`Impasto work` 记录根布局、局部布局、同输出文字、输入、响应式更新和 Canvas 次数；`Impasto Canvas commit` 记录节点、内容盒高度和绘制命令数。Pixel 9 竖屏详情页的主图高度是 204 点。Vulkan 的 acquire、draw 和 present 各有独立区间。

SurfaceFlinger 的 `--latency` 记录提供实际呈现时间。报告将对应的 `queueBuffer` 与送达应用的触摸事件配对，保留配对样本数。首屏统计从启动到首页首个走势 Canvas 实际呈现，详情入场统计从点击到详情主图实际呈现。提交帧 CPU 耗时、实际呈现间隔与输入到呈现延迟分别汇报。

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
