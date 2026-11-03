# 搭建应用内 DevTools 体系

本文是 devtools 的完整开发规划：以 Solid 应用（`apps/gallery-solid`）当前的痛点——UI 溢出无处可看、错误只有一行宿主日志、没有网络与帧耗时可见性——为出发点，给框架补一套对标 React Native（LogBox / DevTools bundler /红屏）、Flutter（Flutter DevTools、RenderFlex 溢出条纹）和 H5 vConsole（屏幕角落小按钮、点开面板）的开发者配套。

核心决策一句话：**不做第二套 UI 栈、不做浏览器式外挂面板，devtool 面板本身就是一份用 `packages/solid-ui` 渲染的普通应用层，挂在渲染器根 stack 的最顶层；数据全部来自进程内已有的链路（console、script error、`FrameTiming`、net 线程、Taffy 布局结果），经 `hostshell` 的一个环形缓冲聚合并由 `@impasto/devtools/host` 宿主模块拉取。** 溢出检测放在 `uicore::Tree::layout` 之后的一次受开关控制的盒子扫描里，等价于 Flutter `RenderFlex` 的溢出诊断，但按需付费。

本文遵循仓库文档约定：只写仓库里能核对的文件与符号，未落地条目以本文为准，落地后按 `docs/refactor-plan.md` 的规则迁走。

---

## 破坏性变更说明

对外部消费者几乎无破坏。两处内部构造点需要同步修改，一处 trait 追加：

1. `hostshell::Boot` 增加公开字段（`devtools` 等）。`crates/macos/src/app.rs` 与 `crates/android/src/host.rs` 里两处 `Boot { .. }` 字面量必须补上新字段。仓库内只有这两处构造点（`rg "Boot \{"` 可核对），不存在外部消费者。
2. `session::Session` trait 追加带默认实现的方法 `take_script_error`，非破坏；仅 `crates/quickjs` 覆写。
3. `packages/solid-renderer` 的 `boot(component)` 追加可选第二参数并新增导出 `registerTopLayer`，纯增量；现有调用 `boot(() => <App/>)` 不需要改。

迁移步骤（仓库内）：

1. `crates/macos/src/app.rs`：`Boot` 字面量加 `devtools: devtools_enabled(), source_map: …`。
2. `crates/android/src/host.rs`：同上（用 `BuildConfig.DEBUG` 对应的运行时值）。
3. 无其它外部迁移。

---

## 可行性评估

全链路已在进程内，不需要新的进程间通道：console 与 script error 已经从 glue.c 经 `fns.log` 进入 `hostshell`（`crates/hostshell/src/shell.rs` 的 `EngineSlot::log_hook`，41-52 行）；帧计时已经是完整的 `FrameTiming`（`crates/hostshell/src/timing.rs`）；HTTP 与 WebSocket 全部经过 `hostshell` 的 net 线程（`crates/hostshell/src/net.rs` 的 `serve`，36 行），包一层记录器即可；布局结果已经在 `Tree` 上（`write_box_from_taffy`，`crates/uicore/src/tree/layout.rs` 1132 行），扫描只是读。宿主模块 ABI（`session::HostModule` + `PluginValue`）已经让 `@impasto/clipboard` 这类包跑通「Rust 方法表 → `JS_NewCModule` 物化 → `host.gen.ts` 生成」，devtool 复用同一通道，不发明第二种封送。面板渲染用的 `stack`、`ScrollArea`、`Tabs`、`Dialog` 全在 `packages/solid-ui`，渲染器根 stack（`packages/solid-renderer/runtime/index.ts` 的 `boot`，100 行）已有 overlay 子层可扩展。sourcemap 产物已经由 `apps/gallery-solid/build.mjs` 生成（`sourcemap: true`），只缺消费者。`Engine` 的 `!Send` 与渲染线程约束不被触碰：所有新数据要么在渲染线程聚合（帧计时、布局扫描），要么经 `Mutex` 环形缓冲（net 线程、日志）。唯一的外部依赖候选（VLQ sourcemap 解码）约两百行 TS 自己写在 devtools 包内，不引入 npm 运行时依赖。结论：完全可行。

---

## 现状盘点

计划触碰的全部现有 API 与调用点。按层列出：

- `crates/quickjs/src/glue.c`
  - `bind_console`（3998 行）：`console.log/info/warn/error` → 数字 level 0/1/2/3 → `fns.log`。没有 `debug`、`table`、`time`。
  - `write_exception`（429 行）：异常文本 + `stack` 属性拼进 512 字节缓冲。
  - `listener_failed`（452 行）：`report`/`freeze` 双策略；`report` 打 level 3 日志并继续派发。
  - `setListenerErrorPolicy`（2198 行附近）：`packages/solid-ui/src/errors.ts` 第 4 行设为 `report`。
  - `bind_core`：约 150 个 `core.*` 函数。**没有 `log`**，所以 `guard.ts` 的 `log()` 与 `reportError` 在真机上是 no-op（mock host 有 `log`，测试全绿掩盖了这条断链）。
  - `settle()`（`runtime_host.inc` 269-273 行）：promise 回填路径 `JS_GetException` 后直接 `JS_FreeValue`，未处理拒绝被静默吞掉；没有 `JS_SetHostPromiseRejectionChecker`。
- `crates/quickjs/src/linked/mod.rs`
  - `hold_frame`（482 行）：freeze 策略下记 `script_error`、`suppress_demand`、画面冻结。
  - `take_script_error`（508 行）：错误槽位，**hostshell 从不消费**。
  - `frame`（784-998 行）：完整帧序，产出 `FrameOutput.phases`。
- `crates/session/src/lib.rs`
  - `Session` trait（441 行）、`FrameOutput`（402 行）、`FramePhases`/`ScriptPhases`/`FrameTrace`（357-398 行）。
- `crates/session/src/modules.rs`
  - `HostModule { name, specifier, methods }`、`HostMethod::sync/promise`、`ModuleRegistry::by_specifier`。devtool 模块按 `clipboard.rs` 的形状新增。
- `crates/hostshell/src/lib.rs`
  - `Boot`（96 行）：`modules`、`http`、`dump_list`、`services`。devtool 开关与 hub 挂这里。
  - `Services::log`（72 行）：唯一日志 sink。
- `crates/hostshell/src/shell.rs`
  - `EngineSlot::log_hook`（41-52 行）：数字 level → `LogLevel`，0/1 都并成 `Info`。
  - 渲染线程 `catch_unwind`（598 行）+ panic streak 3 次退出；无 `panic::set_hook`，panic 正文在 Android 上进不了 logcat。
  - 超预算帧的 Warn 计时行（760-807 行）；`layer_tree_line`（533 行，`Boot.dump_list`）。
- `crates/hostshell/src/timing.rs`
  - `FrameTiming`（4 行起）：18 个字段，含缓存计数。直接进帧环形缓冲。
- `crates/hostshell/src/net.rs`
  - `serve`（36 行）：HTTP/HttpStream/Socket 全在这条线程上，插入记录点。
- `crates/hostshell/src/http.rs`
  - `OpenWireClient`：20s 超时、5 跳重定向、4MiB body 上限。包装它。
- `crates/uicore/src/tree/layout.rs`
  - `Tree::layout`（854 行）调 `compute_layout_with_measure`；`write_box_from_taffy`（1132 行）把 Taffy 结果抄进 `node.x/y/w/h`（父相对）。**没有任何溢出检测**；盒子几何不做像素取整（仅滚动 `settle_pixels`）。
  - `take_layout_trace`（59 行）、`layout_timing`（1067 行）：已有 per-frame 计数。
- `crates/macos/src/app.rs`
  - `IMPASTO_LOG` 读取（813 行）、`IMPASTO_DUMP`（935 行）、`MacServices::log`（862 行）。
- `apps/gallery-solid/build.mjs`
  - `sourcemap: true`：`dist/quotes.js.map` 已产出，无人消费。
- `packages/solid-renderer/runtime/index.ts`
  - `boot(component)`（100 行）：建 `stack{column content, stack overlay}` 后 `render`。
- `packages/solid-ui/src/overlay/registry.ts`
  - `registerOverlay`/`closeTopModal`：模态栈。devtool 面板不进这张栈（它不是模态，是顶层调试层），但错误横幅可以复用 Toast 形状。
- `packages/solid-ui/src/errors.tsx` / `packages/solid-router/src/view.tsx`（184 行）
  - `Errored` 边界 + `ErrorPage`。路由级渲染错误已有 UI；**监听器错误、未处理拒绝、宿主侧错误没有 UI**。
- `docs/refactor-plan.md` 109 行
  - 「常驻 devtool 窗口或 devtools 协议服务」暂缓条目。本计划落地的是**应用内面板**，不是第二窗口也不是协议服务；该条目改写为指向本文。

---

## 设计

### 目标

1. **屏幕上的小按钮**：devtool 开启时，视口角落一枚可拖动的悬浮按钮，带未读错误角标；点开全屏面板，覆盖 Console、错误、网络、性能、检查器、布局、信息七个页面。
2. **错误可见**：JS 异常（监听器、未处理拒绝、启动失败）带堆栈进面板；新错误在按钮上出角标、在屏幕顶部出一条可点开的横幅；堆栈经 sourcemap 映射回 TSX。
3. **溢出可见**：`Row`/`Column` 主轴溢出在布局后被扫描出来，进面板的布局页与 Warn 日志，可在检查器里高亮父子盒子——对齐 Flutter `RenderFlex` 条纹的语义，但不往 `DrawList` 里塞调试命令。
4. **日志与网络**：`console.*` 分级入环形缓冲，面板可过滤、搜索；HTTP/流式/WS 请求有列表和详情（含脱敏头、body 预览、耗时）。
5. **帧可见**：每帧 `FrameTiming` + `FramePhases` 入环形缓冲，面板画最近 120 帧的条形图与相位分解。
6. **环境开关**：`IMPASTO_DEVTOOLS` 控制，关闭时代价严格为零（不扫描、不记录、不注册 UI）。
7. **可扩展**：Rust 侧通道与 JS 侧页面都有注册口子，后续（存储、权限、路由、Kotlin）加页面不改面板骨架。

### 非目标

- 不做浏览器式外挂 devtools 面板、不做 CDP/协议服务（refactor-plan 的暂缓条目维持，另写触发条件）。
- 不做第二窗口或第二 UI 栈。面板用现有控件渲染，自己吃自己的狗粮。
- 不做时间旅行（signal 历史回放）、不做热重载。触发条件写在扩展一节。
- 不在 `uicore` 里画调试图形。高亮框、溢出条纹都由 devtool 的 overlay 盒子画，`DrawList` 不含任何仅调试用的命令。
- 不给 `Engine`/`Typography` 加诊断面。缓存计数已经随 `SubmitStats`/`FrameTiming` 走。

### 对标

| 能力 | React Native | Flutter | vConsole | Impasto 本方案 |
|---|---|---|---|---|
| 入口 | 摇一摇 / `j` 键打开 dev menu | IDE 外挂面板 | 屏幕角落按钮 | 屏幕角落按钮（可拖动） |
| JS 错误 | LogBox + 红屏（dev） | ErrorWidget（框架树内） | 面板 + onError 钩子 | 按钮角标 + 顶部横幅 + 面板详情页 |
| 溢出 | 无内建（Flexbox 静默） | RenderFlex 黄黑条纹 + 一行日志 | 无 | 布局页列表 + 检查器高亮父子盒 + Warn 日志 |
| 网络 | flipper/第三方 | DevTools Network | 面板（劫持 XHR） | net 线程记录器（native 层，含 fetch/fetchStream/WS） |
| 日志 | metro 控制台 | `debugPrint` → IDE | 面板 | console.* → hub 环形缓冲 → 面板 |
| 帧性能 | FPS overlay | Performance 视图 | 无 | 帧条形图 + `FramePhases` 分解 |
| 树检查 | Element inspector | Widget inspector | DOM 树（无） | `debugSnapshot` + 探针选点 |
| sourcemap | metro 自动 | n/a | n/a | 面板内最小 VLQ 消费者 |

