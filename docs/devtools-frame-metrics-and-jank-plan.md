# DevTools 帧指标与卡顿判定改造方案（Hitch / 流水线单级瓶颈 / 触发源与 GPU 回填）

> **适用范围与前置约束**
> - **适用范围**：[crates/embedder/common](../crates/embedder/common/src/diagnostics/telemetry.rs)、[crates/devtools-protocol](../crates/devtools-protocol/src/frame.rs)、[examples/devtools-desktop](../examples/devtools-desktop/rust/src/store/frames.rs) 与 [packages/devtools](../packages/devtools/src/pages/performance.tsx)。
> - **关联文档**：[性能基线与遥测](telemetry-and-p1-plan.md) · [光栅管线与宿主调度](wgpu-rendering-pipeline.md) · [DevTools Desktop Rust Core](devtools-desktop-rust-core.md) · [DevTools Desktop UI 与图表](devtools-desktop-ui-and-charts.md)。
> - **兼容性前提**：**不需要保留对旧版 `.impasto-session` 录制文件或旧版 Agent 的向后兼容**。协议结构体可直接演进并清理历史兼容分支。
> - **架构与规范红线**：
>   - 严格遵守 `just boundaries`（不改动 `crates/engine` 依赖边界，客户端不反向依赖 `engine` / `raster` / `embedder*` / QuickJS）。
>   - Rust 代码遵守 workspace lints（生产代码 `unsafe_code` / `unwrap_used` / `expect_used` / `unused_qualifications` 均为 `deny`）。
>   - 修改导出类型后必须通过 `impasto sync` 生成 `.impasto/types/`，严禁手改生成产物。

---

## 1. 改造前的问题根因与诊断结论

### 1.1 按需渲染下 `FPS` 指标的语义失真

Impasto 是按需渲染引擎（静止时不调度 VSync），当前桌面端 DevTools 将窗口提交频率直接标作 `FPS`，带来三类失真：

