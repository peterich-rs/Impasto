# 补齐框架核心路径的运行日志

要观察的是整条框架路径，不是只看一帧画完没有。手势、布局、录制、提交、脚本、未处理的 Promise、网络和文件 IO 都要能在日志里对上。仍然只打失败和边沿，不把每一次移动、每一个网络分块打出来。

## 1. 可行性评估

这些路径今天都有落点，只是日志没对齐：

- 手势计数在 `FrameTrace`，指针在 `trace`。监听器失败进 `note_script_error`，再被当成 `console` 的 error。
- 布局溢出在扫描打开时打 `impasto::layout`。`Tree::layout` 失败走 `hold_frame`，target 却是 `impasto::script`。
- `Tree::record_into` 失败被清掉命令，没有日志（`crates/quickjs/src/linked/mod.rs`）。
- 提交失败、present 退避和恢复已经有日志。picture 是否在重建只在 `SubmitStats` 里。
- 停帧的脚本错误打 `impasto::script`。未处理拒绝在 `glue.c` 的 `report_rejection`：装了 `__reportUnhandled` 就不再调 `fns.log`，框架日志会丢。
- HTTP 失败打 `impasto::net`，没有方法和 URL。流式失败和 socket 错误只进 devtools 的网络环。
- `files.rs` 的读写失败只变成 Promise 的 `Failed`，没有日志。

同一条 `tracing` 事件已经同时进控制台、xlog 和 devtools 日志环。`uicore` 与 `engine` 继续不依赖 `tracing`。

可行。每帧的速率仍在 `FrameSample`，不进缺省日志。

## 2. 现有面清单

- `crates/hostshell/src/shell.rs` `console_log` — QuickJS 的 0..4 映射到 `impasto::script::console`。
- `crates/hostshell/src/shell.rs` 帧初 — 每个指针 `trace` `pointer`，输入 `trace` `input`，不带种类。
- `crates/quickjs/src/linked/mod.rs` `hold_frame` — `error` `impasto::script`，并再走 `note_script_error`。
- `crates/quickjs/src/linked/mod.rs` `note_script_error` — 调 `(inner.log)(3, …)`，于是监听器异常变成 console error。
- `crates/quickjs/src/linked/mod.rs` `Tree::layout` 的 `Err` — `hold_frame`，没有 `impasto::layout`。
- `crates/quickjs/src/linked/mod.rs` `record_into` — `is_err` 时清空命令并 `clear_paint`。
- `crates/quickjs/src/linked/mod.rs` 溢出循环 — `warn` `impasto::layout`，带轴、父节点、超出量。扫描关掉则没有。
- `crates/quickjs/src/glue.c` `report_rejection` — 文本以 `unhandled rejection:` 开头。hook 成功则不调用 `fns.log`。
- `crates/hostshell/src/net.rs` `serve` — HTTP 的网络和超时是 `warn`，其它失败是 `error`，取消不打。流式失败和 socket 错误不打 `tracing`。
- `crates/hostshell/src/files.rs` — `read_document`、`write_document`、`delete_document` 和 store 的失败返回 `String`。
- `crates/hostshell/src/shell.rs` 提交与 present — `scene submit failed`、退避、恢复。
- `crates/hostshell/src/timing.rs` `BudgetEdge` — 超预算只带 `session_ms`、`submit_ms`、`present_ms`、`budget_ms`。
- `crates/session/src/lib.rs` `FrameTrace` — `pointers`、`scrolls`、`gestures`、`layout_cause`、`text_shaped` 等，只进 `FrameSample`。

## 3. 设计

### 3.1 关键决策

1. **按路径分 target，不把失败都收成 console 或 script。** 手势监听器失败是 `impasto::gesture`。布局失败是 `impasto::layout`。录制失败是 `impasto::render::record`。未处理拒绝是 `impasto::script::promise`。文件失败是 `impasto::io`。网络保持 `impasto::net`。应用自己的 `console.*` 仍是 `impasto::script::console`。
2. **失败打一次，连续的做功用边沿。** 移动中的指针、`DragBy`、`Scroll`、HTTP 分块、成功的文件读取不打。连续超预算、连续排版、连续重建 picture、连续录制失败，都是进入和离开各一条。
3. **停帧只留 `hold_frame` 一条 `impasto::script`。** 布局失败在调用 `hold_frame` 之前另打 `impasto::layout`。`note_script_error` 改为打 `impasto::script`，不再调用 console 的 level 3，避免同一句出现两次。错误页仍读 `take_script_error`。
4. **未处理拒绝始终进框架日志。** `report_rejection` 在调用 `__reportUnhandled` 之后仍然调用 `fns.log`。`console_log` 看见前缀 `unhandled rejection:` 时改 target 为 `impasto::script::promise`，正文不改。已处理的拒绝继续不打。
5. **网络和文件只打失败，并带上能对上调用的字段。** HTTP 补 `method` 和 `url`。流式失败和 socket 的 `error` 事件走同一条。文件失败带 `op` 和文档相对路径，不带沙箱根路径。
6. **不改 `Targets`、sink、`FrameSample`。** 不在 `uicore` 或 `engine` 里加 `tracing`。指针移动维持 `trace`。

