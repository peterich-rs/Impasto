# 用 tracing 重做框架日志

> 设计文档。管道已接到代码。和代码不一致时以代码为准。
> 日志只有三层：调用点、`tracing` 分发、落点（logcat / stdout、xlog 文件、devtools hub）。
> 调用点不写 logcat，也不写文件。`uicore` 与 `engine` 不依赖 `tracing`。
> 每帧计时不是日志，走 `FrameListener`。超预算只在边沿打一条事件，再进同一条分发。

## 1. 可行性评估

现有产生点都在 `hostshell`、`quickjs`、`macos`、`android` 和 `engine` 的一处 Android hook 上。`DiagnosticsHub` 已有 `logs` / `errors` / `frames` 环形缓冲。`Session::frame` 已交出 `FramePhases`，`shell.rs` 已收成 `FrameTiming`。缺的是调用点上的级别和字段，以及一个把成品账本交出去的监听器。

`tracing` 0.1 是中间层：调用点宏、`callsite` 兴趣缓存、字段 `Visit`、一个进程内订阅者。`tracing-subscriber` 0.3 的 `Targets` 只放进 `hostshell`。进程里一份过滤，缺省 `info`。`set_log_targets` 整份替换，然后 `callsite::rebuild_interest_cache()`。不读环境变量。落点不再各自过滤。

落点是可替换的 sink，同一条已经放行的事件同时送出：

- 系统控制台：macOS stdout / stderr，Android `__android_log_write`。logcat 的 tag 是 target，正文不再重复级别和 target。
- 文件：`mars-xlog`。它只做落盘，不做门面。`hostshell` 依赖 crate `mars-xlog`，不依赖 `mars-xlog-core`。宿主提供 `log_dir`。没有目录就不装这个 sink。
- 进程内：devtools 的 `logs` 环形缓冲。hub 不在就不装。

不自建门面 crate。不把 `tracing-subscriber` 或 `mars-xlog` 放进 `uicore` 或 `engine`。

级别名只认 `error`、`warn`、`info`、`debug`、`trace`。`trace` 是 `TRACE`。不认 `warning`。devtools 的 `LogLine.level` 是 `tracing` 的级别值。

可行，有上述三处行为变化。

## 2. 现有面清单

产品路径上的日志，不是 `build.rs` 的 `cargo:` 行，也不是 bench 里的 `println!`。