差异的根本原因：RN/vConsole 劫持 JS 运行时的全局对象，Flutter 依赖 VM 服务协议外接 IDE。Impasto 的 JS、树、引擎在同一个进程同一条渲染线程上，不需要劫持也不需要协议——数据就地可读，唯一要做的是把它聚合起来并给出一个不干扰应用的呈现层。

### 总体架构

```text
┌─ 数据生产 ────────────────────────────────────────────────────────────┐
│ uicore::Tree          glue.c / quickjs        hostshell net 线程       │
│  layout 盒子扫描       console.* / 监听器错误    HTTP / stream / WS     │
│  debugSnapshot/probe   script_error / 拒绝检查   提交/上屏失败(渲染线程) │
└──────┬──────────────────────┬──────────────────────┬──────────────────┘
       │ take_layout_issues   │ fns.log / take_      │ Mutex 环形缓冲
       │ (渲染线程，帧末)      │ script_error         │ (net 线程写)
       ▼                      ▼                      ▼
┌─ 聚合：hostshell::diagnostics::DiagnosticsHub（Arc，进程一份）─────────┐
│  logs[] errors[] network[] frames[]  + revision 计数 + 环形上限        │
└──────┬───────────────────────────────────────────────┬───────────────┘
       │ 消费一：Services::log（现状不变，IMPASTO_LOG 过滤）│ 消费二：
       ▼                                               ▼
   stderr / logcat                    @impasto/devtools/host 宿主模块
                                      （HostModule::sync，PluginValue 快照，
                                        revision 门控的游标拉取）
                                                       │
                                                       ▼
                                       packages/devtools（@impasto/devtools）
                                        store（signal）→ DevtoolsLayer
                                        （registerTopLayer 挂渲染器根 stack
                                        最顶层，solid-ui 控件组成）
                                                       │
                                                       ▼
                                       同一棵 uicore::Tree → 同一 Engine
```

三条铁律：

1. **单线程归属不变**。hub 用 `Mutex` 只因为 net 线程和渲染线程都写；模块方法是 `Sync` 调用，在渲染线程（JS 线程）上锁一下、克隆快照、立刻放锁，无跨线程对象。
2. **面板是普通 UI**。devtool 组件只用 `stack`/`column`/`scroll`/`decorated`/`pointer`/`Text`，不新增布局原语，不做第二套命中。
3. **关闭即零成本**。`IMPASTO_DEVTOOLS=0` 时：`Boot` 不构造 hub、不包 http 记录器、不注册模块；session 不开布局扫描；`installDevtools()` 读到 `enabled()==false` 直接返回禁用句柄。

### 数据面：`DiagnosticsHub`

新模块 `crates/hostshell/src/diagnostics/`。核心是一个带修订号的多通道环形缓冲：

```rust
// crates/hostshell/src/diagnostics/hub.rs
//! Developer diagnostics. Ring buffers with a monotonic revision per
//! channel. Producers are the render thread, the net thread, and the
//! log hook; the only reader is the devtools host module (render thread).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// One console line or host log line.
#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    pub seq: u64,
    pub at_ms: f64,
    pub level: u8, // 0 log, 1 info, 2 warn, 3 error, 4 debug
    pub source: LogSource,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogSource {
    Script,   // console.* and script exceptions
    Layout,   // uicore layout issues
    Host,     // hostshell: submit/present/panic/boot
    Net,      // network failures surfaced by the net thread
}

/// One surfaced failure with a stack, if any.
#[derive(Clone, Debug, PartialEq)]
pub struct DiagError {
    pub seq: u64,
    pub at_ms: f64,
    pub source: &'static str, // "listener" | "rejection" | "boot" | "engine" | "host"
    pub message: String,
    pub stack: Option<String>,
    pub frame: Option<u64>,
}

/// One HTTP call (fetch or fetchStream) or socket lifecycle.
#[derive(Clone, Debug)]
pub struct NetEntry {
    pub seq: u64,
    pub at_ms: f64,
    pub kind: NetKind,
    pub token: u64,
    pub method: String,
    pub url: String,
    pub request_headers: Vec<(String, String)>,
    pub request_body: Option<Vec<u8>>, // preview, capped
    pub status: Option<u16>,
    pub response_headers: Vec<(String, String)>,
    pub response_bytes: Option<usize>,
    pub response_preview: Option<Vec<u8>>, // preview, capped
    pub error: Option<String>,
    pub done_ms: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetKind {
    Http,
    Stream,
    Socket,
}

/// Per-frame timing sample, a trimmed FrameTiming plus phase totals.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameSample {
    pub number: u64,
    pub at_ms: f64,
    pub session_ms: f64,
    pub gesture_ms: f64,
    pub script_ms: f64,
    pub layout_ms: f64,
    pub record_ms: f64,
    pub submit_ms: f64,
    pub present_ms: f64,
    pub layers: u32,
    pub paragraphs: u32,
    pub pictures: u32,
    pub textures: u32,
    pub over_budget: bool,
}

pub(crate) struct Ring<T> {
    buf: VecDeque<T>,
    cap: usize,
    seq: u64,
}

impl<T> Ring<T> {
    pub(crate) fn new(cap: usize) -> Self {
        Self { buf: VecDeque::new(), cap, seq: 0 }
    }

    pub(crate) fn push(&mut self, entry: impl FnOnce(u64) -> T) {
        self.seq += 1;
        if self.buf.len() == self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(entry(self.seq));
    }

    pub(crate) fn revision(&self) -> u64 {
        self.seq
    }

    /// Entries with `seq > since`, plus the current revision.
    pub(crate) fn tail(&self, since: u64) -> (u64, Vec<&T>) {
        let out = self.buf.iter().filter(|e| seq_of(e) > since).collect();
        (self.seq, out)
    }
}
```

hub 本体与通道上限：

```rust
// crates/hostshell/src/diagnostics/hub.rs（续）
pub struct DiagnosticsHub {
    pub(crate) logs: Mutex<Ring<LogEntry>>,
    pub(crate) errors: Mutex<Ring<DiagError>>,
    pub(crate) network: Mutex<Ring<NetEntry>>,
    pub(crate) frames: Mutex<Ring<FrameSample>>,
    /// Bumped by any channel; lets the panel skip the whole pull.
    pub(crate) revision: AtomicU64,
    /// Runtime knobs shared with uicore's scan and the session bindings.
    pub(crate) flags: Mutex<DevtoolsFlags>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DevtoolsFlags {
    pub layout_scan: bool,
    pub pointer_trace: bool,
    pub suppress_dispatch: bool,
}

pub const LOG_CAP: usize = 400;
pub const ERROR_CAP: usize = 100;
pub const NET_CAP: usize = 150;
pub const FRAME_CAP: usize = 240;
pub const PREVIEW_CAP: usize = 2048;

impl DiagnosticsHub {
    pub fn new() -> Self { /* 四个 Ring 按 cap 建好，revision 置 1 */ }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    fn bump(&self) {
        self.revision.fetch_add(1, Ordering::Release);
    }

    pub fn log(&self, level: u8, source: LogSource, text: &str) {
        let at = crate::net::mono_ms();
        self.logs.lock().unwrap_or_else(|p| p.into_inner())
            .push(|seq| LogEntry { seq, at_ms: at, level, source, text: text.to_owned() });
        self.bump();
    }

    pub fn error(&self, source: &'static str, message: &str, stack: Option<&str>, frame: Option<u64>) {
        let at = crate::net::mono_ms();
        self.errors.lock().unwrap_or_else(|p| p.into_inner())
            .push(|seq| DiagError {
                seq, at_ms: at, source: source.into(),
                message: message.to_owned(),
                stack: stack.map(str::to_owned), frame,
            });
        // Errors also land in the log channel so the console page shows them
        // without a second subscription.
        self.log(3, LogSource::Host, message);
        self.bump();
    }
}
```

要点：

- `seq` 是跨通道递增的单调序号（每通道独立计数、hub 级 `revision` 汇总），JS 侧游标拉取 `since` 之后的新条目，重复拉取幂等。
- 上限刻意小：日志 400 条、网络 150 条、帧 240 条（4 秒的 60fps），整个 hub 常驻内存 < 2MiB。
- `mono_ms()` 复用 `crates/hostshell/src/net.rs` 321 行的现有时钟，所有时间戳同一时基。

### 记录点接线

**日志**（渲染线程 + net 线程都可能来）：`EngineSlot::log_hook`（`shell.rs` 41-52 行）在映射完 `LogLevel` 后先写 hub 再走 `Services::log`。level 映射同时补一格：数字 4（`console.debug` 与 `core.log("debug")`）→ `LogLevel::Debug`。

```rust
// crates/hostshell/src/shell.rs（改 log_hook）
pub fn log_hook(&self) -> Box<dyn Fn(u8, &str)> {
    let services = self.services.clone();          // 已有
    let hub = self.hub.clone();                    // 新增：Option<Arc<DiagnosticsHub>>
    Box::new(move |level, text| {
        if let Some(hub) = hub.as_ref() {
            hub.log(level, LogSource::Script, text);
        }
        let parsed = match level {
            3 => LogLevel::Error,
            2 => LogLevel::Warn,
            4 => LogLevel::Debug,
            _ => LogLevel::Info,
        };
        services.log(parsed, text);
    })
}
```

宿主自身的关键事件（提交失败、present 退避、panic、图片缺源）本来就要写 `Services::log`；这些点改成先 `hub.error(...)` 再走原路径（`hub.error` 自带一份日志落点，避免双写）。涉及 `shell.rs` 的提交失败分支、present 退避分支（760-807 行附近）与 panic 分支（598-645 行）。

**panic**：`hostshell::start()` 在拉起线程前安装一次 `std::panic::set_hook`（仅当 hub 存在）：

```rust
// crates/hostshell/src/shell.rs（start 内，线程 spawn 之前）
if let Some(hub) = boot.devtools.as_ref().cloned() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let text = format!("panic: {info}");
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        hub.error("host", &text, Some(&backtrace), None);
        default(info);
    }));
}
```

Android 上这让 panic 正文进 hub → `__android_log_write`（经 `Services::log`），不再只有摘要行。macOS 保持 stderr 行为（默认 hook 前置）。

**script error 消费**：`Session` trait 追加：

```rust
// crates/session/src/lib.rs（Session trait 内）
/// The most recent script failure, if the session recorded one and nobody
/// took it yet. The host consumes this for diagnostics each frame.
fn take_script_error(&mut self) -> Option<String> {
    None
}
```

`crates/quickjs/src/linked/mod.rs` 的 `take_script_error`（508 行）改为 trait 实现（签名相同，`Option<String>`）。`shell.rs` 的 `on_frame` 在 `session.frame` 返回后取一次，有值就 `hub.error("listener", ...)`，并保持现有日志路径不变。这补上了「错误槽位无人消费」的断链。

**网络**：`Boot.http` 的类型是 `Option<Arc<dyn HttpClient>>`。devtool 开启时在 `start()` 里包一层，不动 `net.rs` 的骨架：

