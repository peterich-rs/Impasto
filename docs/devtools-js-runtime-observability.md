# 前置改造 4：JS Runtime 细粒度内存、异步资源与采样剖析

> **适用范围与前置约束**
> - 本文档细化 `crates/javascript`、`crates/engine` 与符号化工具链的改造方案，覆盖 QuickJS 内存全量细分、活跃 Timer/Promise/事件队列监视、主动 GC、高频采样火焰图以及离线 SourceMap 符号还原。
> - **依赖隔离边界（红线）**：
>   - `crates/javascript --no-default-features` 必须维持纯 `std`（零外部依赖）；
>   - `crates/javascript --features runtime` 绝对禁止引入 `engine`、`raster`、`embedder*` 或任何 UI 路径（[check-javascript-boundaries.py](../tools/check-javascript-boundaries.py)）。
> - **性能约束**：详尽的 `JS_ComputeMemoryUsage` 需遍历 JS 堆对象，因此仅在 **1s 低频遥测或按需 RPC 查询** 时调用，严禁放入每帧热路径。

---

## 1. 改造目标与文件清单

| 目标文件 | 动作 | 核心职责 |
|---|---|---|
| [crates/javascript/src/runtime/mod.rs](../crates/javascript/src/runtime/mod.rs) | 修改 | 新增纯 `std` 值结构 `JsMemoryDetail`、`TimerSnapshot`、`JsResourcesSnapshot`、`JsGcResult`；在 `Runtime` 暴露查询与控制接口 |
| [crates/javascript/src/runtime/quickjs/vm.rs](../crates/javascript/src/runtime/quickjs/vm.rs) | 修改 | 实现 `memory_usage_detail()`（全量映射 `JSMemoryUsage`）、`run_gc()`、`job_limit_hits` / `budget_overruns` 计数 |
| [crates/javascript/src/runtime/quickjs/sampling.rs](../crates/javascript/src/runtime/quickjs/sampling.rs) | 修改 | 支持原子动态配置 `SamplerConfig`（采样间隔、栈深、容量上限），为 `StackSample` 补充触发入口标签 `entry` |
| [crates/engine/src/session/mod.rs](../crates/engine/src/session/mod.rs) · [player/mod.rs](../crates/engine/src/js/player/mod.rs) | 修改 | 扩展 `Session` trait 与 `JsSession` 实现，透传 `JsMemoryDetail`、`JsResourcesSnapshot`、`run_js_gc` 与采样配置 |
| [crates/embedder/common/src/diagnostics/hub.rs](../crates/embedder/common/src/diagnostics/hub.rs) | 修改 | 新增 `js_memory` 有界环形缓冲（1s 采样），透传完整 Solid `FrameTrace` 计数 |
| `crates/devtools-symbols/`（或从 [symbols.rs](../crates/cli/src/symbols.rs) 抽离共享模块） | 拆出共享 | 将 `SourceMap` 与 `x_impasto_functions` 符号解析核心抽离供 `crates/cli` 与 `examples/devtools-desktop/rust` 共用 |

---

## 2. `crates/javascript` 内存、资源与 GC 扩展

### 2.1 完整映射 `JSMemoryUsage`（纯 `std` 值结构）