- `crates/hostshell/src/error.rs` — `LogLevel` 四级。`parse` 把 `trace` 收成 `Debug`，未知字符串收成 `Info`。
- `crates/hostshell/src/lib.rs` — `Services::log(&self, level, message: &str)`。`ConsoleLog = Box<dyn Fn(u8, &str)>`。
- `crates/hostshell/src/shell.rs` `console_log`（约 33 行）— console 编号翻译，并用 `"horizontal overflow "` / `"vertical overflow "` 前缀猜 `LogSource`。
- `crates/hostshell/src/shell.rs` `install_panic_hook`（约 220 行）— `hub.error("host", …)`，再交回默认 hook。
- `crates/hostshell/src/shell.rs` `on_frame`（694 行起）— 脚本错误只进 `hub.error`；`dump_list` 打整棵层树；剪贴板失败；渲染线程 panic 保持上一帧；`scene submit failed` 同时 `runtime.log` 和 `hub.error`。
- `crates/hostshell/src/shell.rs`（约 975–1026 行）— 超预算时把 gesture、script、layout、submit、present、shape、picture、js 各段拼成一行 `Warn`。logcat tag 是 `impasto`。这行是当前刷屏的来源。`FrameSample` 没有 shape 的 `b/mh/mm/ph/pm`、`picture_rebuilt`、`picture_reused`、`layout_cause`、`gestures`。
- `crates/hostshell/src/shell.rs` — inspect 写盘失败（约 295、1028 行）；present 退避用 `present_logged` 只打一次（约 1062 行）；表面放弃重试（约 1129 行）；缺图与解码线程停止（约 1175、1183 行）；字体与引擎启动失败（约 1280–1309 行）。
- `crates/hostshell/src/shell/cache_cmds.rs` — `ImageReady` 失败信息打 `Error`。
- `crates/hostshell/src/shell/session_cmds.rs` — present 成功时清 `present_logged`。
- `crates/hostshell/src/present.rs` — `PRESENT_BACKOFF_AFTER` 为 3。注释写明退避后打一次日志。
- `crates/hostshell/src/images.rs` — 缺图、解码失败、像素上限、缓存超预算各自有一次性开关，返回已经拼好的句子。
- `crates/hostshell/src/net.rs` — 网络线程 `hub.log(3, Net, …)`。
- `crates/hostshell/src/diagnostics/hub.rs` — `LogEntry.level` 是 console 的 `u8`。`error` 同时写入 `errors` 和 `logs`。
- `crates/hostshell/src/diagnostics/module.rs` `logs` — 交出 `seq`、`at`、`level`、`source`、`text`。
- `crates/hostshell/src/timing.rs` — `FrameTiming` 与 `write_inspect`。测量不依赖有没有人听。
- `crates/session/src/lib.rs` — `FramePhases`、`ScriptPhases`、`FrameTrace`。会话交出数据，自己不打日志。
- `crates/quickjs/src/linked/mod.rs` — `hold_frame` 只记下脚本错误；溢出走 `(inner.log)(2, issue.log_line())`（约 899 行）。扫描默认关，`Tree::set_debug_scan`。
- `crates/uicore/src/tree/debug.rs` — `LayoutIssue::log_line` 拼一整行。
- `crates/macos/src/app.rs` — `host ready`、`text command ignored`（约 418、685 行）。Error/Warn 走 stderr，过滤在订阅者。
- `crates/macos/src/surface.rs` — dump 失败与 `frame dumped` 直接 `eprintln!` / `println!`。
- `crates/android/src/host.rs` — `log_line` 调 `__android_log_write`，tag 固定 `impasto`。JNI attach、表面丢弃线程、窗口缓冲、关机失败都走这里。
- `crates/engine/src/vulkan.rs` `log_swapchain` — 创建 swapchain 的 hook 里直接 `__android_log_write`。没有 `hostshell` 调用栈。
- `packages/devtools/src/pages/console.tsx` — 按 0 log、1 info、2 warn、3 error、4 debug 上色。

`scheduler.rs` 的 `frame_step` 是纯函数，今天不打日志。`Gpu::Flush`、`Invalidate`、`MemoryWarning`、表面打开成功今天也没有日志。

## 3. 设计

### 3.1 关键决策

1. **调用点用 `tracing` 宏，不自建 `crates/logging`。** 级别写在宏名上，关闭时参数不求值。`tracing::event!` 留给级别要到运行时才知道的桥（console）。否决单一的 `log(level, String)`：字符串在进函数前就已经拼好，热路径退回到今天的 `format!`。
2. **只依赖 `tracing`，不把业务 crate 放到 `tracing-core` 上。** 宏和 `callsite` 在 `tracing`。订阅者 trait 由 `tracing` 再导出。`tracing-subscriber` 只以 `default-features = false`、`features = ["std"]` 进 `hostshell`，用它的 `Targets`。不要 `env-filter`、`fmt`、`json`、`registry`。
3. **中间层只有一个 `ImpastoSubscriber`。** 装在进程启动后、第一条产品日志之前。`register_callsite` 读当前 `Targets`。放行后 `Visit` 一次，收成一条 `LogRecord`，再按注册顺序交给全部 sink。sink 不过滤、不改字段。不给 span 的创建、进入、退出另打一行。否决调用点直接 `Xlog::log` 或 `__android_log_write`：那会绕过级别、target 和其它落点。
4. **帧账本不是日志。** `on_frame` 继续测量，收成 `FrameTiming` 后调用 `FrameListener::on_frame`。hub、inspect、超预算边沿是三个监听器。帧循环里不再 `format!` 计时行。
5. **`uicore` 和 `engine` 不依赖 `tracing`。** 布局溢出仍由 `LayoutIssue` 交出，`quickjs` 在取走问题的地方打事件。`engine::Status` 仍是返回值，`shell.rs` 在 `submit` / `register_font` 失败处打事件。`vulkan.rs` 的 swapchain hook 保持直接写 logcat，不为这一行给 `engine` 加依赖。
6. **五级，按频率而不是按模块凑齐。** `TRACE` 就是要的 verbose。高频率路径只在状态变化时打 `DEBUG`，不按帧抽样刷屏。
7. **Android tag 用 target。** logcat 自己有级别列，正文不写 `INFO` 这类词，也不再把 target 写进正文。macOS 和 xlog 仍是 `级别 target 消息`。
8. **`DiagError` 保留。** panic 和带栈的脚本错误仍走 `hub.error`。订阅者不再把同一句重复推进 `logs`。今天 `error` 会双写，这条去掉。

