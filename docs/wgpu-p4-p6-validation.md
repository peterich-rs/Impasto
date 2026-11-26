# 推进 P4–P6 渲染优化并验证 Gallery

基线是 P3 的 `e042ae49`。本文记录 P4–P6 的实现范围与测量口径。阶段目标见 [wgpu 渲染演进](wgpu-rendering-evolution.md) 和 [遥测与 P1 计划](telemetry-and-p1-plan.md)。

## 可行性和接口边界

`engine::scene::LayerTree` 仍是 UI 到 Raster 的交换格式。引擎在记录时发布滚动边界和可链接的合成层。Raster 持有 Pass、图集、快照和损伤历史。宿主消费 `Vsync::request_frame_with_rate` 的刷新率提示，平台可以按当前显示模式调整。wgpu 30.0.2 的公开 Surface API 提供可渲染纹理和 `Queue::present`，没有请求 buffer age 或传递 present damage 的字段。可保留的工作纹理支持局部光栅化，最终呈现仍需要整面复制。磁盘管线数据由本版本 Vulkan 后端提供，宿主须提供应用私有目录。 [wgpu Surface](https://docs.rs/wgpu/30.0.2/wgpu/struct.Surface.html) · [wgpu Queue](https://docs.rs/wgpu/30.0.2/wgpu/struct.Queue.html)

## 最新实现与剩余验收

| 工作项 | 已实现与验收边界 |
| --- | --- |
| P4.1 Pass 与遮挡 | 独立离屏任务前置。新增有界兄弟层扫描，可剔除被不透明尾层安全区域盖住的子树，碰到目标读取时保守回放。支持均匀圆角实色矩形的深度遮挡，内切区写深度，抗锯齿边缘按原画序混合。任意依赖图的调度仍待实现。 |
| P4.2 文字 | 已编译字形记录代次化图集槽，变体匹配时直接索引。单个密集文字 Picture 也可用 Swash 工作线程准备缺失字形。大批冷字号仍需光栅化及上传。 |
| P4.3 PSO | 实际表面格式预热、WGSL 构建期校验、Vulkan 持久化和后台效果预热已接入。Gallery 全流程同步冷编译归零尚未达成。原生 PSO AOT 和 DX12 磁盘缓存需要额外的后端支持。 [wgpu 管线缓存](https://docs.rs/wgpu/30.0.2/wgpu/struct.Features.html#associatedconstant.PIPELINE_CACHE) |
| P4.4 刷新率 | Pixel 9 记录到低速惯性的 60 Hz 调度和慢速透明度动画的 30 Hz 调度。iPhone 真机 ProMotion、屏幕实际刷新率与功耗仍待测。 |
| P4.5 画质 | 渐变抖动直接用 sRGB 曲线局部斜率计算线性扰动。小字号文字与渐变的跨端原生观感仍待校准。 |
| P5.1 VirtualList | Raster 在预计越出挂载区前 32 ms 请求 UI 更新，UI 用同一预测扩展挂载范围。吸顶标题容器按视口宽度和已测行高布局，挂载前保留原行。并行换窗、资源预取、通用 ScrollDrivenLink 与吸附和回弹的端到端验收仍待完成。 |
| P5.2 局部重绘 | Field 光标单独录制 Picture。保留工作纹理与损伤 scissor 已支持安全场景，最终上屏走整面复制。系统增量呈现与 Swapchain buffer age 需要平台后端支持，Gallery 光标的填充量和 GPU 时间尚待量化。 |
| P5.3 转场 | Router、Sheet 和 Dialog 显式标记可快照的动画。标记节点在透明度为零或变换为单位值时仍生成相应轨道。快照支持矩形裁剪、嵌套离屏组及随后移入屏幕的内容。Sheet 和 Dialog 的内容节点随开合重新挂载到对应父节点，内部更新继续写入同一容器，合成期间可以停止 UI 帧。Gallery 含实测场景，复杂页面的覆盖率和 <100 µs GPU 目标仍待验证。 |
| P5.4 图片 | 小图 atlas 复用已上传的前三级 mip，按独立 texel 边界采样。单 mip 图片可进入同一 atlas。ASTC、ETC2、BC7 素材链路及 GLES3 图墙真机量化仍待完成。 |
| P6.1 矢量 | 符合条件、无 Canvas 路径裁剪且展开后不超过 255 个扇形三角形的复杂填充，通过 GPU 模板计算 NonZero 或 EvenOdd 覆盖。曲线展开在 CPU，含路径裁剪的填充和超限路径使用 Lyon。通用 GPU 曲线、MSDF 图标和主目标 MSAA 合批仍待完成。 |
| P6.2 材质 | 子树 ColorFilter 在合成时应用直通线性 RGBA 矩阵。Vibrancy 在多级模糊回贴中融合饱和度、提亮、Tint 和抖动。渐变效果混合清晰与模糊两路图像，连续变半径核仍待实现。HighlightBorder 使用合成时钟。BlurHash 和 ThumbHash 在 GPU 重建后进入现有图片缓存，ThumbHash 保留宽高比和透明度。 |
| P6.3 输入 | 合成线程直通拖拽与输入预测走独立排期。 |

## 补充测量

Apple M4 Pro Metal、Release、Headless。同机交替跑三组 A/B，表内为各构建三轮分位数的中位数，单位为 µs。

| 场景 | CPU 编码 p50 | CPU 编码 p95 | GPU p95 |
| --- | ---: | ---: | ---: |
| 密集文字墙，P3 → 最新 | 57.50 → 76.25 | 2960.04 → 1297.29 | 226.25 → 227.54 |
| 滚动 1,000 张卡片，P3 → 最新 | 152.71 → 152.50 | 188.04 → 177.00 | 515.83 → 537.33 |
| 滚动 10,000 张卡片，P3 → 最新 | 203.08 → 200.08 | 252.83 → 257.92 | 512.92 → 534.33 |
| 光标基准，保留纹理关闭 → 打开 | 192.54 → 28.25 | 247.29 → 39.79 | 253.92 → 80.83 |

密集文字的 p95 降约 56%，仍高于 400 µs。该样本循环使用八个字号，冷字号的准备和上传进入 p95。仅复用文字画面时，跳过栅格化前的整场扫描，避免拖慢滚动。在 900×1,800、3× DPR 的合成中文滚动样本中，24 帧内上传并保留 2,048 个字形，图集回收和淘汰均为零。

两组卡片滚动的 GPU p95 仍比 P3 高约 4.2%。同一候选的抖动开关 A/B 也记录到 GPU 时间差异。默认渐变抖动随线性通道亮度淡出，暗部继续使用半码值扰动。

光标基准的 79 个稳态帧全部复用保留纹理，GPU p95 降 68.2%。两种配置都需把 485,100 像素整面转码上屏。这个样本用 Bgra8Unorm Headless 目标，Gallery 和实际交换链仍需各自测量。

Pixel 9 Vulkan、Release、Split 拓扑的补测各采集一条 6 秒 Trace。UI 帧只统计首尾合成帧之间的事件。

| 场景 | 合成帧 / UI 帧 | Raster 墙钟 p95（ms） | 帧间隔 p50 / p95（ms） |
| --- | ---: | ---: | ---: |
| 虚拟列表，1,000 行 | 241 / 7 | 8.86 | 8.40 / 16.75 |
| 普通惯性滚动 | 221 / 0 | 8.88 | 8.36 / 16.66 |
| 慢速透明度 | 163 / 0 | 5.27 | 32.79 / 33.52 |
| 合成材质 | 580 / 0 | 4.95 | 8.32 / 8.76 |

虚拟列表合成帧占 97.2%。滚过分组边界后，Android 截图确认第 2 组吸顶标题铺满视口。材质页已在 Android Vulkan 和 macOS Metal 核对画面。上一版捕获还记录到慢速透明度的 `Hz30` 和惯性尾段的 `Hz60`。首轮配对中的第一组滚动 Trace 显示，P3 → 候选的 `surface_acquire` p95 由 0.40 ms 升至 5.65 ms，`encode` p95 由 1.14 ms 降至 1.02 ms。以上是应用帧节奏和 CPU 等待数据。

同一台 Pixel 9 另对路由 Push、全宽 Sheet 和居中 Dialog 各冷启动一次并开合一次（采样时的实验页已移除，现在在 Gallery 的详情页与下单面板上复测）。截图确认目标页面、全宽 Sheet 和居中 Dialog 均正常呈现。

| 场景 | 合成帧 / UI 帧 | 快照烘焙次数 | 快照命中次数 |
| --- | ---: | ---: | ---: |
| Router Push / Pop | 134 / 10 | 11 | 192 |
| Sheet 开合 | 91 / 4 | 1 | 52 |
| Dialog 开合 | 102 / 3 | 2 | 101 |

每条 Trace 采集 5 秒，`transition_snapshot` 的 `bakes` 与 `hits` 按图层计数，一帧可包含多个命中。可用 `tools/android-frame-trace.sh OUTPUT dev.impasto.host.profile 5` 采集，随后通过 `adb shell am start -S -W -a android.intent.action.VIEW -d <URL> -n dev.impasto.host.profile/dev.impasto.host.ImpastoActivity` 启动。此数据验证快照可复用，Router 仍存在重复烘焙，转场 GPU 耗时另需 GPU 时间戳。

复现命令如下。测量前使用独立的 P3 checkout，关闭其他构建任务。

```sh
python3 tools/bench-compare.py <P3 checkout> . --backend metal --rounds 3 --filter tests::p0::bench_scroll_ --out metal-cards.json
python3 tools/bench-compare.py <P3 checkout> . --backend metal --rounds 3 --filter tests::p0::bench_dense_text_wall --out metal-text.json
python3 tools/bench-compare.py . . --backend metal --rounds 3 --filter bench_p3_retained_offscreen_cursor --base-env IMPASTO_RETAIN_OFFSCREEN=0 --head-env IMPASTO_RETAIN_OFFSCREEN=1 --out metal-cursor.json
IMPASTO_TEST_GPU=1 cargo test --locked --release -p raster --lib bench_cjk_atlas_scroll_3x -- --ignored --nocapture
```

本机原始数据位于 `/tmp/impasto-p4/` 的 `remaining-metal-*.json` 和 `remaining-pixel-final/`。内联图像通过普通 Image 的 `source` 接收 `blurhash:<Base83>` 或 `thumbhash:<Base64>`，重建结果与常规图像共用缓存。 [BlurHash 格式](https://github.com/woltapp/blurhash/blob/master/Algorithm.md) · [ThumbHash 格式](https://evanw.github.io/thumbhash/)

## 最新验证

- `just check`、`cargo fmt --all -- --check`、`git diff --check` 和原生模块声明一致性检查通过。
- Metal Release 跑过 77 个 Raster 用例。关闭绑定数组时，小图 atlas 用一次绘制完成两个图源的 mip 采样并通过像素对比。快照用例验证了离屏内容移入后像素正确，ThumbHash 用例与参考解码比较透明度、颜色和宽高比。另分别强制 Early-Z 运行圆角用例，和运行 3× DPR 中文基准。
- macOS 宿主 Clippy 和 20 个宿主测试通过。Gallery 的材质页与虚拟列表有 Metal 首帧截图。
- Android Debug、instrumentation 构建、lint 和两个 ABI 的 APK 对齐检查通过。Pixel 9 完成 Release Gallery 场景采样和 25 项 arm64 Debug instrumentation 测试。
- iPhone Air 模拟器通过 Debug 首帧和进程检查。ProMotion 和功耗需要物理设备数据。

## 首轮 Pixel 9 Gallery

Pixel 9、Android 17、Mali-G715 Vulkan、1080×2424、屏幕最高 120 Hz。使用 Release、Split 拓扑。滚动和变换由 Gallery 深链在 500 ms 后自行执行，材质流光在打开时开始。P3 使用留存的 profile APK，候选用本轮 profile APK。普通滚动和变换各录制三组交替顺序的 A/B。虚拟列表和材质都是本轮新增用例，各记录一条候选 Trace。

| 场景 | 构建 | 合成帧 / UI 帧 | Raster p50 / p95（ms） | 合成帧间隔 p50 / p95（ms） |
| --- | --- | ---: | ---: | ---: |
| 普通滚动 | P3 | 259 / 0 | 3.34 / 4.17 | 8.31 / 8.85 |
| 普通滚动 | 候选 | 219 / 0 | 3.15 / 8.77 | 8.35 / 16.70 |
| 变换 | P3 | 197 / 2 | 3.40 / 4.05 | 8.32 / 8.72 |
| 变换 | 候选 | 196 / 2 | 3.28 / 3.98 | 8.32 / 8.66 |
| 虚拟列表，1,000 行 | 候选 | 238 / 7 | 3.55 / 4.69 | 8.43 / 16.72 |
| 材质流光 | 候选 | 676 / 0 | 3.59 / 4.29 | 8.32 / 8.69 |

表中共用场景为各构建三次结果的中位数。普通滚动三次都在合成采样区间内记录到 0 个 UI 帧。Android 在惯性开始后约 1.49–1.52 秒进入 60 Hz 调度，候选帧间隔 p95 进入约 16.7 ms 档，Raster 墙钟 p95 也上升。降频前，三轮候选的 Raster p95 为 3.76–4.07 ms。变换与材质流光的帧间隔中位数保持约 8.32 ms。

虚拟列表样本在合成区间内的合成帧占比为 `238 ÷ (238 + 7) = 97.1%`。这个比率来自帧事件计数。Sticky Header 使用百分比宽度时，冷启动样本仍可能按内容宽度绘制，吸顶标题的布局尚未通过完整画面验收。

采样单位是 Perfetto 中 `compositor_frame` 的墙钟耗时。帧间隔取相邻合成帧的起点，包含 UI 接管和平台节奏转换的间隔。UI 帧计数取首尾合成帧之间的 `Impasto frame`。表中数据描述应用的 CPU 帧流水线，SurfaceFlinger 的实际呈现间隔、GPU 耗时与耗电需要各自的测量口径。

用 `tools/android-frame-trace.sh OUTPUT dev.impasto.host.profile 6` 采集。采样时启动应用使用 `adb shell am start -S -W`，动作是 `android.intent.action.VIEW`，地址为当时实验页的深链；该页已移除，惯性滚动与虚拟列表场景改为在 Gallery 自选列表上手动拖动复测。共用场景按 `base-scroll-1.perfetto-trace`、`head-transform-1.perfetto-trace` 等方式命名，单次场景使用 `head-virtual-list.perfetto-trace` 和 `head-materials.perfetto-trace`。安装 Python `perfetto` 后用 `python3 tools/compositor-trace-summary.py <trace directory>` 生成汇总。

本机保留了 14 条 Perfetto Trace、APK 指纹和汇总，位于 `/tmp/impasto-p4/final-gallery/`。Metal 原始样本位于 `/tmp/impasto-p4/metal-verified.json` 和 `/tmp/impasto-p4/metal-p3-scenarios.json`。

## 首轮 Apple M4 Pro Metal

使用同一台机器、Release、Headless Metal，运行 `python3 tools/bench-compare.py <P3 checkout> . --backend metal --rounds 3 --out metal.json`。表中为各构建三轮分位数的中位数，单位为 µs。GPU 数值取帧时间戳包围区间，编码数值不含 GPU 等待。

| 场景 | CPU 编码 p50，P3 → 候选 | CPU 编码 p95，P3 → 候选 | GPU p50，P3 → 候选 | GPU p95，P3 → 候选 |
| --- | ---: | ---: | ---: | ---: |
| 动态毛玻璃覆盖层 | 199.88 → 178.33 | 284.58 → 239.25 | 643.71 → 608.21 | 738.08 → 646.29 |
| Canvas 500 条路径 | 323.21 → 219.33 | 418.21 → 262.04 | 898.71 → 595.79 | 968.13 → 662.00 |
| 密集文字墙 | 64.29 → 52.75 | 3284.50 → 3028.63 | 202.38 → 198.29 | 225.92 → 225.17 |
| 滚动 1,000 张卡片 | 168.46 → 167.96 | 195.63 → 188.46 | 510.79 → 545.58 | 524.38 → 564.00 |
| 滚动 10,000 张卡片 | 228.63 → 228.88 | 291.63 → 299.50 | 514.38 → 548.96 | 525.75 → 562.75 |

Canvas 500 路径的总 Pass 为 28 → 15，离屏 Pass 均为 14，主目标 Pass 为 14 → 1。独立五透明组的像素回归也验证了一次主目标 Pass。远离改动区的固定毛玻璃可复用上次模糊结果，动态毛玻璃的取样区每帧变化时仍复用临时纹理池。密集文字的编码 p95 为 3.03 ms，尚未达到 0.4 ms。两个卡片滚动样本的 GPU p95 增加约 7%。

同样运行 `--filter tests::p0::p3::bench_` 补测了三组既有场景。Canvas 路由的 GPU p95 为 770.67 → 612.04 µs。缓存模糊滚动的编码 p95 为 28.04 → 205.75 µs，轮间波动较大。P3 光标基准使用 Headless 目标，默认不会启用窗口的保留工作纹理，该场景的 GPU p95 为 251.25 → 248.58 µs。局部渲染的正确性单独由保留纹理的像素回归覆盖。

## 首轮本地验证

| 检查 | 结果 |
| --- | --- |
| `just check`、`cargo fmt --all -- --check`、`git diff --check` | 通过 |
| 原生模块声明与类型一致性检查 | 通过 |
| `IMPASTO_TEST_GPU=1 cargo test --locked -p raster --lib` | Metal，68 通过，12 个基准或诊断用例默认忽略 |
| `cargo clippy --locked -p embedder-macos --all-targets -- -D warnings`、`cargo test -p embedder-macos` | 通过，20 个宿主测试 |
| `just android-build-check` | Debug APK 构建、lint 及 arm64/x86_64 对齐检查通过 |
| `just android-device-test` | Pixel 9 实机，25 个 instrumentation 测试 |
| `just ios-sim-test --debug` | iPhone Air 模拟器，可见首帧与进程检查通过 |
| Gallery 画面检查 | macOS Metal 与 Pixel 9 Vulkan 的材质页、Pixel 9 的虚拟列表 |

并行构建期间，股票详情引擎测试遇到一次 JS 执行预算中断。单测单独重跑和 `RUST_TEST_THREADS=4 just check` 均通过。

未采集 DRAM 带宽、耗电、Touch-to-Photon 或原生 120 Hz iPhone 真机数据。上述数据用于检查本轮能独立复验的性能变化，尚不足以关闭 P4–P6 的全部验收条目。