```rust
// crates/hostshell/src/diagnostics/recorder.rs
//! Wraps the host HTTP client so the devtools network page sees every call.

use std::sync::Arc;
use session::{HttpPiece, HttpClient, HttpError, HttpRequest, HttpResponse};
use super::hub::{NetEntry, NetKind, PREVIEW_CAP, DiagnosticsHub};

pub struct RecordingHttpClient {
    inner: Arc<dyn HttpClient>,
    hub: Arc<DiagnosticsHub>,
}

impl RecordingHttpClient {
    pub fn wrap(inner: Arc<dyn HttpClient>, hub: Arc<DiagnosticsHub>) -> Arc<dyn HttpClient> {
        Arc::new(Self { inner, hub })
    }
}

fn preview(body: &[u8]) -> Vec<u8> {
    body.iter().copied().take(PREVIEW_CAP).collect()
}

fn redact(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            let masked = matches!(
                name.to_ascii_lowercase().as_str(),
                "authorization" | "cookie" | "set-cookie" | "proxy-authorization"
            );
            (name.clone(), if masked { "***".to_owned() } else { value.clone() })
        })
        .collect()
}

impl HttpClient for RecordingHttpClient {
    fn perform(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let at = crate::net::mono_ms();
        let start = std::time::Instant::now();
        let result = self.inner.perform(request);
        let entry = NetEntry {
            seq: 0, // Ring::push assigns
            at_ms: at,
            kind: NetKind::Http,
            token: request.token,
            method: request.method.clone(),
            url: request.url.clone(),
            request_headers: redact(&request.headers),
            request_body: request.body.is_empty().then(|| preview(&request.body)).filter(|b| !b.is_empty()),
            status: result.as_ref().ok().map(|r| r.status),
            response_headers: result.as_ref().map(|r| redact(&r.headers)).unwrap_or_default(),
            response_bytes: result.as_ref().ok().map(|r| r.body.len()),
            response_preview: result.as_ref().ok().map(|r| preview(&r.body)),
            error: result.as_ref().err().map(|e| e.message.clone()),
            done_ms: Some(crate::net::mono_ms()),
        };
        self.hub.record_net(entry);
        result
    }

    fn stream(
        &self,
        request: &HttpRequest,
        sink: &mut dyn FnMut(HttpPiece<'_>) -> bool,
    ) -> Result<(), HttpError> {
        // Same shape as perform: record head + chunk totals, preview the
        // first body bytes, cap the preview at PREVIEW_CAP.
        let mut entry = NetEntry {
            kind: NetKind::Stream,
            ../* same field init from request */
        };
        let mut delivered = 0usize;
        let mut first: Vec<u8> = Vec::new();
        let outcome = self.inner.stream(request, &mut |piece| match piece {
            HttpPiece::Head(status, headers) => {
                entry.status = Some(status);
                entry.response_headers = redact(headers);
                true
            }
            HttpPiece::Chunk(bytes, done) => {
                delivered += bytes.len();
                if first.len() < PREVIEW_CAP {
                    first.extend_from_slice(&bytes[..(bytes.len().min(PREVIEW_CAP - first.len()))]);
                }
                if done {
                    entry.response_bytes = Some(delivered);
                    entry.response_preview = Some(first.clone());
                }
                sink(HttpPiece::Chunk(bytes, done))
            }
        });
        if let Some(error) = outcome.as_ref().err() {
            entry.error = Some(error.message.clone());
        }
        entry.done_ms = Some(crate::net::mono_ms());
        self.hub.record_net(entry);
        outcome
    }
}
```

WebSocket：`net.rs` 的 `post_socket`（160 行）加一个可选 hub 参数，`open/message/close/error` 各记一条 `NetKind::Socket` 的 `NetEntry`（method 留空、url 取打开时的地址、`token` 用 socket id）。改动集中在 `serve` 与 `socket_loop` 的入参，不新增线程。

**帧采样**：`shell.rs` 的 `on_frame` 在 `end_frame` 之后（`FrameTiming` 已齐，734 行附近组装处）追加：

```rust
if let Some(hub) = hub.as_ref() {
    hub.push_frame(FrameSample {
        number: timing.number,
        at_ms: crate::net::mono_ms(),
        session_ms: timing.session_ms,
        gesture_ms: timing.gesture_ms,
        script_ms: timing.script_ms,
        layout_ms: timing.layout_ms,
        record_ms: timing.record_ms,
        submit_ms: timing.submit_ms,
        present_ms: timing.present_ms,
        layers: timing.layer_count,
        paragraphs: timing.paragraphs,
        pictures: timing.pictures,
        textures: timing.textures,
        over_budget: timing.over_budget(interval_ms),
    });
}
```

### 传输面：`@impasto/devtools/host` 宿主模块

与 `@impasto/clipboard` 同一形状。全部 `Sync` 方法（渲染线程上内联执行，克隆即回），游标拉取：

```rust
// crates/hostshell/src/diagnostics/module.rs
//! The `@impasto/devtools/host` host module. Sync reads over the hub.

use std::sync::Arc;
use session::{HostMethod, HostModule, PluginValue};

use super::hub::DiagnosticsHub;

fn map(pairs: Vec<(&str, PluginValue)>) -> PluginValue {
    PluginValue::Map(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

fn text_preview(bytes: &[u8]) -> PluginValue {
    match String::from_utf8(bytes.to_vec()) {
        Ok(text) => PluginValue::Str(text),
        Err(_) => PluginValue::Str(format!("<{} bytes>", bytes.len())),
    }
}

pub fn module(hub: Arc<DiagnosticsHub>, context: DevtoolsContext) -> HostModule {
    HostModule {
        name: "Devtools",
        specifier: "@impasto/devtools/host",
        methods: vec![
            HostMethod::sync("enabled", {
                let context = context.clone();
                move |_| Ok(PluginValue::Bool(context.enabled))
            }),
            HostMethod::sync("revision", {
                let hub = Arc::clone(&hub);
                move |_| Ok(PluginValue::F64(hub.revision() as f64))
            }),
            HostMethod::sync("logs", { /* since -> {revision, entries:[{seq,at,level,source,text}]} */ }),
            HostMethod::sync("errors", { /* since -> {revision, entries:[{seq,at,source,message,stack,frame}]} */ }),
            HostMethod::sync("network", { /* since -> {revision, entries:[...]} */ }),
            HostMethod::sync("frames", { /* since -> {revision, entries:[...]} */ }),
            HostMethod::sync("memory", { /* -> {mallocBytes, memoryUsedCount, ...} quickjs usage */ }),
            HostMethod::sync("platform", { /* -> {os, osVersion, scale, viewport:{w,h,dpr}, insets, appState} */ }),
            HostMethod::sync("versions", { /* -> {hostshell, enginePin, quickjsPin} */ }),
            HostMethod::sync("sourceMap", { /* -> string|null, reads <bundle>.map next to Boot's js path */ }),
        ],
    }
}

/// What the host knows and the hub does not.
#[derive(Clone)]
pub struct DevtoolsContext {
    pub enabled: bool,
    pub source_map: Option<std::path::PathBuf>,
}
```

`host-stubs`（`crates/hostshell/src/host_stubs.rs` + `bin/host-stubs.rs`）把 `packages/devtools` 加进一方包枚举，`just gen` 产出 `packages/devtools/src/host.gen.ts`，hostshell 的防漂移测试同步扩展。方法名和 kind 由生成物承载，JS 侧不手写。

拉取节奏：面板打开时 `store` 在 `onFrame` 里每 4 帧做一次 `revision()` 比对，变了才逐通道 `logs(since)` 增量拉。面板关闭时只保留角标计数（`errors` 通道单独比对，同样 revision 门控），开销可忽略。

### uicore：布局扫描与检查器数据

**溢出扫描**（v1 只查 flex 主轴，对齐 Flutter `RenderFlex` 的诊断范围）：

```rust
// crates/uicore/src/tree/debug.rs
//! Developer-only layout diagnostics. Everything here is gated behind
//! Tree::debug_flags and costs nothing when the flags are off.

/// One overflow found by the post-layout scan.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutIssue {
    pub parent: NodeId,
    pub axis: Axis,          // Row -> Horizontal, Column -> Vertical
    pub parent_size: f32,    // content-box size on that axis
    pub used: f32,           // extent the children actually occupy
    pub excess: f32,         // used - parent_size, > 0 means overflow
    pub worst_child: NodeId, // the child that extends furthest
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Stable identity for cross-frame dedup: parent slot + axis.
pub(crate) fn signature(issue: &LayoutIssue) -> (u32, Axis) {
    (issue.parent.slot(), issue.axis)
}
```

扫描入口在 `Tree::layout` 的最后一次布局回写之后（`layout.rs` 854 行函数体末尾）：

```rust
// crates/uicore/src/tree/layout.rs（layout 末尾）
if self.debug.scan_overflow {
    self.scan_overflow();
}
```

```rust
// crates/uicore/src/tree/debug.rs（续）
impl Tree {
    /// Report Row/Column children that exceed the container on the main
    /// axis. Scroll viewports, Text with Visible overflow and clip-styled
    /// nodes are intentional overflow and are skipped.
    pub(crate) fn scan_overflow(&mut self) {
        const EPSILON: f32 = 0.5; // half a point, matches hit-test slop
        let mut found: Vec<LayoutIssue> = Vec::new();
        // Walk every node with a computed box. Payload kinds follow
        // tree/node.rs; only Row and Column own main-axis layout.
        for id in self.node_ids() {
            let Some(payload) = self.payload(id) else { continue };
            let axis = match payload {
                Payload::Row(..) => Axis::Horizontal,
                Payload::Column(..) => Axis::Vertical,
                _ => continue,
            };
            let parent = self.border_box(id);
            let limit = match axis {
                Axis::Horizontal => parent.width(),
                Axis::Vertical => parent.height(),
            };
            let mut used = 0.0f32;
            let mut worst = id;
            let mut worst_edge = 0.0f32;
            for child in self.children(id) {
                let box_ = self.border_box(child);
                let edge = match axis {
                    Axis::Horizontal => box_.max_x() - parent.min_x(),
                    Axis::Vertical => box_.max_y() - parent.min_y(),
                };
                used = used.max(edge);
                if edge > worst_edge {
                    worst_edge = edge;
                    worst = child;
                }
            }
            let excess = used - limit;
            if excess > EPSILON {
                found.push(LayoutIssue {
                    parent: id,
                    axis,
                    parent_size: limit,
                    used,
                    excess,
                    worst_child: worst,
                });
            }
        }
        // Keep the newest report and refresh the seen set so the session
        // can log only new signatures.
        let fresh: Vec<_> = found
            .into_iter()
            .filter(|issue| !self.debug.seen_issues.contains(&signature(issue)))
            .collect();
        self.debug.seen_issues = found.iter().map(signature).collect();
        self.debug.issues = found;
        if !fresh.is_empty() {
            self.debug.new_issues.extend(fresh);
        }
    }

    /// Take issues that appeared since the last take. Empty when the scan
    /// is off or nothing new happened.
    pub fn take_layout_issues(&mut self) -> Vec<LayoutIssue> {
        std::mem::take(&mut self.debug.new_issues)
    }

    /// The full current report, for the devtools layout page.
    pub fn layout_issues(&self) -> &[LayoutIssue] {
        &self.debug.issues
    }
}
```

刻意排除的三类「合法溢出」：

1. `Scroll` 的子树（max_content 轨道，内容按完整尺寸布局，`docs/architecture.md`「UI 核心」节明文允许高出视口）。
2. `TextOverflow::Visible` 的 `Text`（不换行时一行可以宽过约束，`TextStyle` 契约）。
3. `Clip` 的子树（裁剪本来就允许内容超出，超出部分被裁掉不是缺陷）。