### 3.2 两条路径

```text
调用点（tracing::error! 等，带 target 与字段）
       │
       ▼
tracing callsite
       │
       ├── Interest::never ──► 返回（字段不求值）
       │
       └── enabled ──► ImpastoSubscriber
                          │  Visit 一次 → LogRecord
                          ▼
                    按注册顺序交出（不再过滤）
                          ├──► ConsoleSink     stdout / stderr 或 logcat
                          ├──► XlogSink        mars-xlog 文件；无 log_dir 则不装
                          └──► DevtoolSink         logs 环形缓冲；无 hub 则不装
```

```text
Session::frame ──► FramePhases
shell 补 gesture、submit、present、shape、picture ──► FrameTiming
       │
       ▼
FrameListener::on_frame
       ├──► DevtoolFrames                         frames 环形缓冲
       ├──► InspectFile                       仅 IMPASTO_INSPECT
       └──► BudgetEdge ──► tracing::warn!     进入带分段原因，恢复一条。不是每一帧
```

测量留在现在的位置：`quickjs` 填 `FramePhases`，`shell.rs` 补宿主段。监听器不算时钟。没有监听器时，这几次 `Instant` 相减仍然发生，因为 `over_budget` 以外的调度不读这些数；代价是一帧几次减法。

### 3.3 级别

| 级别 | `tracing` | 什么时候打 | 默认 `info` |
|---|---|---|---|
| error | `ERROR` | 这一帧保不住、表面放弃、引擎起不来、脚本 `hold_frame`、JNI/窗口缓冲失败 | 可见 |
| warn | `WARN` | 降级后还能继续：字体单张失败、inspect 失败、溢出、超预算边沿、缺图、present 进入退避 | 可见 |
| info | `INFO` | 生命周期：进程就绪、表面打开或销毁、前后台、`Gpu::Flush`、`MemoryWarning`、字体注册成功、present 从失败恢复 | 可见 |
| debug | `DEBUG` | 状态变化，不按帧重复：`FrameStep` 或 `Gpu` 与上一帧不同、`PresentCached` 与 `Submit` 切换、图片重新请求 | 关闭 |
| verbose | `TRACE` | 帧内可重复多次：pointer、按键、IME、单次 layout pass、shape 未命中 | 关闭 |

空闲时每帧 `Skip` 不打 `DEBUG`。只在 `FrameStep` 或 `Gpu` 和上一帧不同的时候打一条。否则调试级别会换成另一种刷屏。

过滤是 `tracing_subscriber::filter::Targets`。debug 构建缺省 `trace`，release 构建缺省 `info`。一条事件只采用最长的 target 前缀；那条的级别可以比缺省更严，也可以更松。缺省 `info` 再加 `impasto::render=warn` 会丢掉该前缀的 `INFO`。前缀是 `starts_with`，不要求后面是结尾或 `::`。

`set_log_targets` 换整份 `Targets`，放下锁之后调 `callsite::rebuild_interest_cache()`。`register_callsite` 返回 `never` 或 `always`。

### 3.4 target 与调用点

target 是调用点上的静态字符串，不建运行时注册表。下表是要落地的全集。未列出的函数不打。