QuickJS 原生 `qjs::JSMemoryUsage`（已由 `rquickjs` 的 `Runtime::memory_usage()` 暴露）包含 23 项精确分类，当前 [vm.rs:68](../crates/javascript/src/runtime/quickjs/vm.rs#L68) 只取了其中 `memory_used_size` 单个字段。在 `crates/javascript/src/runtime/mod.rs` 定义平台无关的值结构：

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JsMemoryDetail {
    // 总分配与已用
    pub malloc_limit: u64,
    pub malloc_size: u64,
    pub malloc_count: u64,
    pub memory_used_size: u64,
    pub memory_used_count: u64,
    // 原子与字符串
    pub atom_count: u64,
    pub atom_size: u64,
    pub str_count: u64,
    pub str_size: u64,
    // 对象、属性与 Shape 隐藏类
    pub obj_count: u64,
    pub obj_size: u64,
    pub prop_count: u64,
    pub prop_size: u64,
    pub shape_count: u64,
    pub shape_size: u64,
    // 函数与字节码
    pub js_func_count: u64,
    pub js_func_size: u64,
    pub js_func_code_size: u64,
    pub js_func_pc2line_count: u64,
    pub js_func_pc2line_size: u64,
    pub c_func_count: u64,
    // 数组与二进制缓冲
    pub array_count: u64,
    pub fast_array_count: u64,
    pub fast_array_elements: u64,
    pub binary_object_count: u64,
    pub binary_object_size: u64,
}
```

- 在 [vm.rs](../crates/javascript/src/runtime/quickjs/vm.rs) 实现 `saturating` 转换（QuickJS C 结构体字段为 `int64_t`，负值防护钳位到 `0`）。
- 实现 `Vm::run_gc(&self) -> Option<JsGcResult>`：在所有者线程同步记录 `before = self.memory_usage_detail()?`，调用 `runtime.run_gc()`，再采集 `after`，返回 `{ before, after, freed_bytes, duration_ms }`。

### 2.2 异步资源快照（Timers / Promises / 队列 / 预算告警）

在 [crates/javascript/src/runtime/mod.rs](../crates/javascript/src/runtime/mod.rs) 定义：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerKind {
    Timeout,
    Interval,
    AnimationFrame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerSnapshot {
    pub id: u64,
    pub kind: TimerKind,
    pub due_in_ms: i64,     // 相对于当前 Runtime 时钟的剩余毫秒（负值表示已到期排队）
    pub interval_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JsResourcesSnapshot {
    pub counts: RuntimeCounts, // 复用已有 18 项实时计数与历史峰值
    pub active_timers: Vec<TimerSnapshot>, // 最多截断前 256 个（按 due 升序）
    pub timers_truncated: usize,
    pub pending_modules: Vec<String>,      // 来自 State::tickets 中尚未完成的模块名
    pub job_limit_hits: u64,               // 触发 JobStop::Limit 次数
    pub budget_overruns: u64,              // eval / timer / jobs / callback 超出预算次数
}
```

- **来源核实**：`State::scheduler.timers` 已保存每个 `Timer` 的 `id`、`kind`（`0`=Timeout、`1`=Interval、`2`=rAF）、`due`、`interval`、`active`；`State::tickets` 保存 `TrackedModule::Pending { module, .. }`。枚举快照无需任何额外 JS 回调或 `Persistent` 解包，成本极低。
- 在 `Vm` 增加 `job_limit_hits: Cell<u64>` 与 `budget_overruns: Cell<u64>`，在 `Runtime::drive` 命中 `JobStop::Limit` 以及 `Budget` 析构/中断超时触发时递增。

---

## 3. 升级协作式采样器（`sampling.rs`）

当前 [sampling.rs](../crates/javascript/src/runtime/quickjs/sampling.rs) 写死了 `INTERVAL = 1ms`、`MAX_DEPTH = 8`、`MAX_SAMPLES = 32` 且不区分入口类型：

1. **原子动态配置 `SamplerConfig`**：
   - 将静态常量改为 `Arc<AtomicSamplerConfig>`（含 `interval_us: AtomicU64`、`max_depth: AtomicUsize`、`max_samples: AtomicUsize`）；
   - 默认值保持 `1000 µs / 8 层 / 32 样本`，开启 Detailed Trace 录屏或 JS Profiler 时通过 `Runtime::set_sampler_config(...)` 实时切换为 `500 µs / 32 层 / 128 样本`（可配置）。
2. **附带触发入口标签**：
   - [vm.rs:52](../crates/javascript/src/runtime/quickjs/vm.rs#L52) 已有 `pub entry: Cell<(&'static str, u64)>`（记录当前正在执行 `"eval"` / `"timer"` / `"jobs"` / `"callback"` 等入口阶段）；
   - `Sampler::poll(&self, entry: &'static str)` 将当前 `entry` 写入 `StackSample { at, entry, stack }`，供 DevTools 火焰图在顶层按入口类别分组或着色。

---

## 4. `crates/engine` 桥接与 `TelemetryHub` 透传

### 4.1 `Session` Trait 扩展（[crates/engine/src/session/mod.rs](../crates/engine/src/session/mod.rs)）

为避免非 JS 测试 Session 改动成本，新增方法全部提供默认空实现：

```rust
fn js_memory_detail(&self) -> Option<javascript::runtime::JsMemoryDetail> { None }
fn js_resources_snapshot(&self) -> Option<javascript::runtime::JsResourcesSnapshot> { None }
fn run_js_gc(&mut self) -> Option<javascript::runtime::JsGcResult> { None }
fn set_js_sampler_config(&mut self, _interval_us: u64, _max_depth: usize, _max_samples: usize) {}
```

在 [crates/engine/src/js/player/mod.rs](../crates/engine/src/js/player/mod.rs) 的 `JsSession` 中直接委托给 `self.vm.host.runtime`。注意以上方法仅在 `#[cfg(feature = "js")]` 下调用 `javascript::runtime`，不违背 `engine` 默认无 JS 依赖的边界规则（[check-javascript-boundaries.py](../tools/check-javascript-boundaries.py)）。

### 4.2 `TelemetryHub` 低频内存轨与 Solid 指标透传

1. **`js_memory` 环形缓冲（[hub.rs](../crates/embedder/common/src/diagnostics/hub.rs)）**：
   - 新增 `JsMemoryEntry { seq, at_ms, detail: JsMemoryDetail }` 环形缓冲（`CAP = 300`，约 5 分钟历史）。
   - 在 [ui_frame.rs](../crates/embedder/common/src/host/ui_frame.rs) 中当 DevTools 订阅了 `runtime` 通道且距上次全量采样 `>= 1000ms` 时，调用一次 `session.js_memory_detail()` 写入 `TelemetryHub`（不影响正常帧的 120Hz 预算）。
2. **Solid 响应式与 FFI 指标透传**：
   - [FrameTrace](../crates/engine/src/session/mod.rs#L73-L117) 中已采集 `updates`、`sets`、`nodes_created`、`ffi_batches`、`ffi_calls`、`style_resolves`、`repaint_boundaries_dirtied`，将其全部补充至 [hub.rs](../crates/embedder/common/src/diagnostics/hub.rs) `PhaseSample` 并通过 Protocol v2 帧流下发。

---

## 5. 离线 SourceMap 符号化共享库

当前 [symbols.rs](../crates/cli/src/symbols.rs) 已经实现了 `.impsym`（`IMPSYM\x01` / `IMPSYM\x02`）与标准 `.js.map`（含 `x_impasto_functions` 扩展字段）的二分定位和函数名恢复，但代码直接位于二进制 crate `crates/cli` 中。

- **抽离共享 crate `crates/devtools-symbols`**（或纳入独立 crate）：
  - 迁出纯解析与符号化查找逻辑（依赖 `oxc_sourcemap`、`flate2`、`serde`、`serde_json`）；
  - `crates/cli` 保持对外命令 `impasto symbolicate` 行为完全不变，改为调用 `devtools_symbols`；
  - `examples/devtools-desktop/rust` 直接依赖 `devtools-symbols`，在桌面端 Rust Core 内存加载目标 App 的 `.js.map` 或 `.impsym` Sidecar，对：
    1. QuickJS 采样调用栈（`StackSample.stack`）；
    2. `DiagError.stack` 运行时异常栈；
    3. `perf` / `TimelineRecording` 导出的 Span `js_stack` 字段
    自动完成零延迟离线还原，并映射本地 `projectRoot` 路径以支持一键跳转 VSCode / Cursor / Zed。

---

## 6. 测试与验证清单

1. **边界校验（必跑）**：
   ```sh
   just boundaries
   cargo check -p javascript --no-default-features
   cargo check -p engine --no-default-features
   ```
2. **单元测试**：
   - `cargo test -p javascript --all-features`：测试创建大量 JS 对象/字符串/数组后 `JsMemoryDetail` 各项分类递增，调用 `run_gc()` 后释放并准确统计差值；测试 `TimerSnapshot` 覆盖 `setTimeout`/`setInterval`/`requestAnimationFrame` 且正确按到期时间排序；测试 `SamplerConfig` 动态修改生效。
   - `cargo test -p engine --features js`：验证 `JsSession` 透传资源快照与 GC 调用。
   - `cargo test -p cli`：验证抽离 `devtools-symbols` 后既有 `impasto symbolicate` 与 trace 符号化测试全绿。