实现方式是扫描时沿父链带一个 `in_tolerant` 标志：进入这三类节点置位，其子树全部跳过。扫描量 O(n)，只在有布局的帧跑；实测 gallery 全树几百节点，单次 < 0.1ms。

**去重与上报**：`seen_issues` 按 `(parent.slot, axis)` 记住上一帧的报告，只有新签名进 `new_issues`。会话在帧末 `take_layout_issues()`，非空时逐条 `console` 级 2（Warn）写日志（`[layout] row overflow 12.5pt: node 42 child 43`），hostshell 的 log_hook 把它带进 hub 的 `Layout` 来源。溢出消失时不发清除事件——布局页每帧读 `layout_issues()` 全量快照，页面自己反映现状。

**检查器数据**：三个只读绑定，全部返回 JSON 字符串（`quickjs` 内建 `JSON.parse`，避免在 C 里拼 PluginValue）：

```rust
// crates/uicore/src/tree/debug.rs（续）
impl Tree {
    /// The render tree with scene rects, for the inspector page. Nodes the
    /// session marked as debug-owned (the devtool layer itself) are cut.
    pub fn debug_snapshot(&self) -> String { /* json: {id,kind,x,y,w,h,flags,text?}[] tree */ }

    /// Hit-test chain for a scene point, root to leaf, skipping debug-owned
    /// subtrees. Same geometry the pointer path uses (last frame).
    pub fn debug_probe(&self, x: f32, y: f32) -> String { /* json chain */ }

    /// Mark a subtree as debug-owned: excluded from snapshots and probes.
    pub fn set_debug_owned(&mut self, id: NodeId, on: bool) { /* flag bit on the slot */ }
}
```

`debug_snapshot` 的节点字段：`id`（u64 handle）、`kind`（payload 名，`row`/`column`/`text`/…）、场景矩形 `x,y,w,h`（复用 `Tree::rect` 的父链累加）、`focusable`、`listeners`（该节点登记过的回调种类列表）、文本节点的当前字符串（截 80 字符）。

**指针追踪**：`Tree` 已有完整指针路径。加两个方法：

```rust
// crates/uicore/src/tree/debug.rs（续）
impl Tree {
    /// Record pointer events for debugPick. When suppress is set, listeners
    /// do not fire; the app stays inert while the inspector selects.
    pub fn set_pointer_trace(&mut self, on: bool, suppress: bool) { /* set flags */ }

    /// The last pointer event as {x, y, phase} in scene points.
    pub fn debug_last_pointer(&self) -> String { /* json */ }
}
```

`Tree::pointer`（`tree/gesture.rs`）入口处记 `debug.last = (x, y, phase)`；`suppress` 为真时直接在派发前返回，命中与手势状态机照常推进（这样「抬起」也能被观察到）。探针选点的完整流程见检查器页。

**会话绑定**（glue.c 的 `bind_core` 新增，C 只在 `crates/quickjs/src/glue.c`）：

| `core.*` | 参数 | 返回 | 说明 |
|---|---|---|---|
| `debugLayoutIssues()` | — | string(JSON) | 当前全量 + 新增标记 |
| `debugSnapshot()` | — | string(JSON) \| `null` | 检查器树 |
| `debugProbe(x, y)` | number, number | string(JSON) \| `null` | 命中链 |
| `debugLastPointer()` | — | string(JSON) | `{x,y,phase}` |
| `debugPointerTrace(on, suppress)` | bool, bool | — | 开关追踪/抑制派发 |
| `debugMarkSubtree(id, on)` | u64, bool | — | 标记 devtool 层根 |
| `debugMemory()` | — | string(JSON) | quickjs 内存用量 |

这些是一次性读函数，不是 setter，不进 `abi.yml` 的变更缓冲（AGENTS 约定变更缓冲管的是嵌入面 setter 表）。C 侧模式与现有 `core.rect` 一致：Rust 返回 `CString`，glue 包成 JS 字符串。`debugMemory` 用 quickjs-ng 的 `JS_ComputeMemoryUsage` 取 `malloc_size`/`memory_used_count` 等字段。

### 呈现面：`packages/devtools`

包结构与依赖方向：

```
packages/devtools/
  package.json         @impasto/devtools；exports: "." 与 "./host"
  src/
    install.ts         installDevtools(): DevtoolsHandle
    handle.ts          句柄与 uninstall
    store.ts           signal 状态 + 游标拉取
    layer.tsx          DevtoolsLayer：悬浮按钮 + 面板壳
    button.tsx         悬浮按钮（拖动、角标）
    banner.tsx         错误横幅
    panel.tsx          面板：标签行 + 页面容器
    pages/
      console.tsx      日志
      errors.tsx       错误列表与详情（含 sourcemap）
      network.tsx      网络列表与详情
      performance.tsx  帧条形图与相位
      inspector.tsx    树、探针、高亮
      layout.tsx       溢出列表
      info.tsx         环境、内存、平台
    highlight.tsx      检查器高亮层
    source-map.ts      最小 VLQ sourcemap 消费者
    registry.ts        registerDevtoolPage
    host.gen.ts        由 just gen 生成
  deps: @impasto/solid-renderer, @impasto/solid-ui, solid-js
```

依赖方向：`devtools → solid-renderer / solid-ui`，反向不允许（`solid-ui` 不知道 devtools 存在）。gallery 引 devtools，`boot` 不引。

**挂载**：渲染器新增顶层图层注册口：

```ts
// packages/solid-renderer/runtime/index.ts（新增导出）
type TopLayerFactory = () => JSX.Element;

const topLayers: TopLayerFactory[] = [];

/** Devtools mounts its floating layer here so apps never change their JSX. */
export function registerTopLayer(factory: TopLayerFactory): () => void {
  topLayers.push(factory);
  return () => {
    const index = topLayers.indexOf(factory);
    if (index >= 0) topLayers.splice(index, 1);
  };
}
```

`boot` 的根 stack 从 `stack{column content, stack overlay}` 变成 `stack { column content, stack overlay, ...topLayers.map(f => f()) }`。顶层图层在 overlay 之上、与模态栈无关（调试层不该被 Dialog 挡住，也不参与 `closeTopModal`）。

**安装**：

```ts
// packages/devtools/src/install.ts
import { createSignal } from "solid-js";
import { callAsync } from "@impasto/solid-renderer";
import { DevtoolsLayer } from "./layer.tsx";
import { store } from "./store.ts";

export type DevtoolsHandle = {
  readonly available: boolean;
  uninstall(): void;
};

/** vConsole-style install. No-op when the host has devtools disabled. */
export function installDevtools(): DevtoolsHandle {
  void callAsync("enabled", []).then/* host module 的 enabled */((value) => {
    if (value !== true) return; // release host: stay inert
    const release = registerTopLayer(() => <DevtoolsLayer />);
    store.attach();
    handle.uninstall = () => {
      release();
      store.detach();
    };
  });
  const handle: DevtoolsHandle = { available: false, uninstall() {} };
  return handle;
}
```

（实际实现里 `enabled` 走宿主模块而非 `core`；`callAsync` 这里指模块调用门面，由 `host.gen.ts` 提供具体签名。）

**store**：signal 化的游标拉取器：

```ts
// packages/devtools/src/store.ts
import { createSignal, onCleanup } from "solid-js";
import { onFrame } from "@impasto/solid-renderer";
import * as host from "./host.gen.ts";

export type LogLevel = "log" | "info" | "warn" | "error" | "debug";

export const [logs, setLogs] = createSignal<LogLine[]>([]);
export const [errors, setErrors] = createSignal<DiagError[]>([]);
export const [network, setNetwork] = createSignal<NetEntry[]>([]);
export const [frames, setFrames] = createSignal<FrameSample[]>([]);
export const [unseenErrors, setUnseenErrors] = createSignal(0);

const cursors = { logs: 0, errors: 0, network: 0, frames: 0 };
let attached = false;
let skip = 0; // pull every 4th frame while a page is open

export const store = {
  attach() {
    if (attached) return;
    attached = true;
    const stop = onFrame(() => {
      // Badge counting runs always; page data only while a page is open.
      if (skip > 0 && !store.pageOpen()) {
        skip = (skip + 1) % 4;
        if (skip !== 0) return;
      }
      void pull();
    });
    onCleanup(stop);
  },
  async pull() {
    const revision = await host.revision();
    if (revision === lastRevision) return;
    lastRevision = revision;
    const [nextLogs, nextErrors, nextNet, nextFrames] = await Promise.all([
      host.logs(cursors.logs), host.errors(cursors.errors),
      host.network(cursors.network), host.frames(cursors.frames),
    ]);
    if (nextLogs.entries.length) { cursors.logs = last(nextLogs).seq; setLogs(merge(logs(), nextLogs.entries)); }
    if (nextErrors.entries.length) {
      cursors.errors = last(nextErrors).seq;
      setErrors(merge(errors(), nextErrors.entries));
      setUnseenErrors(unseenErrors() + nextErrors.entries.length);
    }
    /* network/frames 同形 */
  },
};
```

面板打开时按当前页只拉所需通道（`errors` 始终拉，供角标）。

**悬浮按钮**：44×44 圆形 `decorated`（radius 22，主题色底、白字 logo 字符 "◈"），外面包 `pointer`（press/release/click + drag free）。初始位置右下角，向内让出 insets 16；`onDragBy` 增量按视口夹紧，位置写进本地 signal，经 `transform` 生效（不触发布局）。角标是右上 8pt 红点加数字，`unseenErrors()` 为 0 时不画。长按 600ms 隐藏到下一次 `installDevtools`（复用 `LongPress` recognizer，`guard.ts` 的 `setRecognizers` 已支持）。

```tsx
// packages/devtools/src/button.tsx（骨架）
export function DevtoolsButton(props: { open(): void }) {
  const [pos, setPos] = createSignal(clampInitial());
  return (
    <transform x={pos().x} y={pos().y}>
      <pointer
        onClick={() => props.open()}
        recognizers={{ drag: "free", longPress: true }}
        onDragBy={(dx, dy) => setPos(clamp(pos(), dx, dy))}
        onLongPress={hideUntilNextLaunch}
      >
        <decorated radius={22} background={theme().color.primary} size={44}>
          <column align="center" justify="center" fill>
            <Text fontSize={18} color={0xffffffff}>◈</Text>
            <Show when={unseenErrors() > 0}>
              <decorated radius={8} background={0xffff3b30} w={16} h={16} />
            </Show>
          </column>
        </decorated>
      </pointer>
    </transform>
  );
}
```

**面板壳**：全屏 Sheet 形状——半透明遮罩 `pointer`（点外关闭）+ 90% 高度内容列。底部标签行用 `solid-ui` 的 `Tabs`，页面区是一台 `ScrollArea`。标签页定义在注册表里：

```ts
// packages/devtools/src/registry.ts
export type DevtoolPage = {
  id: string;
  title: string;
  badge?: () => number;
  render: () => JSX.Element;
};

const pages: DevtoolPage[] = [
  { id: "console", title: "日志", render: () => <ConsolePage /> },
  { id: "errors", title: "错误", badge: unseenErrors, render: () => <ErrorsPage /> },
  { id: "network", title: "网络", render: () => <NetworkPage /> },
  { id: "performance", title: "性能", render: () => <PerformancePage /> },
  { id: "inspector", title: "检查", render: () => <InspectorPage /> },
  { id: "layout", title: "布局", render: () => <LayoutPage /> },
  { id: "info", title: "信息", render: () => <InfoPage /> },
];

/** Third-party pages append after the built-ins. */
export function registerDevtoolPage(page: DevtoolPage): () => void { /* push + release */ }
export function devtoolPages(): readonly DevtoolPage[] { return pages; }
```