| target | 级别 | 位置 | 打什么 |
|---|---|---|---|
| `impasto::host` | `INFO` | `macos` `app.rs` 窗口起来之后 | `host ready` |
| `impasto::host` | `DEBUG` | 未识别的文本命令（今 `text command ignored`） | 命令名 |
| `impasto::host` | `INFO` / `ERROR` | `macos` `surface.rs` dump | 写成或失败路径 |
| `impasto::host` | `ERROR` | `android` `host.rs` `Bridge::call`、`detach`、`release_when_dropped`、`configure_window_buffers` | JNI、关机、丢弃线程、缓冲尺寸 |
| `impasto::host` | `TRACE` | `hostshell` 指针、按键、IME 入队处 | `kind`、坐标或键名。默认关 |
| `impasto::render` | `DEBUG` | `on_frame` 看见 `FrameStep` / `Gpu` 变化 | `step`、`gpu`。`scheduler.rs` 仍是纯函数 |
| `impasto::render::surface` | `INFO` | 表面打开成功、平台销毁已确认 | `surface` id |
| `impasto::render::surface` | `WARN` | `Ready::Failed(NotReady)` 且尚未放弃 | 第几次 |
| `impasto::render::surface` | `ERROR` | `OpenRetry::GaveUp` 或其它 `Status` | `status` |
| `impasto::render::present` | `ERROR` | `submit` 返回 `Err` | `status`。同时 `hub.error("engine", …)` 保留给错误页 |
| `impasto::render::present` | `WARN` | `present_failures` 跨过 `PRESENT_BACKOFF_AFTER` | `failures`、`retry_in_ms`。删 `present_logged` |
| `impasto::render::present` | `INFO` | 退避之后第一次 `Shown` | 恢复 |
| `impasto::render::gpu` | `INFO` | `flush_gpu`、`MemoryWarning`、从 `Flush` 回到 `Live` | 原因：`trim`、`memory`、`resume` |
| `impasto::render::gpu` | `DEBUG` | `Invalidate` | 只标记需要重画 |
| `impasto::render::image` | `ERROR` | `images_ready`、`cache_cmds::image_ready` | 沿用缺图、解码失败、像素上限、缓存超预算的一次性开关 |
| `impasto::render::image` | `ERROR` | 解码线程 `send` 失败 | `image decoder stopped` |
| `impasto::script` | `ERROR` | `hold_frame`、启动 eval 失败 | 错误首行。带栈的仍另走 `hub.error("listener")` |
| `impasto::script::console` | 见下表 | QuickJS console 桥 | 原文进 message，不拼前缀 |
| `impasto::layout` | `WARN` | `quickjs` `take_layout_issues` 的循环 | `axis`、`parent`、`parent_size`、`used`、`excess`、`child`。扫描关掉则没有事件 |
| `impasto::net` | `WARN` 或 `ERROR` | `net.rs` 现有错误转发 | 连接失败 `WARN`，协议或取消以外的失败 `ERROR` |
| `impasto::session` | `INFO` | 会话模块注册失败以外的路径今天没有产品日志 | 本轮不新造事件 |

console 编号（`shell.rs` 注释：0 log、1 info、2 warn、3 error、4 debug）：

| console | 事件 |
|---|---|
| `error`（3） | `tracing::error!` |
| `warn`（2） | `tracing::warn!` |
| `log` / `info`（0、1） | `tracing::info!` |
| `debug`（4） | `tracing::debug!` |

字体：启动时缺默认字体或引擎创建失败是 `ERROR`（`prepare_engine`）。运行中单张 `register_font` 失败是 `WARN`（`install_fonts`），帧继续。成功注册打 `INFO`，带 `family`。

`engine` 内部不打。`SubmitStats` 留在 `FrameTiming`，不随日志输出。

### 3.5 中间层与落点

`crates/hostshell/src/log_sub.rs`（新文件）持有过滤器和 sink 列表。调用点看不到 sink。