### 3.2 各路径

| 路径 | 今天 | 改成 |
|---|---|---|
| 手势 | 指针 `trace`；监听器失败进 console | 指针 `trace` 增加 `kind`（`down` / `move` / `up` / `cancel`）。输入 `trace` 增加 `kind`（`key` / `text` / `edit` / `ime`）。`dispatch_gestures` 失败打 `error` `impasto::gesture`。`ScrollPhaseChanged`、`DragBegin`、`DragEnd` 打 `debug` `impasto::gesture`，带 `phase` 或 `kind`。`DragBy` 和 `Scroll` 不打 |
| 布局 | 溢出 `warn`；`layout` 失败算脚本 | `Tree::layout` 的 `Err` 打 `error` `impasto::layout`，字段 `error`，然后 `hold_frame`。溢出行保持。`text_shaped > 0` 的连续段见 3.3 |
| 录制 | 失败被清空 | `record_into` 的 `Err` 用 `Streak`：进入打 `error` `impasto::render::record`，字段 `error`；持续失败不重复；恢复打 `info` `record recovered`，字段 `frames` |
| 提交 | 失败、退避、恢复已有 | 不新增成功日志。`picture_rebuilt > 0` 的连续段见 3.3。`note_step` 的 `frame step` 仍是 `debug` |
| 脚本 | `hold_frame` 打 `impasto::script`，同时又进 console | `hold_frame` 只保留一条 `error` `impasto::script`。`note_script_error` 打同一 target，不再走 `console_log` |
| Promise | hook 存在时框架日志没有这条 | 始终 `fns.log`。target 为 `impasto::script::promise`，级别 `error`，正文仍以 `unhandled rejection:` 开头 |
| 网络 | HTTP 失败没有 method/url；流和 socket 不打 tracing | `warn` 仍是网络和超时，其它失败是 `error`，取消不打。字段加 `method`、`url`。流式 `Err` 同样打。socket 只在 `error` 事件打 `warn`，字段 `url`。成功响应和分块仍只进网络环 |
| 文件 | 失败只拒绝 Promise | `read_document`、`write_document`、`delete_document`、`store_set`、`store_remove`、`store_clear` 的 `Err` 打 `warn` `impasto::io`，字段 `op`、`path`、`error`。读到不存在的 store 文件不是失败。成功不打 |

### 3.3 帧上的边沿

`Streak` 放在 `crates/hostshell/src/timing.rs`。`observe(hot)` 在上升沿给出 `Enter`，持续为真是 `Stay`，下降沿是 `Leave { frames }`。`Stay` 和 `Idle` 不打日志。

| 边沿 | 条件 | target | 进入 | 离开 |
|---|---|---|---|---|
| 预算 | `over_budget` | `impasto::render` | `warn` `frame over budget` | `info` `frame back under budget`，`frames_over` |
| 排版 | `text_shaped > 0` | `impasto::render::text` | `info` `text shaping` | `info` `text shaping stopped`，`frames` |
| picture | `picture_rebuilt > 0` | `impasto::render::picture` | `info` `picture rebuild` | `info` `picture rebuild stopped`，`frames` |
| 录制 | `record_into` 失败 | `impasto::render::record` | `error`，字段 `error` | `info` `record recovered`，`frames` |

预算进入行使用这一帧已经测好的值：`frame`、`session_ms`、`gesture_ms`、`script_ms`、`layout_ms`、`record_ms`、`submit_ms`、`present_ms`、`budget_ms`、`layout_cause`、`layout_passes`、`text_shaped`、`picture_rebuilt`、`scrolls`。`script_ms` 严格大于手势、布局、录制、提交、上屏时，再写 `motion_ms`、`dispatch_ms`、`timer_ms`、`raf_ms`、`callback_ms`、`flush_ms`。

排版进入行：`frame`、`text_shaped`、`text_reused`、`layout_cause`、`layout_passes`。picture 进入行：`frame`、`picture_rebuilt`、`picture_reused`、`layer_count`。

录制的 `Streak` 在 QuickJS 会话里，不放进 `FrameListener`。录制发生在 `Session::frame` 返回之前。

另外三处状态，与帧边沿无关：表面 `Destroyed::Dropped` 打 `info` `impasto::render::surface` `surface destroyed`；`Invalidate` 打 `debug` `impasto::render::gpu`；图片 `reload` 打 `debug` `impasto::render::image`，字段 `source`。