**错误呈现策略**（对应用户「错误 UI 弹窗还是怎么处理」的问题）：

三层，从轻到重：

1. **按钮角标**：任何来源的新错误让角标 +1。不打断应用。
2. **顶部横幅**：`errors` 通道出现 source 为 `listener`/`rejection` 的新条目时，屏幕顶部滑入一条可点击横幅（红底白字，消息首行 + 「详情」），5 秒自动收起或点击打开面板错误页。横幅是 devtool 图层内的组件，不是 `Toast`（Toast 属于应用语义；调试信息不该混进应用的 overlay 栈）。freeze 策略下帧不跑、横幅不动——文档里写明：freeze 是「画面冻住保现场」，横幅本来就不该再动。
3. **详情页**：堆栈逐帧列出，sourcemap 命中的帧显示 `src/pages/market.tsx:42:7` + 该行源码上下文（`sourceMap()` 的 `sourcesContent`），未命中的帧保留 bundle 坐标。提供「复制」走 `@impasto/clipboard`。

路由级渲染错误继续走现有的 `Errored` + `ErrorPage`（`packages/solid-router/src/view.tsx` 184 行），devtool 不接管——那是应用已经决定的 UI。devtool 补的是**应用没接住的那部分**：监听器错误、未处理拒绝、宿主错误。

**sourcemap 最小消费者**：只做本仓库需要的两件事——解析 `sources`/`sourcesContent`/`mappings`，查 `(line, col)` 最近的前驱映射。esbuild 的 mappings 是标准 VLQ，逐行惰性建索引：

```ts
// packages/devtools/src/source-map.ts
/** Minimal source-map consumer: nearest mapping for (line, column).
 *  Supports what esbuild emits for the single-file gallery bundle. */
export class SourceMap {
  private lines: Segment[][] = [];
  private constructor(
    readonly sources: string[],
    readonly sourcesContent: (string | null)[],
    mappings: string,
  ) {
    this.parse(mappings);
  }

  static async load(text: string): Promise<SourceMap | null> {
    const raw = JSON.parse(text) as {
      sources?: string[]; sourcesContent?: (string | null)[]; mappings?: string;
    };
    if (!raw.mappings || !raw.sources) return null;
    return new SourceMap(raw.sources, raw.sourcesContent ?? [], raw.mappings);
  }

  /** line/col are 0-based. Null when the bundle has no mapping there. */
  original(line: number, col: number): { source: string; line: number; col: number; name?: string } | null {
    const segments = this.lines[line];
    if (!segments) return this.fallback(line);
    let best: Segment | null = null;
    for (const seg of segments) {
      if (seg.col <= col) best = seg;
      else break;
    }
    const pick = best ?? segments[0];
    if (!pick) return this.fallback(line);
    return {
      source: this.sources[pick.source] ?? "?",
      line: pick.sourceLine,
      col: pick.sourceCol,
      name: pick.name === undefined ? undefined : this.names[pick.name],
    };
  }

  private parse(mappings: string) { /* base64 VLQ walk: state vars
      source/sourceLine/sourceCol/name carry across lines per the spec */ }
}
```

VLQ 解码约 40 行（标准 base64 字符表 + 连续位），完整实现写在 phase 里。`sourceMap()` 宿主方法读 `Boot` 记下的 `<js 路径>.map`（macOS 的 `--js` 参数、Android 的 assets 目录），文件缺失返回 null，面板退化为 bundle 坐标。

**性能页**：120 根竖条画在 `column` 里（每根是 `decorated`，高度 = `session_ms + submit_ms + present_ms` 归一化），超预算的条染红，正常染绿，触摸单根显示该帧相位表。相位表就是 `FramePhases` 的字段直出：`motion/dispatch/timer/raf/callback/flush/layout/layoutcb/sync/record/semantics` + `laywork/passes/reuse/shaped` + 输入计数。数据从 `frames` 通道的最后一条附带 `phases` 快照取（宿主把最近一帧的完整 `FramePhases` 存在 hub 的一个槽里随 `frames` 一起给）。

**检查器页**：两个视图。树视图把 `debugSnapshot()` 的 JSON 渲染成缩进列表（每行 kind + 尺寸 + 文本预览），点一行在**高亮层**画该节点的场景矩形框（`decorated` 无 pointer、红色描边、盖全屏 stack——普通盒子不参与命中，`Decorated` 命中交给子节点，无子即透明）。探针视图：「选择」按钮开 `debugPointerTrace(true, true)`，此时应用输入被抑制（`suppress=true`），下一次 `debugLastPointer()` 的 phase 变成 `up` 时取坐标调 `debugProbe(x, y)`，得到根到叶命中链，面包屑展示、叶子高亮。退出探针恢复 `debugPointerTrace(false, false)`。`debugMarkSubtree(devtool 层根, true)` 在 install 时调一次，保证探针永远命中应用节点而不是面板自己。

**布局页**：`debugLayoutIssues()` 列表，每行 `row 溢出 12.5pt`（父节点 id、尺寸、最远子节点 id），点行通过 `core.rect(parent)` 与 `core.rect(worstChild)` 在高亮层同时框出父（红）子（黄）两个盒子。「说明」折叠块写三类合法溢出的排除规则，让使用者知道为什么某处没报。

**信息页**：`platform()`（os、版本、缩放、viewport、insets、`app_state` 当前值）、`versions()`（hostshell 版本、engine pin 提交、quickjs pin tag）、`debugMemory()`（quickjs malloc 字节数与对象计数）、`metrics()`。留一个「环境变量」块：`IMPASTO_LOG`/`IMPASTO_DEVTOOLS`/`IMPASTO_DUMP` 当前生效值（宿主读 env 后回填进 `platform()`）。

### 控制台能力补齐（console 面）

现状缺口随本计划一起补：

- `console.debug`（level 4 → `LogLevel::Debug`，`log_hook` 映射表扩一格）。
- `console.time(label)` / `console.timeEnd(label)` / `console.timeLog(label)`：prelude.js 里一张 `Map` + `performance.now`，`timeEnd` 打一行 info。纯 JS，不进 glue.c。
- `console.table` 暂缓（等真实使用场景）。
- `core.log(level, message)` 绑定补上（glue.c `bind_core` 加一项，字符串 level → 数字），修好 `reportError` 真机 no-op 的断链。
- **未处理拒绝**：glue.c 在 `qjs_open` 里装 `JS_SetHostPromiseRejectionChecker`，回调里取 reason 格式化（复用 `write_exception` 的 stack 拼接），按 promise 指针去重（一个小 open-addressing 集，每轮 job drain 清空）后走 `fns.log(3, "unhandled rejection: …")`；`runtime_host.inc` 的 `settle()` 与 `qjs_module_event`/`qjs_socket_event` 里的 `JS_FreeValue` 前各补一行同样的上报。

### `IMPASTO_INSPECT` 帧报告

refactor-plan 80 行的目标随本计划落地，与面板共享同一份快照：

- `IMPASTO_INSPECT=<dir>` 时，macOS 宿主（`app.rs`，与 `IMPASTO_DUMP` 同一处读取）把目录传进 `Boot`。
- 渲染线程在 present 成功后写两样：`<dir>/frame.png`（现有 dump 读回路径）与 `<dir>/frame.json`。
- `frame.json` 内容：帧号、`FrameTiming` 全字段、`FramePhases` 全字段、层树递归结构（每层 kind/origin/子孙 picture 的命令数与 owner 节点 id）、`SubmitStats` 缓存计数、当前 `layout_issues`、quickjs 内存。

```rust
// crates/hostshell/src/diagnostics/inspect.rs（骨架）
pub fn write_frame(
    dir: &std::path::Path,
    tree: &protocol::LayerTree,
    timing: &FrameTiming,
    phases: Option<&session::FramePhases>,
    issues: &[serde_json::Value], // session 序列化后的 layout issues
) -> std::io::Result<()> {
    let mut layers = Vec::new();
    walk(tree, tree.root, &mut layers); // Layer 链: first_child/next
    let report = serde_json::json!({
        "frame": timing.number,
        "timing": timing_json(timing),
        "phases": phases.map(phases_json),
        "layers": layers,
        "issues": issues,
    });
    std::fs::write(dir.join("frame.json"), serde_json::to_vec_pretty(&report)?)
}
```

层树走查基于 `protocol::layer.rs` 现有的 `first_child`/`next` 链。写文件在渲染线程上同步做（与 dump 读回同一线路），不新开线程边界。CI 里 `IMPASTO_INSPECT=target/frame just host` 可以做无头断言（dump 模式本来一帧即退）。

### 开关、安全与生产

| 环节 | 行为 |
|---|---|
| `IMPASTO_DEVTOOLS=0` 或未设 | macos/android 宿主不构造 hub、不注册模块、不包 http 记录器；`enabled()` 恒 false；JS 层不挂载。发布 APK 构建脚本显式置 0 |
| `IMPASTO_DEVTOOLS=1` | 全链路开启。`just host` 与 debug APK 缺省传 1 |
| 头脱敏 | `authorization`/`cookie`/`set-cookie`/`proxy-authorization` 值替换 `***`，面板里「显示原值」仅对当前会话生效且不写回 hub |
| body 预览 | 2KiB 上限，UTF-8 失败显示 `<N bytes>` |
| 环形上限 | 日志 400 / 错误 100 / 网络 150 / 帧 240，溢出丢最旧 |
| 面板自身 | `debugMarkSubtree` 排除探针命中；面板上的点击不产生「应用点击」副作用（`suppress` 只在探针模式开） |

性能预算（写在文档里，落地时用 `just bench` 校验）：devtool 关闭时帧耗时增量 0；开启且面板关闭时 < 0.05ms/帧（一次 revision 原子读）；面板打开时 < 0.3ms/帧（增量克隆 + signal 写入）；布局扫描 < 0.1ms/帧（gallery 规模）。

### 扩展性

- **通道扩展（Rust）**：新通道 = `Ring<T>` 字段 + `bump()` + 模块一个 sync 方法。已预排的下一批：`storage`（AsyncStorage/FileSystem 调用记录，挂 `hostshell/src/modules/` 现有模块外层）、`permissions`（请求历史）、`router`（路由栈快照，从 solid-router 的 signal 拉）。
- **页面扩展（JS）**：`registerDevtoolPage` 追加在内置页之后，页面 id 唯一性由注册表保证。`@impasto/i18n`、`@impasto/form` 以后各出自己的一面。
- **Kotlin 路线**：hub 与宿主模块都在 hostshell，语言无关。Kotlin 会话恢复时（refactor-plan 110 行）按同一 `@impasto/devtools` 门面接（`module_prepare` 封送已是语言无关设计），面板代码不重写——面板是普通 UI 包，宿主模块是数据源，两者解耦。
- **外接面板（暂缓维持）**：hub 的 JSON 快照方法（`inspect.rs` 的 `frame.json` 组装器）就是未来协议服务的序列化层；出现第二个真实应用时把同一份快照接到 socket 上即可，不动数据面。
- **热重载 / 时间旅行（暂缓）**：热重载的触发条件是 quickjs 会话支持模块级重评估；时间旅行的触发条件是 solid signal 的历史钩子（需上游 `next` 分支持持 `_$afterCreate` 一类口子）。两者都不在本计划内，但 hub 的通道设计不为它们设障。

---

## 分阶段实施

每个阶段独立可编译、可测试。阶段 0 全是修断链，单独可合入。

### Phase 0：修好现有的断链