```rust
pub struct LogRecord {
    pub level: tracing::Level,
    pub target: &'static str,
    pub message: String,
    pub fields: Vec<(&'static str, String)>,
    pub frame: Option<u64>,
}

pub trait LogSink: Send + Sync {
    /// 已经过滤。实现不得再丢级别，也不得阻塞调用线程。
    fn write(&self, record: &LogRecord, line: &str);
}

pub struct ImpastoSubscriber {
    sinks: Vec<Arc<dyn LogSink>>,
}
```

- `register_callsite`：当前 `Targets::would_enable` 为假则 `Interest::never`，否则 `Interest::always`。替换过滤后必须重建兴趣缓存，否则 callsite 仍用上一次的结论。
- `new_span` / `on_record`：按 `span::Id` 记下字段。不打行。
- `event`：抄出当前 span 的 `frame`，`Visit` 成 `LogRecord`，格式化一行，再逐个 `sink.write`。某个 sink 失败不影响其它 sink。
- 行形状，控制台与 xlog 文件用同一串：`ERROR impasto::render::present scene submit failed status=NotReady frame=4264`。字段值含空白时加引号。一条事件一行。
- 订阅者未装上之前（JNI 极早失败）保留 `android::log_line` 直写。安装点放在 native 入口第一件事。此后不再新增绕过订阅者的产品日志。

三个 sink：

| sink | 何时装 | 写到哪 |
|---|---|---|
| `ConsoleSink` | 始终 | macOS：`ERROR`/`WARN` 走 stderr，其余 stdout，行里带级别和 target。Android：`__android_log_write`，tag 是 target，正文只有消息和字段。`ERROR`→6、`WARN`→5、`INFO`→4、`DEBUG`→3、`TRACE`→2。不再过滤 |
| `XlogSink` | 宿主交出非空 `log_dir` | `mars-xlog`。见下 |
| `DevtoolSink` | `DiagnosticsHub` 存在 | `LogEntry`。用 `LogRecord` 的字段，不解析行文本 |

`XlogSink` 的约束：

- Boot 时 `Xlog::init_with_layout`。`name_prefix` 为 `impasto`。`AppenderMode::Async`。级别设成 `Verbose`，避免和 `Targets` 各过一遍。布局用 `Layout::pieces`，只含消息本体和换行，不再套一层 Mars 行（`[level][time][pid, tid]…`）。文件里的一行就是控制台那一行。
- `write` 不得阻塞渲染线程。`mars-xlog` 的异步通道满了会堆住调用方，所以 sink 自己用有界通道 `try_send`。满则丢弃，并在下一次成功写入前打一条 `impasto::host` 的 `WARN`（丢弃条数）。工作线程再调 `Xlog::log`。渲染线程不调 `flush`。
- 不把 `mars-xlog-core` 的压缩、加密或 mmap 类型引进 `uicore` / `engine` / `quickjs`。调用点不引用 `Xlog`。

帧 span 放在 `on_frame` 递增 `frame_number` 之后，级别用 `ERROR`，这样默认 `info` 也会进入，事件才能带上 `frame`。订阅者不打印这个 span。表面还没打开就返回的路径不递增帧号，与今天 `on_frame` 的位置一致，不把递增挪到函数开头。

按需的分段耗时用子 span，级别 `TRACE`，只在该 target 打开时存在。关闭时不进订阅者。这不替代 `FrameTiming`。

### 3.6 帧监听器

```rust
// crates/hostshell/src/timing.rs
pub trait FrameListener {
    fn on_frame(&mut self, timing: &FrameTiming, phases: &session::FramePhases);
}
```

`Runtime` 持有 `Vec<Box<dyn FrameListener>>`。Boot 时：

- 有 devtools 就装 `DevtoolFrames`。样本补上 shape、`picture_rebuilt`、`picture_reused`、`layout_cause`、`gestures`。
- `IMPASTO_INSPECT` 仍由 `shell.rs` 的 `write_inspect` 写，因为这份 `frame.json` 要带层树。不另做只写计时的监听器。
- 始终装 `BudgetEdge`。`over_budget` 从假变真：`tracing::warn!`，字段只有 `session_ms`、`submit_ms`、`present_ms`、`budget_ms`。从真变假：`tracing::info!`，字段 `frames_over`。不打 shape 与 js 分段。那些数在 `frames` 通道和 inspect 文件里。