1. **低频定时刷新被误读为卡顿**：Gallery 示例在 [quotes-provider.tsx](../examples/gallery-solid/src/state/quotes-provider.tsx#L4-L56) 中以 `PUBLICATION_MS = 200` 推进行情脉冲，每 200ms 唤醒 UI 绘制 1 帧。在 [frames.rs](../examples/devtools-desktop/rust/src/store/frames.rs#L9-L103) 中，`LIVE_WINDOW_MS = 2_000.0`、`IDLE_AFTER_MS = 500.0`；由于 `200ms < 500ms`，`is_idle` 永不触发，公式 `(presented - 1) * 1_000 / (end - start)` 稳定算出 `5`，并在 [OverviewPage.tsx](../examples/devtools-desktop/src/pages/OverviewPage.tsx#L47-L48) 与 [Workbench.tsx](../examples/devtools-desktop/src/shell/Workbench.tsx#L31-L34) 显示为 `FPS: 5` / `5 fps`。这把健康的按需提交速率与掉帧卡死混为一谈。
2. **2 秒滑动窗口动静混合稀释**：若 2 秒窗口内前 1.5s 为 200ms 定时刷新（约 7 帧）、后 0.5s 为满帧 60Hz 滚动（30 帧），尽管滚动零掉帧，公式仍会给出 `~18 fps`，无法反映真实交互流畅度。
3. **缺少严重度量化**：原有 `jank_count` 仅做布尔计数（`over_budget || missed_vsync || skip_reason.is_some()`），超预算 1ms（掉 1 帧）与卡死 180ms（掉 10+ 帧）均只记 `+1`。

### 1.2 双帧交错流水线下的串行误判与 `FrameStrip` 堆叠失真

当前宿主采用 **UI + Raster 独立 `TaskRunner` 并发流水线**：
- [host.rs](../crates/embedder/common/src/host.rs#L1099-L1106) 定义 `FramePipeline::DEPTH = 2`，Raster 处理第 $N$ 帧时，UI 可交错准备第 $N+1$ 帧；
- [surface.rs](../crates/raster/src/surface.rs#L449-L454) 默认 `desired_maximum_frame_latency = 2`。

```text
60Hz（预算 16.67ms）双线程交错流水线示意：
VSync #1 (0ms)          VSync #2 (16.67ms)       VSync #3 (33.33ms)
|-- UI(F1): 10ms --|    |-- UI(F2): 10ms --|     |-- UI(F3): 10ms --|
                   |-- Raster(F1): 10ms --|      |-- Raster(F2): 10ms --|
                                          |-- GPU(F1): 8ms --|
```

在此模型下，UI 每帧 10ms、Raster 每帧 10ms、GPU 每帧 8ms，**稳态可满帧 60Hz、0 掉帧**（仅首帧到上屏延迟增加 1 帧，这是双帧流水线以延迟换吞吐的刻意设计）。但现行实现存在两处架构级偏差：

1. **用端到端延迟判定单帧吞吐超预算**：[telemetry.rs](../crates/embedder/common/src/diagnostics/telemetry.rs#L62-L67) 定义 `e2e_ms` 为 `present_done_ns - vsync_ns`（包含 `ui_ms + queue_ms + raster_ms = 20ms`），而 [raster_host/frame.rs](../crates/embedder/common/src/host/raster_host/frame.rs#L268-L272) 与 [timing.rs](../crates/embedder/common/src/timing.rs#L45-L51) 直接用该 `e2e_ms` 对比单帧 `budget_ns`（16.67ms）得到 `missed_vsync` / `over_budget`，导致健康流水线帧帧被判超预算；连续交错时 `queue_ms` 还包含等待前一帧 Raster 完成的线程排队时间。
2. **`frame_strip` 垂直硬叠全部阶段**：[projection.rs](../examples/devtools-desktop/rust/src/projection.rs#L196-L234) 把 `Script + Layout + Record + Queue + Raster + GPU` 全部垂直堆在同一根柱子上对比 `budget_ms` 红线，在视觉上将上述稳态帧画到 `28ms`（远超红线）。

### 1.3 现象复盘：为何高速滚动时全在红线下方，停手静止后反而连续超红线

结合实测 Frame Strip，这一看似反直觉的现象由以下三点共同造成：

1. **横轴为帧序号而非时间轴，样本构成完全切换**：高速滚动时 90%+ 的帧只改滚动偏移，走 [submit_layer_delta](../crates/embedder/common/src/host/raster_host/frame.rs#L62-L80) 增量更新或 [CompositorMode](../crates/embedder/common/src/host/compositor_animation.rs) 托管路径（几乎零 Script、不重建 Picture、不重绘 Canvas）；停手后按需渲染停止生成轻量帧，留在 Frame Strip 右侧连续排列的 **100% 是每 200ms 触发一次的重量级行情刷新帧**（列表可见 [StockRow / Spark Canvas](../examples/gallery-solid/src/pages/stocks.tsx#L118-L202) 与宽屏右侧 [StockView](../examples/gallery-solid/src/pages/stock.tsx#L131-L199) 同步重绘）。
2. **低频唤醒引发系统 DVFS 降频**：持续滚动期间系统将 CPU 维持在性能核高频、GPU 维持高频率与热缓存；停手切换为 5Hz（每 200ms 唤醒 1 次）后，操作系统降频至节能核与低频档位，同一行情刷新帧的 `Script` / `Record` / `Raster` 墙钟耗时同步放大 2–3 倍，在 120Hz（`8.33ms`）预算下极易越线。
3. **`e2e_ms` 串联判错 + 全阶段堆叠展示**：把跨线程流水线累加耗时拿去撞单帧红线，使问题帧与正常帧同时在图上显著高出红线。

### 1.4 行业对标原则

- **Flutter DevTools（流水线 Jank 归因）**：UI 线程与 Raster 线程独立对比 `budget_ms`；`UI > budget` 记 UI Jank、`Raster > budget` 记 Raster Jank；柱状图将 UI 与 Raster 分柱展示，端到端 `e2e_ms`（`totalSpan`）单独作为延迟指标。
- **Apple Instruments（Hitch Time Ratio）**：淡化全局平均 FPS，改以 **`ms/s`（每秒卡顿毫秒数）** 衡量用户可感知的不流畅时间（`< 5 ms/s` 流畅，`5–10 ms/s` 轻微卡顿，`>= 10 ms/s` 明显卡顿），搭配 **On-time %** 与 **Dropped Frames**。

---

## 2. 统一指标体系与数学公式（单一事实源）

Rust 分析端（[frames.rs](../examples/devtools-desktop/rust/src/store/frames.rs)）与内嵌面板 JS 端（`packages/devtools/src/metrics.ts`）严格使用同一套公式。

### 2.1 两个正交指标维度

| 维度 | 核心指标 | 用途与语义 |
|---|---|---|
| **流畅度（主指标）** | `hitch_ms_per_sec`（`ms/s`）、`dropped_frames`、`on_time_pct`、`ui_jank_count` / `raster_jank_count` / `gpu_jank_count`、`p50_ms` / `p95_ms` / `max_ms`（瓶颈耗时）、`p95_e2e_ms`（延迟） | 衡量是否卡顿、掉了几帧、瓶颈在 UI / Raster / GPU 哪一段，不被静止或低频定时重绘污染。 |
| **渲染频次（次指标）** | `present_rate_hz`（`frames/s`）、`burst_fps: Option<f64>`、`wake_count` | 衡量画面真实提交频次与功耗特征；区分“5/s 后台定时刷新”与“60/s 连续滚动交互”。 |

### 2.2 单帧计算口径

设单帧有效预算 `budget`（优先取 `frame.budget_ms > 0`，否则取 `1000.0 / session.refresh_rate`，均无效时为 `0.0`）：

1. **拓扑区分**：
   $$\text{is\_split} = (\text{frame.topology} \ne \text{"shared"})$$
2. **GPU 耗时（ms）**：
   $$\text{gpu\_ms} = \begin{cases} \text{frame\_span\_us} / 1000.0 & \text{if } \text{frame\_span\_us} > 0 \\ 0.0 & \text{otherwise} \end{cases}$$
3. **流水线单级瓶颈耗时 `stage_duration_ms`**：
   - **Split 拓扑**：$\max(\text{ui\_ms},\ \text{raster\_ms},\ \text{gpu\_ms})$（`compositor_only` 帧 `ui_ms = 0`，即 $\max(\text{raster\_ms},\ \text{gpu\_ms})$）。
   - **Shared 拓扑**：$\max(\text{ui\_ms} + \text{raster\_ms},\ \text{work\_ms},\ \text{gpu\_ms})$。
4. **分阶段超预算判定**：
   - **Split 拓扑**：
     - `ui_over = frame.ui_over_budget || (budget > 0 && frame.ui_ms > budget)`
     - `raster_over = frame.raster_over_budget || (budget > 0 && frame.raster_ms > budget)`
     - `gpu_over = frame.gpu_over_budget || (budget > 0 && gpu_ms > budget)`
   - **Shared 拓扑**：
     - `cpu_over = (budget > 0 && (frame.ui_ms + frame.raster_ms).max(frame.work_ms) > budget)`；超支时按 `ui_ms >= raster_ms` 归因给 `ui_over` 或 `raster_over`（二者皆可标注）。
     - `gpu_over = frame.gpu_over_budget || (budget > 0 && gpu_ms > budget)`
   - `any_over = ui_over || raster_over || gpu_over`
   - `failed_or_skipped = frame.skip_reason.is_some() || !frame.presented`
   - **重要边界**：Split 流水线下**不**把 `e2e_ms > budget` 直接算作吞吐超预算或掉帧；`deadline_overrun_ms` 作为端到端目标截止线超支指标单独统计（Shared 拓扑若启用目标截止线可计入，Split 拓扑仅在延迟分析与单帧明细中展示，避免把双帧管线正常的 1 帧固有延迟重复计入 Hitch）。
5. **单帧掉帧数 `dropped_vsyncs` 与离散 Hitch 时长**：
   - 若 `budget > 0` 且 `stage_duration_ms > budget`：
     $$\text{stage\_dropped} = \max\left(1,\ \left\lceil \frac{\text{stage\_duration\_ms}}{\text{budget}} \right\rceil - 1\right)$$
     否则 `0`。
   - 若 `failed_or_skipped` 或 `any_over` 为真，单帧至少计 `1` 次掉帧：
     $$\text{dropped\_vsyncs} = \max(\text{stage\_dropped},\ \mathbb{I}[\text{failed\_or\_skipped} \lor \text{any\_over}])$$
   - 离散 Hitch 时长（错过截止线至少使上一帧多停留一个完整刷新周期）：
     $$\text{frame\_hitch\_ms} = \begin{cases} \text{dropped\_vsyncs} \times \text{budget} & \text{if } \text{budget} > 0 \\ 0.0 & \text{otherwise} \end{cases}$$

### 2.3 窗口聚合与边界处理

1. **规范窗口时长 `window_sec`**：
   - **Live 模式**（窗口半径 `LIVE_WINDOW_MS = 2_000.0`）：设 `observed_span_ms` 为当前会话首帧至今的时间跨度（以宿主采样时钟为起点，在 Desktop 侧按本地单调时间推进），取 `observed_span_ms.clamp(250.0, LIVE_WINDOW_MS) / 1000.0`。既避免冷启动第 1 秒把 60/s 算成 30/s，也防止稀疏帧因只用 `end - start` 产生分母抖动。
   - **离线回放 / 区间对比模式**：取 `(end - start).max(budget.max(1.0)) / 1000.0`。
2. **聚合公式**：
   - `hitch_ms_per_sec = total_hitch_ms / window_sec`
   - `on_time_pct = (count - jank_count) as f64 / count as f64 * 100.0`（`jank_count` 为 `any_over || failed_or_skipped` 的帧数）
   - `present_rate_hz`：`idle` 时为 `0.0`，否则 `presented_count as f64 / window_sec`
   - `p50_ms` / `p95_ms` / `max_ms`：对窗口内帧的 `stage_duration_ms` 排序取分位数；`p95_e2e_ms` 对 `frame.e2e_ms` 排序取 P95。
3. **连续互动等效帧率 `burst_fps: Option<f64>`**：
   - 当 `idle == true` 或无有效 `target_hz`（`1000.0 / budget`）时返回 `None`。
   - 对窗口内按 `at_ms` 升序的已呈现帧，若相邻帧间隔 $\Delta t \le \max(2.5 \times \text{budget},\ 50.0)\text{ ms}$，归为同一连续段；截取最后一个长度 $\ge 3$ 的连续段（覆盖最近一次滚动/动画）：
     $$\text{burst\_fps} = \text{target\_hz} \times \frac{\text{burst\_presented}}{\text{burst\_presented} + \text{burst\_dropped}}$$
   - 若无长度 $\ge 3$ 的连续段，返回 `None`。
4. **空闲态（`idle`）行为**：
   - `IDLE_AFTER_MS = 500.0`（距最近成功呈现超过 500ms 判定为 `idle`）。
   - `idle == true` 时，`present_rate_hz = 0.0`、`burst_fps = None`；但仍在 2s 窗口内的 `hitch_ms_per_sec`、`dropped_frames` 与 `p95_ms` **继续保留**直至自然滑出窗口，避免手指刚停下，刚刚发生的卡顿记录立刻被清空。

### 2.4 五大典型场景核算校验

| 场景 | 输入条件 | `hitch_ms_per_sec` | `dropped_frames` | `on_time_pct` | `present_rate_hz` | `burst_fps` |
|---|---|---:|---:|---:|---:|---:|
| ① 完全静止 | 2s 内 0 帧 | `0.0` | `0` | `100%` (空显示 `—`) | `0.0` (`Idle`) | `None` |
| ② Gallery 200ms 定时刷新 | 60Hz (`16.67ms`)，2s 内 10 帧，UI=3ms, Raster=4ms, GPU=1ms | `0.0 ms/s` | `0` | `100%` | `5.0/s` | `None` |
| ③ 动静混合（1.5s 定时 + 0.5s 滚动） | 60Hz，7 帧 @200ms + 30 帧 @16.67ms，均不超预算 | `0.0 ms/s` | `0` | `100%` | `18.5/s` | `Some(60.0)` |
| ④ 双线程高负载但流畅 | Split，60Hz，UI=10ms, Raster=10ms, GPU=8ms, E2E=20ms | `0.0 ms/s` | `0` | `100%` | `60.0/s` | `Some(60.0)` |
| ⑤ 连续滚动单帧卡顿 | 60Hz，1s 内 59 帧，其中 1 帧 Raster=25ms（或 GPU=25ms） | `16.67 ms/s` | `1` | `98.3%` | `59.0/s` | `Some(59.0)` |

---

## 3. 分模块详细改动规划

### 3.1 模块 A：`crates/embedder/common`（底层采集、流水线超预算与 GPU 回填）

#### A.1 新增触发源与目标截止时间 — [telemetry.rs](../crates/embedder/common/src/diagnostics/telemetry.rs)

1. 新增 `FrameTrigger` 枚举：
   ```rust
   #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
   pub enum FrameTrigger {
       #[default]
       Wake,       // 定时器 / 异步任务 / 首次唤醒（Command::Wake）
       Vsync,      // 平台 VSync 回调驱动 UI（Command::Frame）
       Compositor, // Raster 托管合成轨道驱动（CompositorMode）
   }

   impl FrameTrigger {
       pub fn name(self) -> &'static str {
           match self {
               Self::Wake => "wake",
               Self::Vsync => "vsync",
               Self::Compositor => "compositor",
           }
       }
   }
   ```
2. `ScheduleTiming` 扩展字段与方法：
   - 新增：`pub trigger: FrameTrigger`、`pub target_present_ns: Option<u64>`（统一在进程单调纳秒坐标系下）。
   - 新增方法：
     - `is_split_pipeline(self) -> bool`：`self.topology != Some("shared")`
     - `e2e_ns(self) -> u64`：`self.present_done_ns.saturating_sub(self.vsync_ns.unwrap_or(self.ui_start_ns))`
     - `target_lead_ms(self) -> Option<f64>`：`self.target_present_ns.map(|target| target.saturating_sub(self.vsync_ns.unwrap_or(self.ui_start_ns)) as f64 / 1_000_000.0)`
     - `deadline_overrun_ms(self) -> Option<f64>`：`self.target_present_ns.map(|target| self.present_done_ns.saturating_sub(target) as f64 / 1_000_000.0)`

#### A.2 透传平台 `target_present_ns` 与触发源

平台胶水层（Android `preferredFrameTimeline.expectedPresentationTimeNanos`、iOS/macOS `CADisplayLink.targetTimestamp`、`CVDisplayLink` `output`）均已将平台目标时间戳写入 `FrameTime.target_ms`，无需修改平台层代码。需通过差值换算以消除平台时钟与进程单调时钟的原点偏移：

- 提取公共辅助函数 `target_present_ns(base_ns: u64, time: FrameTime) -> Option<u64>`：
  - 若 `time.target_ms.is_finite()`、`time.vsync_ms.is_finite()`，且 `lead_ms = time.target_ms - time.vsync_ms` 满足 `0.0 < lead_ms && lead_ms <= 1_000.0`，则 `base_ns.saturating_add(ms_to_ns(lead_ms))`；
  - 否则若 `time.interval_ms.is_finite() && time.interval_ms > 0.0`，退回 `base_ns.saturating_add(ms_to_ns(time.interval_ms))`。
- [ui_frame.rs](../crates/embedder/common/src/host/ui_frame.rs#L392-L417)：`frame_schedule()` 中设置 `trigger: if from_vsync { FrameTrigger::Vsync } else { FrameTrigger::Wake }`；当 `from_vsync` 为真时由 `vsync_ns` 推导 `target_present_ns`。
- [compositor_animation.rs](../crates/embedder/common/src/host/compositor_animation.rs#L475-L487)：`Plan::submission()` 中设置 `trigger: FrameTrigger::Compositor`，由 `posted_ns` 推导 `target_present_ns`，并从 Raster 开始安装窗口和采样轨道时计算阶段耗时。

#### A.3 按流水线阶段判定超预算与 GPU 回填联动

1. [raster_host/frame.rs](../crates/embedder/common/src/host/raster_host/frame.rs#L262-L274)：
   ```rust
   let deadline_ns = schedule
       .target_present_ns
       .or_else(|| (schedule.budget_ns > 0).then(|| schedule.vsync_ns.unwrap_or(schedule.ui_start_ns).saturating_add(schedule.budget_ns)));
   schedule.missed_vsync = deadline_ns.is_some_and(|deadline| schedule.present_done_ns > deadline);
   ```
2. [timing.rs](../crates/embedder/common/src/timing.rs#L44-L52) 与 `record_frame`（[#L240-L322](../crates/embedder/common/src/timing.rs#L240-L322)）：
   - 在 `FrameTiming` 上新增 `ui_over_budget(self) -> bool` 与 `raster_over_budget(self) -> bool`：
     - Split 拓扑：`ui_over_budget = !self.schedule.compositor_only && budget > 0 && self.schedule.ui_ms() > budget`；`raster_over_budget = budget > 0 && self.schedule.raster_ms() > budget`；
     - Shared 拓扑：合并执行，使用 `work_ms > budget`（归入二者中耗时较高的一侧）。
   - 重写 `FrameTiming::over_budget(self) -> bool`：
     - Split 拓扑：`self.ui_over_budget() || self.raster_over_budget()`（不因 `ui + queue + raster` 超过 1 个周期误判）；
     - Shared 拓扑：`self.interval_ms > 0.0 && self.work_ms.max(self.schedule.e2e_ms()) > self.interval_ms`。
3. [hub.rs](../crates/embedder/common/src/diagnostics/hub.rs#L189-L275) & [#L1083-L1124](../crates/embedder/common/src/diagnostics/hub.rs#L1083-L1124)：
   - `FrameSample` 新增 `pub ui_over_budget: bool`、`pub raster_over_budget: bool`、`pub gpu_over_budget: bool`。
   - 在 `TelemetryHub::patch_gpu_timing()` 的 `patch` 闭包中，当 GPU 回填拿到 `timing` 且 `frame.budget_ms > 0.0`：
     ```rust
     if let Some(gpu_timing) = timing.as_ref()
         && frame.budget_ms > 0.0
         && gpu_timing.frame_span_us / 1_000.0 > frame.budget_ms
     {
         frame.gpu_over_budget = true;
         frame.over_budget = true;
     }
     ```
     由于既有 `patch_gpu_timing` 会在返回 `true` 时推进 `frame.changed_at`、`frames.seq`、`self.revision` 并通知 `capture_changed`，远程订阅和 Trace 录制会自动收到回填后的超预算标记。

#### A.4 诊断通道导出补全

- [remote_frame.rs](../crates/embedder/common/src/diagnostics/remote_frame.rs)：填充 `trigger`、`target_lead_ms`、`deadline_overrun_ms`、`ui_over_budget`、`raster_over_budget`、`gpu_over_budget`、`input_events`（由 `frame.gestures + phase.pointers + phase.keys + phase.edits + phase.ime + phase.scrolls + phase.focus + phase.blur` 汇总，供审计判断无交互后台唤醒）。
- [remote.rs](../crates/embedder/common/src/diagnostics/remote.rs#L991-L1023)：`frame_value()` 同步输出上述字段，保持增量流与捕获帧摘要一致。
- [snapshot.rs](../crates/embedder/common/src/diagnostics/snapshot.rs#L214-L288)：`frames_channel` 增加 `gpuSpanMs`、`missedVsync`、`presented`、`trigger`、`targetLeadMs`、`deadlineOverrunMs`、`uiOverBudget`、`rasterOverBudget`、`gpuOverBudget`。

---

### 3.2 模块 B：`crates/devtools-protocol` 与 `packages/devtools/rust`

1. **[crates/devtools-protocol/src/frame.rs](../crates/devtools-protocol/src/frame.rs)**（无需兼容旧协议与旧录制）：
   - 将 `pub presented: Option<bool>` 简化为 `pub presented: bool`，移除 `FrameRecord::was_presented()`，所有调用点（[frames.rs](../examples/devtools-desktop/rust/src/store/frames.rs#L83)、[store/mod.rs](../examples/devtools-desktop/rust/src/store/mod.rs#L260)、[resources.rs](../examples/devtools-desktop/rust/src/store/resources.rs#L27)）直接读取 `frame.presented`。
   - 新增字段：
     - `pub trigger: String`（`"wake" | "vsync" | "compositor"`）
     - `pub target_lead_ms: Option<f64>`
     - `pub deadline_overrun_ms: Option<f64>`
     - `pub ui_over_budget: bool`
     - `pub raster_over_budget: bool`
     - `pub gpu_over_budget: bool`
     - `pub input_events: u32`
   - 清理旧字段缺省反序列化兼容性测试，改为当前 `FrameRecord` 完整字段 serde 往返测试。
2. **[packages/devtools/rust/src/types.rs](../packages/devtools/rust/src/types.rs#L105-L143)** 与 **[native.stub.ts](../packages/devtools/src/native.stub.ts)**：
   - 插件侧 `FrameSample` 同步补全 `presented: bool`、`missed_vsync: bool`、`gpu_span_ms: Option<f64>`、`trigger: String`、`target_lead_ms: Option<f64>`、`deadline_overrun_ms: Option<f64>`、`ui_over_budget: bool`、`raster_over_budget: bool`、`gpu_over_budget: bool`。

---

### 3.3 模块 C：`examples/devtools-desktop/rust`（统计引擎、双柱投影、审计规则）

#### C.1 `FrameStats` 结构重构 — [types.rs](../examples/devtools-desktop/rust/src/types.rs#L38-L51)

```rust
#[derive(Clone, Default, ImpastoType)]
pub struct FrameStats {
    pub count: u32,
    pub presented_count: u32,
    pub failed_present_count: u32,
    pub idle: bool,
    pub budget_ms: f64,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub p95_e2e_ms: f64,
    pub jank_count: u32,
    pub ui_jank_count: u32,
    pub raster_jank_count: u32,
    pub gpu_jank_count: u32,
    pub dropped_frames: u32,
    pub hitch_ms: f64,
    pub hitch_ms_per_sec: f64,
    pub on_time_pct: f64,
    pub compositor_pct: f64,
    pub present_rate_hz: f64,
    pub burst_fps: Option<f64>,
    pub wake_count: u32,
}
```

#### C.2 统计引擎 — [frames.rs](../examples/devtools-desktop/rust/src/store/frames.rs) & [store/mod.rs](../examples/devtools-desktop/rust/src/store/mod.rs#L496-L512) & [lib.rs](../examples/devtools-desktop/rust/src/lib.rs#L573-L615)

- 在 `frames.rs` 新增统一纯函数：`is_split(frame)`、`cpu_duration(frame)`、`gpu_duration(frame)`、`stage_duration(frame)`（单级瓶颈耗时）、`effective_budget(frame, fallback_budget)`、`stage_flags(frame, fallback_budget)`、`dropped_vsyncs(frame, fallback_budget)`、`frame_hitch_ms(frame, fallback_budget)`、`detect_burst_fps(entries, fallback_budget)`。
- `live_stats(frames, idle, window_ms, fallback_budget)` 与 `stats(frames, fallback_budget)`：
  - `store/mod.rs` 从 `data.info.as_ref().and_then(|info| info.refresh_rate).map(|hz| 1_000.0 / f64::from(hz))` 计算 `fallback_budget`，并传入实时观测窗口跨度（由首帧至 `now` 计算，钳位于 `[250.0, LIVE_WINDOW_MS]`）。
  - `lib.rs` 中 `worst_frame()` 改为按 `frames::stage_duration` 排序；`compare_frame_range()` 从 `state.data.info` 传入 `fallback_budget` 调用 `frames::stats`。

#### C.3 Frame Strip 双柱投影与总览轨道 — [projection.rs](../examples/devtools-desktop/rust/src/projection.rs#L173-L289) & [#L658-L683](../examples/devtools-desktop/rust/src/projection.rs#L658-L683)

1. **`frame_strip` 彻底去除跨线程垂直硬叠**：
   - Y 轴上限基于 `frames::stage_duration(entry).max(entry.budget_ms)` 计算，不再用全阶段总和拉高比例尺。
   - **Split 拓扑（正常分线程帧）绘制并排双子柱**（总宽 `w`，左/右子柱宽 `sub_w = (w - 1.0).max(w * 0.45) * 0.5`，间距 `1px`）：
     - **左半柱（UI 阶段）**：垂直堆叠 `script_ms`（橙 0）+ `layout_ms`（紫 1）+ `record_ms`（青绿 2）；若 `ui_over_budget` 则在左柱顶部画红色（7）超预算帽。
     - **右半柱（渲染阶段）**：绘制 `raster_ms`（蓝 4）；若存在 `gpu_ms > 0`，当画幅足够时在右半柱旁/内置子条或叠加微调对比显示 `gpu_ms`（绿 5，各自对照红线而非线性相加突破红线，保证当 `raster <= budget` 且 `gpu <= budget` 时二者皆处于红线下方）；若 `raster_over_budget || gpu_over_budget` 则在右柱顶部画红色（7）超预算帽。
   - **Compositor 托管帧**：单画全宽青色柱（6，高为 `stage_duration`），超线顶标红帽。
   - **Shared 串行拓扑**：保留单柱垂直堆叠 `script + layout + record + raster`（与 `gpu` 独立对照红线）。
   - **跳过/呈现失败帧**：底部保留红色失败条。
2. **`hit_frame_strip` Tooltip**：
   - 更新为：`UI {ui:.1} · Raster {raster:.1} · GPU {gpu:.1} · E2E {e2e:.1} / {budget:.1} ms · {trigger}`（有掉帧时追加 `· +{dropped} dropped`）。
3. **`overview_track(0)`**：
   - 将总览第一条轨迹的采样值由 `frames::duration(frame)` 改为 `frames::stage_duration(frame)`（对应图表标题 `Frame bottleneck`），确保高 GPU 帧也能冲破预算虚线、健康双帧流水线不误越线。

#### C.4 审计规则升级 — [audits.rs](../examples/devtools-desktop/rust/src/analyzer/audits.rs)

1. **升级 `frame.jank_burst`**：
   - 对最近 60 帧按新公式计算窗口 `hitch_ms_per_sec`、`dropped_frames` 及 `ui_jank_count` / `raster_jank_count` / `gpu_jank_count`。
   - 触发阈值：`hitch_ms_per_sec >= 10.0` 或 `dropped_frames >= 3` 或存在 `stage_duration > 2 * budget` 时报 `"critical"`；`5.0 <= hitch_ms_per_sec < 10.0` 时报 `"warning"`。
   - `metric` 示例：`16.7 ms/s hitch · 2 dropped (Raster)`，明确指明主导瓶颈段（UI / Raster / GPU）。
2. **新增 `frame.idle_repaint_churn`（无交互后台定时重绘检测）**：
   - 取最新一帧往前 5 秒的观测窗口。当帧时间跨度 `>= 2_000ms` 且至少包含 10 帧时，统计其中成功呈现且 `trigger == "wake"`、`input_events == 0`、`!compositor_only` 的纯定时/异步唤醒重绘频次 `wake_hz`：
     - `wake_hz >= 20.0`：报 `"warning"`（疑似未暂停的动画回调或高频定时器空转）；
     - `2.0 <= wake_hz < 20.0`：报 `"info"`（提示存在后台低频周期重绘，如 Gallery 200ms `5/s` 刷新，附带平均 `timer_ms` 和已采集样本的平均 `repainted_view_count`）。
   - `evidence()` 列出触发源为 `wake` 且消耗最高的帧及其 `layout_cause`、`timer_ms` 和 `repainted_view_count`。

---

### 3.4 模块 D：双端 DevTools UI

#### D.1 DevTools Desktop（`examples/devtools-desktop/src/`）

1. **[workspace.ts](../examples/devtools-desktop/src/state/workspace.ts#L40-L46)**：
   - 移除含糊的单一 `cadence`，暴露结构化视图派生状态：
     - `hitchLabel`：无帧时 `"—"`，否则 `"{hitchMsPerSec.toFixed(1)} ms/s"`（`< 0.05` 显示 `"0 ms/s"`）；
     - `hitchTone`：`>= 10` 返回 danger，`>= 5` 返回 warn，否则正常/ok；
     - `cadenceLabel`：`idle` 时显示 `"Idle"`（若全部失败显示 `"No presents"`），存在 `burstFps` 时显示 `"{burstFps.toFixed(0)}/s burst"`，否则显示 `"{presentRateHz.toFixed(1)}/s"`。
2. **[OverviewPage.tsx](../examples/devtools-desktop/src/pages/OverviewPage.tsx#L11-L59)**：
   - 第 1 张指标卡从 `FPS` 改为 **`Hitch`**：
     - 主值：`hitchLabel()`（带分级着色）；
     - 副文：无掉帧时显示 `"{onTimePct}% on-time · {cadenceLabel} · {targetHz} Hz"`；有掉帧时显示 `"{droppedFrames} dropped ({UI/Raster/GPU 拆分}) · {cadenceLabel}"`。
   - 第 2 张指标卡 **`Frame P95`**：
     - 主值：基于瓶颈耗时的 `p95Ms`；
     - 副文：`"{budget} ms budget · E2E {p95E2eMs}"`（如有超预算帧附 `"{jankCount} over"`）。
   - Activity 第 1 条轨迹名称由 `"Frame total"` 更新为 `"Frame bottleneck"`。
3. **[Workbench.tsx](../examples/devtools-desktop/src/shell/Workbench.tsx#L31-L39)** & [#L160-L164](../examples/devtools-desktop/src/shell/Workbench.tsx#L160-L164)：
   - 底部状态栏改为：`"{hitch} hitch · {cadence} · P95 {p95}"`（空闲时显示 `"Idle · {hitch} hitch · P95 {p95}"`）。
4. **[FramesPage.tsx](../examples/devtools-desktop/src/pages/FramesPage.tsx#L156-L165)**：
   - 区间对比表扩充为：`Hitch (ms/s)`、`Dropped`、`On-time`、`Cadence`、`P95 bottleneck`、`P95 E2E`、`UI / Raster / GPU over`。

#### D.2 应用内悬浮面板（`packages/devtools/`）

1. **新增 `packages/devtools/src/metrics.ts`**（纯 TS 计算模块）及 `packages/devtools/src/metrics.test.ts`：
   - 实现与 Rust 完全对齐的 `frameStageMs()`、`frameDroppedVsyncs()`、`frameHitchMs()`、`summarizeFrames()`。
2. **[performance.tsx](../packages/devtools/src/pages/performance.tsx#L9-L140)**：
   - 顶部汇总文本升级为：`Hitch {hitchMsPerSec} ms/s · 掉帧 {droppedFrames} · {presentRateHz}/s · {count} 帧 · 峰值 {peak}ms`。
   - 柱状图高度按 `frameStageMs(frame)`（单级瓶颈）计算，不再误把跨线程总和作超预算判断；告警色同步受 `uiOverBudget || rasterOverBudget || gpuOverBudget` 驱动。
   - 单帧明细表新增：`触发源`（`vsync` / `compositor` / `wake`）、`阶段瓶颈 (UI / Raster / GPU)`、`目标截止 / 超支`、`阶段超预算归因`。

---

## 4. 实施顺序与关键坑点

### 4.1 四阶段实施顺序

1. **M1 — Embedder 与协议层**：修改 `telemetry.rs`、`ui_frame.rs`、`compositor_animation.rs`、`raster_host/frame.rs`、`timing.rs`、`hub.rs`、`remote_frame.rs`、`remote.rs`、`snapshot.rs`、`crates/devtools-protocol/src/frame.rs`、`packages/devtools/rust/src/types.rs` 与 `native.stub.ts`；先跑通 `cargo test -p embedder -p devtools-protocol`。
2. **M2 — Desktop Rust Core**：改造 `examples/devtools-desktop/rust/src/{types.rs, lib.rs, store/frames.rs, store/mod.rs, store/resources.rs, projection.rs, analyzer/audits.rs}`，补全 6 大核心场景单测。
3. **M3 — 绑定同步与 Desktop UI**：运行 `impasto sync --platform macos` 更新 `.impasto/types/`，然后改造 `workspace.ts`、`OverviewPage.tsx`、`Workbench.tsx`、`FramesPage.tsx`。
4. **M4 — 应用内面板、文档回写与全量验证**：落地 `packages/devtools/src/{metrics.ts, metrics.test.ts, pages/performance.tsx}`；同步更新 [telemetry-and-p1-plan.md](telemetry-and-p1-plan.md#L58) 中关于 `missed_vsync` 的旧描述；执行全量门禁与本地实机验收。

### 4.2 关键注意事项

1. **`FrameRecord::presented` 简化后的全仓清理**：改为非可选 `bool` 后，同步检查 `examples/devtools-desktop/rust` 与 `crates/embedder/common` 中全部测试构造点，确保默认值或显式赋值与预期一致（默认 `false` 时在测试辅助构造里显式置 `presented: true`）。
2. **`impasto sync` 生成顺序**：所有暴露给 TS 的 Rust 类型（`FrameStats`、`packages/devtools/rust/src/types.rs`）改动后，必须先执行 `impasto sync` 再做 TS 类型检查，严禁手动修改 `.impasto/types/`。
3. **不越过依赖边界**：新增枚举与计算函数全部留在 `embedder` / `devtools-protocol` / `devtools-desktop` / `packages/devtools`，保持 `just boundaries` 通过。

---

## 5. 验证与验收清单

### 5.1 自动化测试矩阵

| 层级 | 测试内容 |
|---|---|
| `crates/embedder/common` | ① `target_present_ns` 与 `deadline_overrun_ms` 换算；② `FrameTrigger` 在 `Wake` / `Vsync` / `Compositor` 下的正确标记；③ Split 流水线 `UI=10ms, Raster=10ms @ 16.67ms` 不误判 `over_budget`；④ `patch_gpu_timing` 在 GPU 超预算时回填 `gpu_over_budget = true` 并递增 `changed_at`。 |
| `crates/devtools-protocol` | 新版 `FrameRecord` 全字段 serde 往返校验。 |
| `examples/devtools-desktop/rust` | ① 完全静止；② 200ms 定时刷新（`0 ms/s hitch` + `5/s` + `burst_fps = None`）；③ 动静混合窗口 `burst_fps` 不被稀释；④ Split 流水线双线程满载不误判 Jank；⑤ UI / Raster / GPU 各自超预算时的掉帧数与归因；⑥ `frame.jank_burst` 与 `frame.idle_repaint_churn` 审计边界；⑦ `frame_strip` Split 双柱与坐标裁剪投影断言。 |
| `packages/devtools` | `metrics.test.ts` 验证 JS 侧 Hitch / 掉帧 / 频次计算与 Rust 一致。 |

### 5.2 命令门禁与实机验收

```bash
# 1. Desktop 绑定同步 + Rust clippy/test + TS 静态分析
just devtools-desktop

# 2. 仓库全量门禁（style-check, boundaries, clippy, test, solid, standard-check）
just check

# 3. macOS 宿主（just check 默认排除 embedder-macos）与格式校验
cargo clippy -p embedder-macos --all-targets -- -D warnings
cargo test -p embedder-macos
cargo fmt --all -- --check
```

- **实机视觉验收**（`just host` 启动 Gallery + 启动 `devtools-desktop` 挂载）：
  - **静止行情页（200ms 定时刷新）**：Overview 卡片显示 `Hitch: 0 ms/s`、`100% on-time · 5/s`，不再误导性显示 `5 fps`；状态栏显示 `0 ms/s hitch · 5/s`。
  - **高速甩动列表**：Frame Strip 对 Split 帧并排显示 UI 与 Raster/GPU 双柱，仅真正越线阶段标红帽；滚动结束后 500ms 状态栏自动切回 `Idle` 且保留窗口内刚发生的卡顿记录。