**File: crates/quickjs/src/glue.c**

- `bind_core` 函数表新增 `log`：`js_core_log(level, message)`，字符串 level（`"error"|"warn"|"info"|"debug"`）映射数字 3/2/1/4，其余落 1，走 `host->fns.log`。
- `bind_console` 增加 `debug`（level 4）。
- `qjs_open` 装 `JS_SetHostPromiseRejectionChecker`：

```c
/* Dedupes per drain: quickjs calls the checker every loop while the
 * rejection stays unhandled. */
#define REJECT_SEEN_MAX 64
static void promise_rejection_checker(JSContext *ctx, JSValueConst promise,
                                      JSValueConst reason, JS_BOOL is_handled) {
    Host *host = JS_GetRuntimeOpaque(JS_GetRuntime(ctx));
    if (!host || is_handled || !host->fns.log) {
        return;
    }
    for (int i = 0; i < host->reject_seen_count; i++) {
        if (JS_VALUE_GET_PTR(host->reject_seen[i]) == JS_VALUE_GET_PTR(promise)) {
            return;
        }
    }
    if (host->reject_seen_count < REJECT_SEEN_MAX) {
        host->reject_seen[host->reject_seen_count++] = JS_DupValue(ctx, promise);
    }
    char *text = format_exception_with_prefix(ctx, reason, "unhandled rejection: ");
    if (text) {
        host->fns.log(host->fns.session, text, strlen(text), 3);
        js_free(ctx, text);
    }
    /* The dup leaks one reference into the set on purpose; the set resets
     * each drain in qjs_drain_jobs and frees there. */
}
```

- `qjs_drain_jobs`（5092 行附近）开头清空 `reject_seen`（逐个 `JS_FreeValue`），保持集合生命周期与 job 轮一致。
- `runtime_host.inc` 的 `settle()`（269-273 行）、`qjs_module_event`、`qjs_socket_event`：`JS_FreeValue` 前若 `JS_IsException`，复用 `write_exception` 格式化并 `fns.log(3, …)`。

理由：`formatError`/`reportError` 已经写好（`packages/solid-ui/src/errors.ts`），只缺绑定；拒绝检查器是 quickjs-ng 官方钩子，比在 prelude 里包 `Promise` 干净。

**File: packages/solid-ui/src/errors.ts**

- `installErrorHooks` 追加 QuickJS 分支：没有 `addEventListener` 时注册一个 `globalThis.__reportUnhandled`（由 glue 在拒绝检查器里优先调用，若存在则走 JS 格式化再回宿主日志，不存在则用 C 格式化）。保持 web 路径不变。

**File: apps/gallery-solid/src/state/binder.ts、state/{flash,settings,watchlist}.ts**

- 现有 `catch {}` 全部改为 `catch (error) { reportError(error); }`。`binder.ts` 的死节点注释保留（吞异常的理由仍成立），但吞之前上报。

**File: crates/quickjs/src/lib.rs（测试区）**

- 新增集成测试：`a_console_debug_reaches_the_sink`（level 4）、`an_unhandled_rejection_is_logged`（`Promise.reject(new Error("boom"))` 后 drain，断言日志含 `unhandled rejection: boom` 与 `stack`）、`core_log_routes_levels`。

验证：`just test`（quickjs 走 NotLinked 时这些测试在本地有源码才跑；CI 上不跑解释器路径，与现状一致）。`just solid` 不受影响。

### Phase 1：DiagnosticsHub 与日志通道

**File: crates/hostshell/src/diagnostics/mod.rs（新）**

```rust
//! Developer diagnostics: ring buffers plus the devtools host module.
mod hub;
mod module;
mod recorder;
mod inspect;

pub use hub::{DevtoolsFlags, DiagnosticsHub, LogSource};
pub use module::{DevtoolsContext, module};
pub use recorder::RecordingHttpClient;
pub use inspect::write_frame;
```

**File: crates/hostshell/src/diagnostics/hub.rs（新）**

- 「设计」节的 `Ring`/`LogEntry`/`DiagnosticsHub` 全量实现。
- 单元测试：环形丢最旧、`seq` 单调、`tail(since)` 游标语义、`revision` 只增、`error()` 连带落一条 level 3 日志。

**File: crates/hostshell/src/lib.rs**

- `mod diagnostics;` 与再导出。
- `Boot` 增字段：

```rust
/// Enable the in-app devtools: hub construction, module registration,
/// http recording, and the panic hook. Default off.
pub devtools: bool,
/// Path of the bundle source map, served to the devtools panel.
pub devtools_source_map: Option<std::path::PathBuf>,
```

- `Boot::with_devtools(mut self, source_map: Option<PathBuf>) -> Self` 便利方法（置位并记路径）。

**File: crates/hostshell/src/shell.rs**

- `EngineSlot` 增加 `hub: Option<Arc<DiagnosticsHub>>`；`log_hook` 按「设计」节改写（hub 先写、level 4 映射 Debug）。
- `start()`：devtools 开启时构造 hub、装 panic hook（「设计」节代码）、用 `RecordingHttpClient::wrap` 包 `Boot.http`、`Boot.modules` 追加 `diagnostics::module(hub, context)`、把 hub 克隆进渲染线程闭包。
- `on_frame`：`session.frame` 之后 `let script_error = session.take_script_error();` 有值则 `hub.error("listener", …)`；帧末 `push_frame`（「设计」节代码）；提交失败与 present 退避两处补 `hub.error("engine", …)`。

**File: crates/session/src/lib.rs**

- `Session` trait 追加 `take_script_error` 默认实现（「设计」节代码）。

**File: crates/quickjs/src/linked/mod.rs**

- `take_script_error`（508 行）改为 trait 实现，去掉 inherent 同名方法（避免解析歧义），`lib.rs` 测试里的调用点同步。

**File: crates/macos/src/app.rs、crates/android/src/host.rs**

- `Boot` 字面量补 `devtools: devtools_enabled(), devtools_source_map: …`。macOS 读 `IMPASTO_DEVTOOLS`（缺省关；`just host` 显式传 1）、source map 取 `--js` 参数同目录 `.map`。Android 从 `Bridge` 的 BuildConfig 透传，debug APK 为真。

**File: crates/hostshell/src/diagnostics/module.rs（新）**

- 「设计」节的模块骨架，本阶段先实现 `enabled`/`revision`/`logs`/`errors` 四法（`errors` 通道已随 `take_script_error` 接线有数据）。`network`/`frames`/`memory`/`platform`/`versions`/`sourceMap` 返回空结构，后续阶段填。
- `logs` 方法示意（其余同形）：

```rust
HostMethod::sync("logs", {
    let hub = Arc::clone(&hub);
    move |args| {
        let since = int_arg(args, 0)?;
        let guard = hub.logs.lock().unwrap_or_else(|p| p.into_inner());
        let (revision, tail) = guard.tail(since);
        Ok(map(vec![
            ("revision".into(), PluginValue::F64(revision as f64)),
            ("entries".into(), PluginValue::List(tail.iter().map(|e| map(vec![
                ("seq".into(), PluginValue::F64(e.seq as f64)),
                ("at".into(), PluginValue::F64(e.at_ms)),
                ("level".into(), PluginValue::F64(e.level as f64)),
                ("source".into(), PluginValue::Str(format!("{:?}", e.source).to_lowercase())),
                ("text".into(), PluginValue::Str(e.text.clone())),
            ])).collect())),
        ]))
    }
}),
```

**File: crates/hostshell/src/host_stubs.rs、crates/hostshell/src/bin/host-stubs.rs**

- 一方包枚举加 `packages/devtools`（`package_folder("devtools")`），防漂移测试断言 `Devtools` 模块表与 `host.gen.ts` 一致。本阶段 `packages/devtools` 目录先建（含 `package.json` 与空 `host.gen.ts` 占位，`just gen` 填充）。

验证：`cargo test -p hostshell`（hub、模块、防漂移）、`cargo test -p quickjs`（script error 进 errors 通道，经 `open_with_io` 的 sink 断言）、`cargo check --workspace`。

### Phase 2：网络记录

**File: crates/hostshell/src/diagnostics/recorder.rs（新）**

- 「设计」节 `RecordingHttpClient` 全量实现 + `DiagnosticsHub::record_net`（锁 network ring、bump）。
- 单元测试（沿用 `net.rs` 测试的本地 `TcpListener` 模式）：`perform` 记录状态与 body 预览、`stream` 聚合 chunk 计数与首段预览、脱敏表命中、错误条目带 message、取消（cancel flag）不记条目。

**File: crates/hostshell/src/net.rs**

- `post_socket`（160 行）加 `hub: Option<&Arc<DiagnosticsHub>>` 参数（或把 hub 放进一个小的 `NetCtx` 一起传），`serve` 与 `socket_loop` 透传；`open/message/close/error` 四个事件点各记一条 `NetKind::Socket`。`fetch_image_body`（128 行）不记（图片管线有自己的 census）。

**File: crates/hostshell/src/diagnostics/module.rs**

- 实现 `network` 方法（同 `logs` 形状；`response_preview` 用 `text_preview` 转 Str，二进制显示 `<N bytes>`）。

验证：`cargo test -p hostshell`。

### Phase 3：帧、内存与平台通道

**File: crates/hostshell/src/shell.rs**

- 帧末 `push_frame`（Phase 1 已接线，本阶段补 hub 上「最近一帧完整 `FramePhases`」槽：`Mutex<Option<FramePhasesJson>>`，面板性能页取）。

**File: crates/quickjs/src/glue.c**

- `bind_core` 增加 `debugMemory`：`JS_ComputeMemoryUsage` 后取 `malloc_size`、`memory_used_count`、`atom_count`、`str_count` 拼 JSON 字符串返回。

**File: crates/quickjs/src/linked/ffi.rs / nodes.rs**

- Rust 侧 `debug_memory() -> String` 对应实现与测试。

**File: crates/hostshell/src/diagnostics/module.rs**

- 实现 `frames`（游标拉取 + 最近 phases 附带）、`platform`（os/osVersion/scale/viewport/insets/appState——metrics 来自 `shell` 已存的最新 `Metrics`，app_state 读 `HostConfig` 的共享槽）、`versions`（hostshell `env!("CARGO_PKG_VERSION")`、engine pin 与 quickjs pin 经各自 `build.rs` `cargo:rustc-env` 注入，缺失返回 `null`）、`sourceMap`（读 `devtools_source_map`，512KiB 上限，超出返回 null）。

**File: crates/engine/build.rs、crates/quickjs/build.rs**

- 各自把 pin 文件内容（engine 提交哈希、quickjs tag）以 `cargo:rustc-env=IMPELLER_PIN=…`/`QUICKJS_PIN=…` 发出，lib 侧 `pub const PIN: &str = env!("…")`（build 脚本在无源码时也要能发——engine pin 文件始终在仓库，quickjs.pin 同理，无源码不影响）。

验证：`cargo test -p hostshell -p quickjs -p engine`；`IMPASTO_DEVTOOLS=1` 手工 `just host` 后宿主日志能看到的字段齐全（此时还没有 UI，用模块直调的小测试脚本验证）。

### Phase 4：uicore 布局扫描

**File: crates/uicore/src/tree/debug.rs（新）**

- 「设计」节 `LayoutIssue`/`Axis`/`scan_overflow`/`take_layout_issues`/`layout_issues` 全量实现。
- `Tree` 增加 `debug: DebugState`（`scan_overflow: bool`、`issues: Vec<LayoutIssue>`、`new_issues: Vec<LayoutIssue>`、`seen_issues: HashSet<(u32, Axis)>`、`pointer_trace: bool`、`suppress_dispatch: bool`、`last_pointer: Option<(f32, f32, &'static str)>`、`debug_owned` 位集），`Default` 全零/关。