`FrameListener` 不接图片解码和网络。那两条已经有 hub 的 `NetEntry` 与图片缓存状态。

### 3.7 hub 与面板

`LogEntry` 改为：

```rust
pub struct LogEntry {
    pub seq: u64,
    pub at_ms: f64,
    pub level: tracing::Level,
    pub target: &'static str,
    pub message: String,
    pub fields: Vec<(&'static str, String)>,
    pub frame: Option<u64>,
}
```

`logs` host 方法增 `target`、`fields`、`frame`。`source` 由 target 前缀推导，供旧面板过滤：`impasto::script` → `script`，`impasto::layout` → `layout`，`impasto::net` → `net`，其余 → `host`。级别不再用 console 的 0..4。

`packages/devtools` 的 `LogLevel` 与 `console.tsx` 的 `LEVEL_LABEL` 改成 `ERROR`、`WARN`、`INFO`、`DEBUG`、`TRACE`。过滤增加 target 前缀。错误页仍读 `errors` 通道。

### 3.8 调用形状

`shell.rs` 里 submit 失败，替换 `(runtime.log)` 加手写 `format!`：

```rust
tracing::error!(
    target: "impasto::render::present",
    status = %status,
    "scene submit failed"
);
```

溢出，替换 `log_line()` 字符串：

```rust
tracing::warn!(
    target: "impasto::layout",
    axis = issue.axis.name(),
    parent = issue.parent.to_raw(),
    parent_size = issue.parent_size,
    used = issue.used,
    excess = issue.excess,
    child = issue.worst_child.to_raw(),
    "overflow"
);
```

`LayoutIssue::log_line` 在没有引用后删除。`console_log` 的前缀嗅探删除。

## 4. 分阶段实施

每阶段结束 `cargo test --workspace` 与 clippy 能过。`just check` 在最后一阶段。

### Phase 1：订阅者

- **File: `crates/hostshell/Cargo.toml`** — `tracing = "0.1"`，`tracing-subscriber` 仅 `std`（`Targets`），`mars-xlog`（只给 `XlogSink`）。
- **File: `crates/hostshell/src/log_sub.rs`** — `ImpastoSubscriber`、`LogRecord`、`LogSink`、`ConsoleSink`、`XlogSink`、`DevtoolSink`。缺省 `info`。`set_log_targets` 替换后重建兴趣缓存。
- **File: `crates/hostshell/src/lib.rs`** — 安装函数接受 `log_dir: Option<PathBuf>` 与可选 hub。有目录才构造 `XlogSink`。`LogLevel` 仍留着，供尚未迁移的调用点编译。
- **File: `crates/macos/src/main.rs`**、**`crates/android/src/host.rs`** — native 入口安装订阅者，并传入平台日志目录。Android 在任何 `log_line` 之前；安装前的失败仍走 `log_line`。
- 验证：缺省 `info` 下 `debug` 不通过；`info` 加 `impasto::render=warn` 丢掉该前缀的 `INFO`，别处的 `INFO` 仍在。同一条事件在控制台与 `.xlog` 各一行，正文相同。渲染线程打日志不调用 `Xlog::flush`。

### Phase 2：换掉现有调用点，拆掉计时行

