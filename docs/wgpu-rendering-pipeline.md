# 光栅管线与宿主调度

Impasto 的 UI 队列运行脚本、布局和绘制录制。Raster 队列读取已提交的 `LayerTree`，复用按 `PictureId` 和 generation 安装的 `CompiledPicture`，并向窗口呈现。UI 和 Raster 共享窗口字体库。WGSL 在构建时由 Naga 校验，图形设备按平台使用 Metal、Vulkan、GLES 或 DX12。 [场景提交](../crates/raster/src/submission.rs) · [绘制入口](../crates/raster/src/compositor/output.rs)

## 一帧如何上屏

1. **Prepare**。图片由 I/O 队列解码，Raster 安装上传结果。字形准备依次检查可见 Layer、Picture、Scope 分块和字形边界。Swash 以物理字号和亚像素位置生成灰度或彩色位图。宿主预算为稳态 2 ms、冷首帧 5 ms，超出预算的缺字交给 Rayon，资源到齐后重试。已有窗口画面在等待期间保持显示。 [字形准备](../crates/raster/src/compositor/glyph_prepare.rs) · [图集](../crates/raster/src/atlas.rs)
2. **Collect**。按绘制顺序跳过裁剪外的内容，将顺序绘制写进 `DrawList`。命令用整数索引引用 Pipeline、绑定组、裁剪和网格，并合并状态相同且缓冲连续的相邻实例。Canvas 使用编译时生成的 Lyon 网格，适合直接绘制的凸路径带抗锯齿边缘。复杂填充及遮罩按需使用 MSAA 解析。 [绘制命令](../crates/raster/src/compositor/draw_list.rs) · [Canvas](../crates/raster/src/compositor/canvas.rs)
3. **Encode**。常规 sRGB 窗口使用一个主 RenderPass。模糊、混合、透明度组、遮罩及色彩转换按需借用 `AttachmentPool`。离屏输出在采样命令编码完成后可以复用。需要读取先前背景的效果先结束当前工作目标的绘制，再采样该目标，最后将工作目标输出到窗口。晚采集在写入 drawable 前完成已知离屏工作。 [编码与纹理生命周期](../crates/raster/src/compositor/passes.rs)
4. **Present**。Raster 提交资源上传与绘制。UI 的双帧流水线跟踪在途提交，并在 Raster 忙时保留最新待交付场景，合并重载和场景更新信息。 [帧流水线](../crates/embedder/common/src/host/ui_frame.rs)

## 冷启动与交接

Android、iOS 和 macOS 在平台线程准备窗口、输入及显示回调。入口求值在 `ui-boot` 上运行，Raster 在独立队列上初始化。UI 提交首个 LayerTree 后，在当前任务边界交出会话，保持逻辑 UI 队列的标识及 FIFO 顺序。定时任务以原绝对截止时间重新投递，原任务句柄继续支持取消。QuickJS 移交前卸载钩子，在平台线程刷新 C 栈顶并重新安装钩子，随后采用平台脚本预算。 [队列](../crates/embedder/common/src/task_runner.rs) · [宿主交接](../crates/embedder/common/src/host.rs) · [QuickJS](../crates/javascript/src/runtime/quickjs/vm.rs)

稳态输入在 UI 执行本次帧前交付。UI 与 Raster 的显示请求各自持有 FrameToken，平台只取消匹配的请求。符合条件的滚动和透明度轨道由 Raster 采样，并通过原生显示时钟续帧。显示时钟的交付仍由各平台管理。 [合成轨道](scroll-compositor.md)

## 回归测量

用同一台设备、相同刷新率、同一构建配置和同一回放对比前后版本。记录提交、设备、系统版本、刷新率、构建配置和原始 Trace。Android 使用 `tools/android-frame-trace.sh OUTPUT dev.impasto.gallery 6`，Apple 平台使用 Instruments 记录原生显示和渲染线程。

| 场景 | 记录指标 |
| --- | --- |
| 冷启动 | 点击图标至首帧完成呈现，另记录 UI 交接时间 |
| 120 Hz 长列表甩动 | 帧时 P50、P95、P99、掉帧率、UI 与 Raster CPU 耗时 |
| Canvas 密集交互 | 帧时 P95、Raster 编码耗时和 MSAA Pass 数 |
| 毛玻璃与透明度弹窗 | 离屏纹理峰值、主 Pass 数与 Raster 编码耗时 |
| 快速触摸和输入 | 输入到首次可见帧的时延、帧背压及 vsync 取消事件 |

可运行 `cargo test -p raster` 检查绘制语义和像素回归，`just host` 检查 macOS 首帧。渲染诊断仅使用 `IMPASTO_PREWARM`、`IMPASTO_LATE_ACQUIRE`、`IMPASTO_PARALLEL_PREPARE`、`IMPASTO_GPU_VALIDATE` 和 Surface 诊断组 `IMPASTO_SURFACE_COLOR`、`IMPASTO_SURFACE_LATENCY`、`IMPASTO_PRESENT_MODE`。 [依赖与开关检查](../tools/check-javascript-boundaries.py)

### Pixel 9 调试构建采样

Pixel 9 在 120 Hz 下分别回放一次改造前后的调试包。两侧验证壳使用相同的 120 行列表脚本。Gallery 的应用脚本有差异，走势图结果包含应用与引擎的共同变化。 [回放脚本](../tools/android-frame-replay.sh) · [结果统计](../tools/android-frame-report.py)

| 指标 | 改造前 | 改造后 |
| --- | ---: | ---: |
| 验证壳启动标记至首帧呈现 | 532.25 ms | 489.91 ms |
| 验证壳列表提交帧耗时 P95 | 4.54 ms | 4.13 ms |
| 验证壳列表输入到呈现 P50 | 29.49 ms | 25.52 ms |
| 验证壳列表输入到呈现 P95 | 31.94 ms | 38.54 ms |
| Gallery 实时十字线 Canvas 绘制 CPU 耗时 P95 | 1.69 ms | 0.97 ms |
| Gallery 实时十字线输入到呈现 P95 | 74.84 ms | 41.63 ms |

改造后的验证壳启动 Trace 中，首个 UI 帧由 `ui-boot` 执行，后续 86 个 UI 帧在 Android 主线程执行，87 个 Raster 帧均在独立 Raster 线程执行。单轮调试构建用于验证链路，性能验收仍需多轮发布构建采样。