**File: crates/uicore/src/tree/layout.rs**

- `layout`（854 行）末尾接 `scan_overflow` 调用。
- `set_pixel_scale` 附近新增 `pub fn set_debug_scan(&mut self, on: bool)`（会话从 hub flags 转发）。

**File: crates/uicore/src/tree/mod.rs、crates/uicore/src/lib.rs**

- 模块声明与公共再导出（`LayoutIssue`、`Axis`）。

**File: crates/uicore/src/layout_test/tree.rs（或新 layout_test/debug.rs）**

- 测试组：
  - `a_row_with_a_too_wide_child_reports_overflow`：`Row` 里放固定宽子节点超出父宽度，断言一条 `Horizontal` issue、`excess` 数值正确、`worst_child` 是超宽者。
  - `a_column_overflow_reports_vertical`。
  - `scroll_content_never_reports`：`Scroll` 内容高过视口，断言零 issue。
  - `visible_text_overflow_never_reports`。
  - `clip_subtree_never_reports`。
  - `issues_dedupe_across_frames`：同树第二次 `layout` 后 `take_layout_issues` 为空；把子节点改宽后新签名出现。
  - `scan_off_costs_nothing`：`set_debug_scan(false)` 时 `layout_issues()` 恒空。

验证：`cargo test -p uicore`。性能：`just bench` 的 `submit_bench` 无回退（扫描默认关）。

### Phase 5：检查器与指针追踪绑定

**File: crates/uicore/src/tree/debug.rs（续）**

- `debug_snapshot`（DFS 全树，场景矩形用现有 `rect` 累加路径；`debug_owned` 命中的子树剪枝；JSON 手拼或 `serde_json`——`uicore` 目前不依赖 serde，为避免新依赖，提供 `fn write_snapshot(&self, out: &mut String)` 的手写序列化，字段固定、无转义歧义字符集处理文本用现有 `escape`（`record` 里给 `DrawList` 转文本时已有类似工具，如无可复用 `serde_json`——决策：引入 `serde_json` 仅作 dev 序列化会被 `uicore` 常驻依赖，**不引入**，手写序列化并配 round-trip 测试）。
- `debug_probe(x, y)`：复用 `hit.rs` 的命中路径，收集根到叶链（含每级 rect 与 payload 名），同样剪 `debug_owned`。
- `set_debug_owned`、`set_pointer_trace`、`debug_last_pointer`。
- `pointer.rs`/`tree/gesture.rs` 的 `pointer` 入口记 `last_pointer` 并在 `suppress_dispatch` 时早退（状态机照常）。

**File: crates/uicore/src/layout_test/pointer.rs**

- 探针测试：命中链顺序（根→叶）、`debug_owned` 子树被剪、suppress 时监听器不派发但 `debug_last_pointer` 更新、phase 字符串正确。

**File: crates/quickjs/src/linked/nodes.rs、bridge.rs**

- Rust 侧桥函数：`debug_layout_issues`/`debug_snapshot`/`debug_probe`/`debug_last_pointer`/`debug_pointer_trace`/`debug_mark_subtree`，读 `Tree` 并返回 JSON 字符串。
- 会话 `frame` 开头从 hub flags 同步 `set_debug_scan`（hub 经 `HostConfig` 侧通道传入——实现上 hostshell 在 `Session` 构造时把 `Arc<DiagnosticsHub>` 的 flags 交给 quickjs 会话，`metrics` 或 `frame` 入口读一次原子标志）。

**File: crates/quickjs/src/glue.c**

- `bind_core` 增加上表六个函数（模式照 `core.rect`：参数换算 → Rust 调用 → `JS_NewStringLen`）。
- `crates/quickjs/src/lib.rs` 测试：编译小脚本断言 `JSON.parse(core.debugLayoutIssues()).issues.length === 1`。

验证：`cargo test -p uicore -p quickjs`。

### Phase 6：devtools 包骨架与错误呈现

**File: packages/devtools/package.json（新）**

```json
{
  "name": "@impasto/devtools",
  "version": "0.1.0",
  "type": "module",
  "exports": {
    ".": "./src/index.ts",
    "./host": "./src/host.gen.ts"
  }
}
```

**File: packages/solid-renderer/runtime/index.ts**

- `registerTopLayer` 与 `topLayers` 渲染（「呈现面」节代码）；`boot` 签名改 `boot(component: () => HostNode)`（保持不变——图层走注册表，boot 无需新参数）。

**File: packages/devtools/src/{install,handle,store,registry}.ts、layer.tsx、button.tsx、banner.tsx、panel.tsx（新）**

- 「呈现面」节全部骨架落地。store 本阶段接 `errors`/`logs`；`banner.tsx` 三态（无/最新消息/已点开）；按钮可拖动、角标、长按隐藏。
- 指针细节：拖动用 `setRecognizers` 的 `drag: "free"`，`onDragBy(dx, dy)` 增量夹紧到 `[8, viewport - 52]`。

**File: packages/devtools/src/pages/errors.tsx（新）**

- 列表（source 色点 + 消息首行 + 相对时间）+ 详情（堆栈帧列表、sourcemap 命中行、复制按钮走 `@impasto/clipboard`）。
- 本阶段 sourcemap 未接，堆栈原样展示（source-map.ts 在 Phase 7）。

**File: packages/devtools/src/pages/console.tsx（新）**

- 级别过滤（五级多选）、来源过滤、子串搜索、清屏（本地 signal 清空，不动 hub）、自动滚底。

**File: packages/devtools/src/index.ts（新）**

- 导出 `installDevtools`、`registerDevtoolPage`、类型。

**File: packages/solid-ui/testing/renderTest.ts**

- `installMockHost` 增加 `enabled/revision/logs/errors/...` 假实现（内存数组 + 手动 bump），供 devtools 的 node 测试用。