- **File: `crates/hostshell/src/shell.rs`** — 第 2 节列出的 `(runtime.log)` 改为第 3.4 节的事件。删除超预算 `format!`。`on_frame` 在帧号递增后进入 frame span。
- **File: `crates/hostshell/src/timing.rs`** — 增加 `FrameListener`。`FrameTiming` 字段集不删。
- **File: `crates/hostshell/src/shell.rs`** — `BudgetEdge`、`DevtoolFrames`、`InspectFile`。`present_logged` 从 `Runtime` 删除。
- **File: `crates/hostshell/src/shell/session_cmds.rs`** — 去掉对 `present_logged` 的复位；恢复日志由 `Shown` 边沿负责。
- **File: `crates/hostshell/src/present.rs`** — 注释改为「跨过退避阈值打一条」，不再写「只打一次直到成功」以外的一次性标志。
- **File: `crates/hostshell/src/shell/cache_cmds.rs`** — 图片失败改为 `impasto::render::image`。
- **File: `crates/quickjs/Cargo.toml`** — `tracing = "0.1"`。
- **File: `crates/quickjs/src/linked/mod.rs`** — 溢出改为字段事件；`hold_frame` 打 `impasto::script`。
- **File: `crates/uicore/src/tree/debug.rs`** — 删除 `log_line`。`LayoutIssue` 字段保留。
- **File: `crates/macos/src/app.rs`**、**`surface.rs`** — 产品 `eprintln!` / `println!` 与 `MacServices::log` 的过滤移入订阅者。`Services::log` 可改成空实现直到 Phase 3 删签名，避免半截 trait。
- **File: `crates/android/src/host.rs`** — 安装点之后的 `log_line(…)` 改为 `tracing`。
- **File: `crates/hostshell/src/error.rs`** — 本阶段末尾删除 `LogLevel`，若测试桩还引用则保留到那些桩改完。
- 验证：默认 `info` 下连续超预算帧只有进入和恢复两条，不再出现 `frame 4264 gesture … shapes b…`。

### Phase 3：hub 与面板

- **File: `crates/hostshell/src/diagnostics/hub.rs`** — `LogEntry` 换成 3.7 的字段。`log(u8, …)` 删除。`error` 不再双写 `logs`。
- **File: `crates/hostshell/src/diagnostics/module.rs`** — `logs` 增 `target`、`fields`、`frame`。
- **File: `crates/hostshell/src/net.rs`** — 错误改为 `tracing`，由订阅者进 hub。
- **File: `crates/hostshell/src/shell.rs`** — `console_log` 改为按 3.4 的表发事件，删除前缀嗅探。
- **File: `packages/devtools/src/store.ts`**、**`pages/console.tsx`** — 级别与 target 过滤。
- **File: `crates/quickjs` 里 console 测试** — 断言级别名，不再断言 `u8` 编号。

### Phase 4：补上今天沉默的生命周期

只加第 3.4 节里标为今天没有日志的事件：表面打开成功、`flush_gpu`、`MemoryWarning`、`FrameStep` 变化、`TRACE` 输入。不改调度结果。

- **File: `crates/hostshell/src/shell.rs`** — 表面与 GPU。
- **File: `crates/hostshell/src/input.rs`**（若指针在该文件入队；否则是实际入队的文件）— `TRACE`。
- **File: `crates/engine/src/vulkan.rs`** — 不改。hook 仍直接写 logcat。
- **File: `crates/engine/src/player/mod.rs`**、**`crates/uicore/src/tree/record/mod.rs`** — bench 的 `println!` 保持，不是产品日志。

### Phase 5：文档

- **File: `docs/architecture.md`** — 用第 3.2 节的两条路径替换对「帧末把日志写入环形缓冲」的笼统句。写明 `tracing` 与 `FrameListener` 的分工。
- **File: `docs/platform-hosts.md`** — tag 仍是 `impasto`；macOS stderr 只承接 `ERROR`/`WARN`；Android 极早路径仍是 `log_line`。
- **File: `docs/refactor-plan.md`** — 「诊断与日志」里「四级已经落地、超预算帧的计时行是 Warn」整段换成指向本文的现状：计时进行 `frames`，日志是调用点事件。
- **File: `docs/devtools-plan.md`** — console 字段改为 `target` / `fields` / `frame`。
- **File: `AGENTS.md`**、**`README.md`** — 日志过滤写成 `Targets` 与 `set_log_targets`。不写环境变量。

## 5. 架构说明