### 3.4 缺省能看见什么

debug 构建缺省 `trace`，上面的 `debug` 和 `trace` 也会输出。release 构建缺省 `info`，看见停帧、未处理拒绝、布局失败、录制失败、提交失败、网络失败、文件失败、三条帧边沿和表面销毁。手势阶段、`Invalidate` 和指针移动要把级别调高才出现。

```rust
hostshell::set_log_targets(
    hostshell::Targets::new()
        .with_default(hostshell::LevelFilter::INFO)
        .with_target("impasto::gesture", hostshell::LevelFilter::DEBUG),
);
```

## 4. 分阶段实施

### Phase 1：连续段与预算行

- **File: `crates/hostshell/src/timing.rs`** — `Streak`。`BudgetEdge` 进入行按 3.3 写字段。恢复行仍是 `frames_over`。
- **File: `crates/hostshell/src/shell.rs`** `frame_listeners` — 增加 `WorkEdge`，负责排版和 picture 两条边沿。有没有 hub 都装。
- 验证：超预算连续两帧只有一条 `warn`。`text_shaped` 与 `picture_rebuilt` 只在开始和结束各一行。

### Phase 2：脚本、Promise、手势失败

- **File: `crates/quickjs/src/linked/mod.rs`** — `note_script_error` 打 `error` `impasto::script`，去掉 `(inner.log)(3, …)`。`hold_frame` 不再自己打第二条。`Tree::layout` 失败先打 `impasto::layout`。`dispatch_gestures` 的 `Err` 打 `impasto::gesture`。`ScrollPhaseChanged`、`DragBegin`、`DragEnd` 打 `debug`。
- **File: `crates/quickjs/src/glue.c`** `report_rejection` — hook 之后仍然调用 `fns.log`。
- **File: `crates/hostshell/src/shell.rs`** `console_log` — 前缀 `unhandled rejection:` 使用 target `impasto::script::promise`。
- **File: `crates/hostshell/src/shell.rs`** — 指针 `trace` 写 `kind`；输入 `trace` 按 `InputEvent` 写 `kind`。
- 验证：现有「unhandled rejection」测试仍能在日志正文里找到该前缀。`hold_frame` 只有一条 `impasto::script`。

### Phase 3：录制、网络、文件

- **File: `crates/quickjs/src/linked/mod.rs`** — `record_into` 的错误按 `Streak` 打 `impasto::render::record`。恢复时打 `info`。
- **File: `crates/hostshell/src/net.rs`** — HTTP 与流式失败带 `method`、`url`。socket 的 `error` 事件打 `warn`。取消和成功响应不打。
- **File: `crates/hostshell/src/files.rs`** — 3.2 列出的失败打 `warn` `impasto::io`。
- **File: `crates/hostshell/src/shell/surface_cmds.rs`** — `Dropped` 打 `surface destroyed`。
- **File: `crates/hostshell/src/shell/cache_cmds.rs`** — `invalidate` 与 `reload` 打 `debug`。
- **File: `docs/logging-redesign.md`** — 删掉「超预算不带计时」和「监听器失败只进 console」这类与本文冲突的句子。全量分段仍只在 `FrameSample`。

## 5. 架构说明

- 没有改 `FrameListener` 的签名，也没有改 `hostshell` 对外的类型。
- 日志在已经发生失败或边沿的那一层打。`uicore` 的 `pointer` 仍只返回 `Gesture`，不打日志。
- 不把 `FrameSample` 写进 xlog，不新增依赖，不改 `Targets` 的匹配。
- 明确不做：每个 `DragBy` 或 `Scroll` 一行、每个 HTTP 分块一行、每次成功的文件读取一行、在 `engine` 内部打提交日志。Vulkan swapchain 仍直接写 logcat。

## 6. 文件变更汇总

- `crates/hostshell/src/files.rs` — 文件与 store 失败打 `impasto::io`
- `crates/hostshell/src/net.rs` — 流式失败和 socket 错误进入 `impasto::net`，HTTP 补 `method` 与 `url`
- `crates/hostshell/src/shell.rs` — console 前缀分流、指针和输入的 `kind`、安装 `WorkEdge`
- `crates/hostshell/src/shell/cache_cmds.rs` — `invalidate` 与 `reload` 的 `debug`
- `crates/hostshell/src/shell/surface_cmds.rs` — 表面销毁
- `crates/hostshell/src/timing.rs` — `Streak`、预算行字段、排版和 picture 边沿
- `crates/quickjs/src/glue.c` — 未处理拒绝始终调用 `fns.log`
- `crates/quickjs/src/linked/mod.rs` — 脚本、布局、手势、录制的日志
- `docs/logging-redesign.md` — 去掉与本方案冲突的旧句