**File: packages/devtools/src/*.test.ts（新）**

- store 游标测试（增量合并、角标计数）、banner 出现/收起、按钮拖动夹紧。

验证：`just solid`（devtools 测试并入 `pnpm -r test`）、`just check`。

### Phase 7：sourcemap 与网络、性能页

**File: packages/devtools/src/source-map.ts（新）**

- 「呈现面」节 `SourceMap` 全量实现：base64 VLQ 解码、跨行状态变量、惰性行索引、`original(line, col)`、`fallback`（行内无映射时取该行第一段）。
- 单测：手工构造的 mappings（已知映射点断言精确命中）、行尾/行首/越界列、无 sourcesContent。

**File: packages/devtools/src/pages/errors.tsx（改）**

- 打开面板时 `host.sourceMap()` 一次，`SourceMap.load` 缓存；堆栈帧正则 `at (\S+) \((.*):(\d+):(\d+)\)`（QuickJS 栈格式以 `write_exception` 输出为准，测试里钉住）逐帧映射，命中帧显示 `sources:content` 对应行 ±2 行。

**File: packages/devtools/src/pages/network.tsx（新）**

- 列表（方法徽标、URL 截断、状态色码、耗时、字节）+ 详情（请求/响应头表、body 预览、流式条目显示 chunk 计数与首块预览、socket 条目显示事件序列）。
- 过滤：仅失败 / 仅流式 / 子串。

**File: packages/devtools/src/pages/performance.tsx（新）**

- 120 帧条形图（`ForEach frames()` 尾部窗口，`decorated` 竖条，高度归一化到 96pt，超预算红）、当前帧相位表、缓存计数（paragraphs/pictures/textures）。
- 无任何新布局原语；条形图就是一排固定尺寸盒子。

验证：`just solid`；手工 `just host` 打开面板核对。

### Phase 8：检查器与布局页

**File: packages/devtools/src/highlight.tsx（新）**

- 高亮层：`stack fill` + N 个 `decorated`（描边、无 pointer），每个的 x/y/w/h 来自目标节点 `core.rect(id)`；红色单框（选中）、红黄双框（溢出父子）、2pt 描边。盒子不参与命中（无 pointer 即透明）。

**File: packages/devtools/src/pages/inspector.tsx（新）**

- 树视图（`debugSnapshot` → 缩进列表，展开/折叠 signal，行点击高亮 + 详情侧栏：kind、rect、focusable、listeners、文本）。
- 探针视图（「选择元素」按钮 → `core.debugPointerTrace(true, true)` → onFrame 轮询 `core.debugLastPointer()`，phase 到 `up` 取点 `core.debugProbe(x, y)` → 面包屑 + 叶子高亮 → `debugPointerTrace(false, false)`）。
- install 时 `core.debugMarkSubtree(<图层根 id>, true)`（图层根 id 由 `layer.tsx` 挂载后经 `createSignal` 传出）。

**File: packages/devtools/src/pages/layout.tsx（新）**

- `debugLayoutIssues()` 列表 + 点击双框高亮 + 合法溢出说明折叠块。

**File: apps/gallery-solid/src/quotes.tsx**

- 顶部 `const devtools = installDevtools();`（`onCleanup(devtools.uninstall)`），放在 `boot` 之前。bundle 体积增量走 esbuild tree-shake，发布构建（`IMPASTO_DEVTOOLS=0`）时 `enabled()` 为 false、图层不挂载、dead code 由 QuickJS 常量折叠不了——接受这部分字节进 bundle（面板代码 ~30KiB min），文档记录。

**File: packages/devtools/src/pages/{inspector,layout}.test.ts**

- mock host 上：探针状态机（trace→up→probe→restore）、高亮框 rect 断言、溢出列表渲染。

验证：`just solid`；手工核对 gallery 的选中高亮与已知溢出点。

### Phase 9：`IMPASTO_INSPECT` 帧报告

**File: crates/hostshell/src/diagnostics/inspect.rs（新）**

- 「设计」节 `write_frame` 全量实现：层树走查（`protocol::Layer` 的 `first_child`/`next` 链、每 picture 的 `commands.len()` 与 owner）、`timing_json`/`phases_json`、issues 序列化（复用 `uicore::LayoutIssue` 的手写序列化输出）。
- 单测：三层嵌套树的 JSON 形状断言（用 `LayerTree::empty`/构造辅助）。

**File: crates/hostshell/src/shell.rs**

- `Boot` 增 `inspect_dir: Option<PathBuf>`；present 成功且该目录存在时调 `write_frame`（与 `IMPASTO_DUMP` 同帧、同样只写一次然后按 dump 语义退出进程）。

**File: crates/macos/src/app.rs、main.rs**

- 读 `IMPASTO_INSPECT`（935 行 dump 读取旁），传 `Boot.inspect_dir`。Android 不接（无头 dump 场景在 macOS/CI）。

**File: docs/refactor-plan.md**

- 「诊断与日志」节的 `IMPASTO_INSPECT` 目标段落改写为已落地，指向本文。

验证：`IMPASTO_INSPECT=target/frame IMPASTO_DUMP=target/frame.png just host`，断言目录里 `frame.json` 存在且 `frame`/`layers` 字段可解析。

### Phase 10：接线、门面与文档

**File: justfile**

```make

# Dev host with the on-screen devtools panel (default for local runs).
host: gallery-dist
    IMPASTO_DEVTOOLS="${IMPASTO_DEVTOOLS:-1}" cargo run --manifest-path "{{root}}/Cargo.toml" -p macos -- --js "{{root}}/apps/gallery-solid/dist/quotes.js" "{{logo}}"
```

- `android` recipe 在 debug 变体拷贝后不改行为（Bridge 读 BuildConfig），补一行注释。

**File: apps/gallery-solid/build.mjs**

- 无改动（sourcemap 已开）。核对 esbuild sidecar 输出名与 `devtools_source_map` 推导一致（`quotes.js` → `quotes.js.map`）。

**File: docs/architecture.md**

- 「仓库怎么切」目录清单加 `packages/devtools/`；「一帧」节补一句：devtool 开启时帧末多一次 O(n) 盒子扫描与环形缓冲写入，全部在渲染线程、受开关控制。

**File: docs/refactor-plan.md**

- 「暂缓」109 行的 devtool 条目替换为：「外接 devtools 面板（协议服务）。触发条件是出现第二个真实应用；应用内面板已落地，见 `docs/devtools-plan.md`。」

**File: AGENTS.md**

- Monorepo 清单加 `packages/devtools/` 一行；「必须守住的边界」补一条：devtool 面板只准用现有布局原语渲染，不准往 `uicore`/`engine` 里加调试绘制命令。

### Phase 11：验证

**File: 全仓库**

- `just check`：cargo 全测 + Kotlin + Solid（devtools 包测试并入）。
- `just quality`：`-D warnings` 全绿。
- 像素金样：devtool 缺省关，`just test` 的金样断言不受影响；`IMPASTO_DEVTOOLS=1` 的画面不进金样。
- 手工清单（写进 `docs/devtools-plan.md` 末尾的验收清单）：
  1. `just host` 出现悬浮按钮，拖动夹紧，长按隐藏。
  2. 面板七页可切换，关面板后台拉取停止（仅角标）。
  3. gallery 制造一个监听器抛错：角标 +1、横幅出现、详情堆栈映射到 `src/…tsx`。
  4. `Promise.reject` 无 catch：错误页出现 `unhandled rejection` 条目。
  5. 布局页能列出 gallery 里已知的主轴溢出；点击双框高亮。
  6. `/data` 页 fetch 后网络页出现条目，authorization 头显示 `***`。
  7. 性能页条形图随滚动刷新，超预算帧红色。
  8. 检查器选中 VirtualList 行节点，高亮框贴合。
  9. `IMPASTO_DEVTOOLS=0 just host`：无按钮、`just bench` 无回退。
  10. `IMPASTO_INSPECT=… just host`：`frame.json` 与 PNG 成对写出。

---

## 架构说明

- **Semver 影响**：`Boot` 加字段与 `Session` 加默认方法都是破坏性变更（构造点在仓库内全部更新）；JS 侧 `registerTopLayer`、`installDevtools`、`registerDevtoolPage` 为新增导出。devtools 包版本 0.1.0，与其它包同步演进。
- **线程归属**：hub 的四个 ring 都被 `Mutex` 保护，写者三个（渲染线程：日志/错误/帧；net 线程：网络；平台线程：无）。模块方法全部 `Sync`，在渲染线程上短临界区克隆；不出现 `Arc<Mutex<_>>` 跨越 `Engine` 或 `Typography` 的 `!Send` 边界。`frame.json` 写盘在渲染线程同步执行，与 dump 读回同一条线路，不开新线程。
- **副作用分析**：`revision` 是 `AtomicU64`，热路径（面板关闭时）只有一次 acquire 读；布局扫描 O(n) 且默认关；http 记录器在 net 线程上多两次 `Vec` 克隆（预览 2KiB 上限），不在渲染线程；panic hook 只在 devtools 开启时装，且先落 hub 再交默认 hook，不吞栈。`debug_owned` 位集挂在槽位 flags 上，随节点退役自然失效。
- **明确不改的东西**：`protocol::Command` 不加调试命令（`DrawList` 保持纯产品语义）；`engine` crate 不加任何诊断面（缓存计数已经走 `SubmitStats`）；`uicore` 的 `record` 不加调试分支（高亮是 devtool 的普通盒子）；`services.rs` 的 `Services` trait 不动（hub 在 `log_hook` 层旁路，不动 sink 协议）；`abi.yml` 不动（新增的是读函数与一次性回调，不是 setter 变更面）。
- **新增跨 crate 依赖**：`hostshell → serde_json`（仅 `inspect.rs` 的 frame.json 组装，hostshell 已有重依赖生态，可接受；若要避免，inspect 也手拼——决策留给实现时按 `cargo tree` 体积定）；`devtools → solid-renderer/solid-ui/clipboard`（包级，方向合法）。`uicore`、`engine`、`protocol`、`session` 不新增外部依赖。
- **循环依赖**：devtools 挂载靠 `registerTopLayer`（renderer 提供注册表、devtools 提供工厂），renderer 不 import devtools，图层级无环。
- **Kotlin 与 iOS 的位置**：hub/模块/记录器全在 hostshell 与 session 层，语言无关；Kotlin 会话恢复时按同一模块门面接。iOS 宿主出现时（预构建矩阵有列）只需在 `crates/ios` 的 Boot 装配里置 `devtools` 字段。
- **与 freeze 策略的交互**：`freeze` 下帧不再跑，面板与横幅停在冻结时刻——这是特性（保现场）不是缺陷，文档写明；`report`（缺省）下一切照常。
- **数据真实性**：所有面板数据来自与绘制同一条链路（同一棵 `Tree`、同一次 `FrameTiming`、同一个 net 线程），不存在「面板看到的和应用跑的不是一份」的偏差类 bug。

---

## 文件变更清单

- AGENTS.md -- Monorepo 清单与边界各加一条 devtool 约定
- apps/gallery-solid/build.mjs -- 无代码改动，核对 sourcemap sidecar 名
- apps/gallery-solid/src/quotes.tsx -- `installDevtools()` 接线
- apps/gallery-solid/src/state/binder.ts -- 静默 catch 改为上报
- apps/gallery-solid/src/state/flash.ts -- 同上
- apps/gallery-solid/src/state/settings.ts -- 同上
- apps/gallery-solid/src/state/watchlist.ts -- 同上
- crates/android/src/host.rs -- Boot 字面量补 devtools 字段（BuildConfig 透传）
- crates/engine/build.rs -- pin 提交哈希经 cargo:rustc-env 发出
- crates/hostshell/src/bin/host-stubs.rs -- 一方包枚举加 devtools
- crates/hostshell/src/diagnostics/mod.rs -- 新：模块声明与再导出
- crates/hostshell/src/diagnostics/hub.rs -- 新：环形缓冲与 DiagnosticsHub
- crates/hostshell/src/diagnostics/module.rs -- 新：@impasto/devtools/host 宿主模块
- crates/hostshell/src/diagnostics/recorder.rs -- 新：RecordingHttpClient
- crates/hostshell/src/diagnostics/inspect.rs -- 新：frame.json 组装与写盘
- crates/hostshell/src/host_stubs.rs -- devtools 包枚举与防漂移
- crates/hostshell/src/http.rs -- 无改动（被包装，不改本体）
- crates/hostshell/src/lib.rs -- Boot 新字段、diagnostics 模块、再导出
- crates/hostshell/src/net.rs -- socket 事件记录点透传 hub
- crates/hostshell/src/shell.rs -- log_hook 扩展、panic hook、take_script_error 消费、push_frame、inspect 写盘
- crates/hostshell/src/timing.rs -- 无改动（FrameSampling 在别处做映射）
- crates/macos/src/app.rs -- Boot 字面量、IMPASTO_INSPECT 读取
- crates/macos/src/main.rs -- 无行为改动（参数不变）
- crates/quickjs/build.rs -- quickjs pin 经 cargo:rustc-env 发出
- crates/quickjs/src/glue.c -- core.log/console.debug/拒绝检查器/debug* 绑定
- crates/quickjs/src/lib.rs -- 新集成测试（rejection、debug 绑定、issues）
- crates/quickjs/src/linked/mod.rs -- take_script_error 改 trait 实现、debug flags 同步
- crates/quickjs/src/linked/nodes.rs -- debug_* 桥函数
- crates/quickjs/src/prelude.js -- console.time/timeEnd/timeLog
- crates/session/src/lib.rs -- Session::take_script_error 默认方法
- crates/uicore/src/layout_test/debug.rs -- 新：溢出扫描测试组
- crates/uicore/src/layout_test/pointer.rs -- 探针与 suppress 测试
- crates/uicore/src/lib.rs -- LayoutIssue/Axis 再导出
- crates/uicore/src/tree/debug.rs -- 新：扫描、快照、探针、指针追踪
- crates/uicore/src/tree/gesture.rs -- pointer 入口记 last_pointer 与 suppress 早退
- crates/uicore/src/tree/layout.rs -- layout 末尾接扫描
- crates/uicore/src/tree/mod.rs -- debug 模块与 Tree::debug 状态
- docs/architecture.md -- 目录清单与一帧节补 devtool 描述
- docs/refactor-plan.md -- 暂缓条目改写、IMPASTO_INSPECT 落地迁移
- docs/devtools-plan.md -- 本文（落地后按规则迁往 architecture）
- justfile -- host recipe 注入 IMPASTO_DEVTOOLS 缺省
- packages/devtools/package.json -- 新：@impasto/devtools
- packages/devtools/src/banner.tsx -- 新：错误横幅
- packages/devtools/src/button.tsx -- 新：悬浮按钮
- packages/devtools/src/handle.ts -- 新：安装句柄
- packages/devtools/src/highlight.tsx -- 新：检查器高亮层
- packages/devtools/src/host.gen.ts -- 新（just gen 生成）
- packages/devtools/src/index.ts -- 新：公开导出
- packages/devtools/src/install.ts -- 新：installDevtools
- packages/devtools/src/layer.tsx -- 新：DevtoolsLayer
- packages/devtools/src/panel.tsx -- 新：面板壳
- packages/devtools/src/pages/console.tsx -- 新：日志页
- packages/devtools/src/pages/errors.tsx -- 新：错误页（含 sourcemap 映射）
- packages/devtools/src/pages/info.tsx -- 新：信息页
- packages/devtools/src/pages/inspector.tsx -- 新：检查器页
- packages/devtools/src/pages/layout.tsx -- 新：布局页
- packages/devtools/src/pages/network.tsx -- 新：网络页
- packages/devtools/src/pages/performance.tsx -- 新：性能页
- packages/devtools/src/registry.ts -- 新：页面注册表
- packages/devtools/src/source-map.ts -- 新：最小 VLQ 消费者
- packages/devtools/src/store.ts -- 新：signal 状态与游标拉取
- packages/solid-renderer/runtime/index.ts -- registerTopLayer 与根 stack 顶层图层
- packages/solid-ui/src/errors.ts -- installErrorHooks 的 QuickJS 分支
- packages/solid-ui/testing/renderTest.ts -- mock host 增 devtools 方法

---

## 验收清单（落地后手工过一遍）

1. `just host`：按钮出现、可拖动夹紧、长按隐藏、角标随新错误增长。
2. 面板：七页切换、关闭后拉取降频到角标通道。
3. 监听器抛错：角标、横幅、详情堆栈、sourcemap 映射到 TSX 行。
4. `Promise.reject` 无人接：错误页出现 `unhandled rejection`。
5. 布局页：能列出 gallery 已知溢出，点击出红黄双框。
6. 网络页：fetch/fetchStream/WS 各一条形状，敏感头 `***`。
7. 性能页：条形图随交互刷新，超预算帧红，相位表与超预算日志行一致。
8. 检查器：探针模式点选叶子，面包屑与高亮框贴合，退出后应用输入恢复。
9. `IMPASTO_DEVTOOLS=0`：无按钮，`just bench` 与基线无差。
10. `IMPASTO_INSPECT=<dir>`：`frame.json` + PNG 成对写出，层树结构可解析。