- **三层**：调用点只产生 `tracing` 事件；`ImpastoSubscriber` 过滤并分发；`ConsoleSink`、`XlogSink`、`DevtoolSink` 只消费 `LogRecord`。新增落点实现 `LogSink`，不加调用点，也不改过滤语法。
- **依赖**：`quickjs`、`hostshell` → `tracing`。`hostshell` 另加 `tracing-subscriber` 的 `Targets`（`std`，不带 `env-filter` / `regex`）和 `mars-xlog`。`uicore`、`engine`、`protocol`、`session` 不新增这两个依赖。`macos` 与 `android` 经 `hostshell` 安装订阅者并传入 `log_dir`。
- **渲染线程**：`event` 里格式化并调用 sink。`ConsoleSink` 与 `DevtoolSink` 同步写。`XlogSink` 只 `try_send`。默认 `info` 下指针与 `Skip` 不进 `event`。`TRACE` 打开后 hub 的锁和 xlog 通道都在这条路径上，是显式打开的成本。
- **不改**：`FrameTiming` 的测量点、`write_inspect` 的文件角色、`DiagError` 的错误页、`IMPASTO_DUMP`、panic hook 先写 hub 再交默认 hook、`scheduler::frame_step` 的纯函数、`vulkan.rs` 的 hook 日志。
- **明确不做**：自建日志 crate、`RateGate`、把 40 个计时字段打成一行、把 `FrameSample` 放进 `tracing` span、让调用点直接调用 `Xlog` 或 logcat、把 `mars-xlog-core` 链进渲染树、在 `uicore` 布局热路径里打日志。

## 6. 文件变更汇总

- `crates/hostshell/Cargo.toml` — `tracing`、仅 `std` 的 `tracing-subscriber`、`mars-xlog`
- `crates/hostshell/src/log_sub.rs` — 订阅者、`LogRecord`、`ConsoleSink`、`XlogSink`、`DevtoolSink`
- `crates/hostshell/src/lib.rs` — 安装入口接受 `log_dir` 与 hub；最终删除 `LogLevel` 与 `Services::log` 的旧签名
- `crates/hostshell/src/error.rs` — 删除 `LogLevel`
- `crates/hostshell/src/shell.rs` — 调用点事件、frame span、删除计时行、装配 `FrameListener`
- `crates/hostshell/src/shell/cache_cmds.rs` — 图片失败事件
- `crates/hostshell/src/shell/session_cmds.rs` — 删除 `present_logged` 复位
- `crates/hostshell/src/present.rs` — 注释与退避边沿对齐
- `crates/hostshell/src/timing.rs` — `FrameListener`
- `crates/hostshell/src/diagnostics/hub.rs` — `LogEntry` 字段；`error` 不再双写
- `crates/hostshell/src/diagnostics/module.rs` — `logs` 增字段
- `crates/hostshell/src/net.rs` — 网络错误改为 `tracing`
- `crates/hostshell/src/input.rs` — `TRACE` 输入（以实际入队文件为准）
- `crates/quickjs/Cargo.toml` — `tracing`
- `crates/quickjs/src/linked/mod.rs` — console、溢出、`hold_frame`
- `crates/uicore/src/tree/debug.rs` — 删除 `log_line`
- `crates/macos/src/main.rs` — 安装订阅者
- `crates/macos/src/app.rs` — 产品日志走订阅者，sink 不再过滤
- `crates/macos/src/surface.rs` — dump 结果改为事件
- `crates/android/src/host.rs` — 安装订阅者；安装点之后离开 `log_line`
- `crates/engine/src/vulkan.rs` — 不改
- `packages/devtools/src/store.ts` — 级别与 `LogLine` 字段
- `packages/devtools/src/pages/console.tsx` — 五级标签与 target 过滤
- `docs/architecture.md` — 两条路径
- `docs/platform-hosts.md` — 宿主写出差异
- `docs/refactor-plan.md` — 替换旧的「日志已分级」段落
- `docs/devtools-plan.md` — console 字段
- `AGENTS.md` — 日志过滤是 `Targets`
- `README.md` — 同上
