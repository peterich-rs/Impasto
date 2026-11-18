# 阶段 0：JS 宿主契约基线

本文只记录 2026-11-18 这次实施时读到的代码、命令和结果。设计评审稿 v3 仍以 `10f2477d86f82393fc14e3801d775e42451f9cc0` 为阅读基线。阶段 0 不改生产行为。

## 实施基线

| 项 | 值 |
|---|---|
| 远程 `main` | `a61fdffd8ec932ea8e5262fe669754f060198d87` |
| `main` 说明 | `feat: land screen canvas and redraw the quotes desk (#19)`，2026-11-18 15:46:33 +0800 |
| 本分支起点 | 同上。工作区在切出 `cursor/js-host-phase-0-1-fe55` 前是干净的 |
| 设计阅读基线 | `10f2477d86f82393fc14e3801d775e42451f9cc0` |
| 该提交的位置 | `origin/feat/layout-replay`，不是 `main` 的祖先 |
| PR 20 | 实施时 GitHub 上仍是 OPEN，`mergeCommit` 为空，标题 `feat: replay fixed layout and defer offscreen canvas`，head 就是上面的 `10f2477` |
| rustc | 1.100.0 (0ef1bc386 2026-11-09)，`rust-toolchain.toml` 的 stable |
| cargo | 1.100.0 (3acc79c0c 2026-11-05) |
| Node | v22.23.4 |
| pnpm | 9.15.9 |
| 平台 | Linux x86_64，`6.12.113+`。本机没有 macOS，也没有 Android NDK 构建 |
| `Cargo.lock` | 3325 行，SHA-256 `d0a5b9bd87cda8e2e9a33533bfda548ac573c1d28087d3e683e91b2f666be550`。阶段 0 不改锁文件 |

`git ls-remote origin refs/heads/main` 与 `gh pr view 20` 都是这次实施时现场查的。用户说明里 PR 20 已合并、而先前读到的 `a61fdffd` 过期；现场远程与这个说明不一致。后续提取以现场 `main` 为准。

## 与设计基线的差异

`git diff 10f2477d86f82393fc14e3801d775e42451f9cc0 origin/main` 里，和阶段 0/1 有关的结论：

| 文件 | 结论 |
|---|---|
| `crates/session/src/io.rs` | 两边 blob 相同（`328c393f7539e6d843275f3d89778aaedb14f273`） |
| `crates/quickjs/src/prelude.js` | 相同 |
| `crates/quickjs/src/linked/rt/turns.rs` | 相同 |
| `crates/quickjs/src/linked/rt/mod.rs` | 相同。启动、job drain、关闭顺序没有分叉 |
| `crates/session/src/modules.rs` | `Settle`、`MethodExec`、`ImmediateExec`、`ModuleRegistry` 的 `new` / `restricted` / `module` / `by_specifier` / `method` / `call`、`ModuleConfig`、`Shared` 两边都在，签名相同 |
| 仅设计基线有 | `Method.types`、`Method::with_types`、`MethodTypes`、`EventType`、`ModuleExports`、`Module.exports`。`main` 的 `Method` 只有 `name` 和 `invoke`，`Module` 只有 `name`、`specifier`、`methods` |
| `crates/quickjs/src/linked/rt/modules.rs` | 设计基线按 `exports.events` 决定是否导出监听器，并按事件名过滤。`main` 始终导出 `addEventListener` / `removeEventListener`，不按事件名过滤。这是未合并 PR 的行为，不是 `main` 的行为 |
| `crates/quickjs/src/lib.rs` | 公共 `open*` 函数集合相同。差异主要是设计基线上的 canvas / replay 测试和 `ModuleExports` 字段 |
| `crates/session/Cargo.toml` | 设计基线多一段 Android `ndk-sys`。`main` 没有 |
| `FrameTrace`、`Session::set_semantics_enabled` | 只在设计基线。属于 UI 会话，不在本批移动名单里 |

这不推翻阶段 1 的边界：`IoEvent` 仍是 `Http` / `HttpHead` / `HttpChunk` / `Plugin` / `Socket` 这五种自有数据；`restricted` 仍是 `strip_prefix("@impasto/")` 再相等；`ModuleConfig`、`document_relative`、`Shared` 留在 `session`。偏差是：不把设计稿点名、但 `main` 上不存在的 `MethodTypes`、`EventType`、`ModuleExports` 加回来。加上去会改变 `Method` / `Module` 的字段，现有结构体字面量编不过，也等于把未合并 PR 的监听器策略带进来。

## 依赖

`cargo tree -p session -e normal,build`：`session` 直接依赖 `protocol` 和 `uicore`。`uicore` 再带到 bitflags、csscolorparser、kurbo、lru、taffy、thiserror、tracing、unicode-segmentation。没有 rquickjs，没有平台 SDK。

`js-host` 要搬走的类型只使用 `std`（`Arc`、`AtomicBool`、`RwLock`、`String`、`Vec`）。`document_relative`、`ModuleConfig`、`Shared` 也只用 `std`，但按设计留在 `session`。`Session`、`FrameTime`、`InputEvent` 使用 `uicore`，不移动。

`quickjs` 的 normal/build 依赖包含 `session`、`uicore`、`reactive`、`protocol`、`libc` 和 `quickjs-jit` 0.14.2。阶段 1 不改这条边。

## 公共签名（`main`）

移动候选，签名以 `crates/session/src/io.rs`、`modules.rs`、`lib.rs` 为准：

- `PluginValue`：`Null`、`Bool(bool)`、`F64(f64)`、`Str(String)`、`Bytes(Vec<u8>)`、`List`、`Map(Vec<(String, PluginValue)>)`。`Map` 是有序且允许重复键的向量，不是映射集合
- `PluginResult`：`Done(PluginValue)`、`Failed { message: String }`
- `Work`、`Settle::{new,send}`、`Drop` 未发送时 `Failed { message: "settle dropped" }`
- `MethodExec`：`worker`、`main_thread`、`now`，trait 是 `Send + Sync`。`ImmediateExec` 三个方法都在调用线程执行 `work` 或 `send`
- `SyncFn`、`PromiseFn`、`Invoke::{Sync, Promise}`、`Method::{sync, promise}`、`Module { name, specifier, methods }`
- `ModuleRegistry::{new, restricted, module, by_specifier, method, call}`。`call` 对 Promise 返回 `module "<module>.<method>" is not sync`，找不到时走 `module_not_linked`
- `module_not_linked`：有方法时 `module "<module>.<method>" is not linked`，否则 `module "<module>" is not linked`
- `HttpRequest` 字段：`token`、`method`、`url`、`headers`、`body`、`cancel: Arc<AtomicBool>`、`timeout_ms: Option<u64>`
- `HttpResponse`、`HttpFailure::dom_name`、`HttpError`、`HttpPiece`、`HttpClient::{perform, stream}`。`stream` 默认先 `perform`，sink 接受 head 且 body 非空才送一个 `Chunk(..., true)`
- `HTTP_BODY_LIMIT` = 4 MiB
- `IoJob`：`Http`、`HttpStream`、`Abort { token }`、`SocketOpen`、`SocketSend`、`SocketClose`
- `IoEvent`：`Http`、`HttpHead`、`HttpChunk`、`Plugin`、`Socket`。没有 `Debug`
- `document_relative` 留在 `session`。拒绝文案固定为 `path is outside the document directory`

留在 `session` 的还有 `ModuleConfig { registry, exec, settle, app_state, launch_url }`、`restrict`、`disconnected`（`app_state` 初始 `"active"`，`launch_url` 初始 `None`，settle 丢弃结果）、`Shared` / `shared` / `shared_read` / `shared_write`（锁中毒后 `into_inner`）。`FrameTime`、`InputEvent`、`Session` 不移动。

## 调用点

生产代码不直接写 `js_host`。构造、执行和交付都走 `session::`。

| 角色 | 位置 |
|---|---|
| 定义 | `crates/session/src/io.rs`、`modules.rs`；`IoEvent` 在 `crates/session/src/lib.rs:321` |
| 会话配置 | `crates/shell/src/shell.rs:520` 用 `ModuleRegistry::new`，`:521` 组装 `ModuleConfig`，`:533` 把 `IoJob` 送进 `net_tx`。`crates/shell/src/lib.rs:62` 的 `IoSink`、`:74` 的 `SessionFactory` 使用 `session::ModuleConfig` |
| 执行器 | `crates/shell/src/exec.rs` 的 `RtExec` 与 `settle_factory`。完成进 `Command::PluginDone` |
| 网络线程 | `crates/shell/src/net.rs` 消费 `IoJob`，调用 `HttpClient`。`crates/shell/src/http.rs:117` 是 `OpenWireClient` |
| 交付 | `crates/shell/src/shell/session_cmds.rs:73` 起把命令变成 `IoEvent`，`:175` `io_done` 交给会话 |
| QuickJS 启动 | `crates/quickjs/src/lib.rs:47` `open_with_io` 先 `JsSession::open_with`，返回后再 `install_io`（`:58`）。`finish_session`（`:116`）同样后装 |
| QuickJS 内部 | `linked/mod.rs:362` `install_io`；`:1106` `io_event` 匹配五种 `IoEvent`。`linked/bridge.rs:941` `submit_or_queue`：有 sink 才提交，否则走 inline |
| 模块实现 | `crates/shell/src/modules/` 的 `app_state`、`clipboard`、`fs`、`linking`、`permissions`，以及 `diagnostics/module.rs`。`fs` 与 `files.rs:74` 调用 `document_relative` |
| HTTP 包装 | `crates/shell/src/diagnostics/recorder.rs` 实现 `HttpClient` |
| 宿主 | `crates/macos/src/app.rs:1028` 和 `crates/android/src/app.rs:678` 只调用 `shell::http_client()`，再用 `shell::start`。这两个 crate 不写 `ModuleRegistry` 或 `IoEvent` |
| CLI | `crates/cli/src/test.rs:76` 用 `ModuleConfig` 结构体更新语法 |

`PluginValue` 还出现在 shell 模块、诊断模块和 `quickjs` 的插件转换里。阶段 1 保持 `session::PluginValue` 这个路径，不改这些文件的 import。

## 启动、帧和关闭

`Engine::start`（`crates/quickjs/src/linked/rt/mod.rs`）的顺序：

1. `bindings::install`：`__scene`、`console`、定时器、`performance`、`__host`、声明 `#reactive`
2. 依次执行 `abi.js`、`scene.js`、`prelude.js`、`reactive-boot.js`。每次 `eval` 自己 drain job
3. 把 `__reactive` 存进引用表并从全局删掉
4. 执行用户脚本或字节码，再 drain 一次

`open_with_io` 在上述过程结束之后才 `install_io`。因此入口脚本里的 `fetch`、`fetchStream`、`WebSocket` 看不到调用方传入的 sink。

没有 sink、也没有 `HttpClient` 时：

- `fetch` 把 `HttpFailure::Network` / `"network is not linked"` 放进 `pending.http`。第一帧 `flush_pending` 用 `dom_name()` 拒绝 Promise，脚本看到的是字符串 `TypeError`，不是那句 message
- `WebSocket` 把 socket error 放进 `pending.socket`，第一帧同步调用 `onerror`
- `fetchStream` 在 `httpStream` 返回前就同步投递错误，而 prelude 在调用返回后才 `streams.set`。这次错误找不到 entry，调用方的 Promise 一直 pending

第一帧里，测试观察到的日志顺序是：`socket-error`，然后 `timer`，然后 `fetch-fail:TypeError`，然后 `raf`。打开过程中只有同步日志和 Promise microtask，定时器和网络完成还没发生。

`frame` 在此之后还有 motion end、reactive、`onFrame`、mutation flush 和布局。本阶段没有用新测试覆盖这段后半，现有 UI 测试仍是这部分的回归。

关闭：`Engine::shutdown` 先 `begin_shutdown`，再在 VM 还在时 `release_all`、清定时器和引用、去掉 userdata，然后丢掉 context 和 runtime。`Drop` 调用 `shutdown`。没有公开入口把 `IoEvent` 送到已经 drop 的会话。活着的会话对未知 token 的 `io_event` 不产生日志。

job 上限仍是每轮 32，现有测试 `recursive_jobs_hit_the_round_limit_and_shut_down` 锁着这个失败文本。阶段 0 没有重跑它。

## 新增回归

| 测试 | 锁定的行为 |
|---|---|
| `crates/session/tests/host_contracts.rs` | settle 发送一次、丢弃一次、跨线程仍一次；`ImmediateExec` 三个方法都在调用线程；Promise 收到传入的 executor；first-wins、重复方法和重复 specifier；`restricted` 的空名单、大小写、相似前缀、空包名、重复包名和模块原序；`PluginValue` 的字节、嵌套和重复键；默认 `stream` 的 head、空 body、sink 返回 false、错误原样返回；`dom_name`、4 MiB、`document_relative` 文案；六种 `IoJob` 和五种 `IoEvent`；`ModuleConfig` 字面量、`disconnected`、中毒锁恢复；`Send` / `Sync` |
| 原有 `modules.rs` / `io.rs` 单测 | `settle_drop_sends_failed`、`settle_send_does_not_drop_twice`、`immediate_exec_runs_on_the_caller`、`registry_first_wins_and_call_rejects_promise`、两条 `document_relative` |
| `open_runs_entry_before_io_install_and_frame_flushes_pending` | 上面的打开顺序和第一帧日志。`IoJob` 计数保持 0。`fetchStream` 没有 `stream-` 日志 |
| `unknown_io_completion_is_ignored_and_drop_discards_pending_fetch` | 未知 token 被忽略；未等到帧就 drop，fetch 回调不运行 |

## 命令和结果

本机、阶段 0 提交前：

| 命令 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 通过 |
| `cargo test -p session --lib --test host_contracts` | 6 个库内测试 + 11 个契约测试通过。libtest 报告 `finished in 0.00s` |
| `cargo clippy -p session --all-targets -- -D warnings` | 通过 |
| `cargo test -p quickjs open_runs_entry_before_io_install` | `tests::open_runs_entry_before_io_install_and_frame_flushes_pending` 通过，`finished in 0.00s` |
| `cargo test -p quickjs unknown_io_completion_is_ignored` | 通过，`finished in 0.00s` |
| `cargo clippy -p quickjs --all-targets -- -D warnings` | 通过 |
| `cargo tree -p session -e normal,build` | 见上文。没有把 UI 依赖算进将要抽出的类型 |
| `cargo tree -p quickjs -e normal,build` | `quickjs` 依赖 `session`、`uicore`、`reactive`、`quickjs-jit` |

没有跑分配采样或泄漏检测，所以这里没有内存结论。libtest 的 `0.00s` 只说明这几个测试落在它的计时分辨率里，不是性能对比。

全量 `cargo test --workspace`、Linux `cargo check` / clippy、样式生成、gallery、Kotlin、Solid 以及 macOS 宿主不在这一节。它们在阶段 1 的树上按 CI 顺序补跑，结果补在文末。取消的 runner 不算通过。

## 已知问题（不在阶段 0/1 修）

1. 入口脚本先于 `install_io`。顶层 `fetch` 不会变成 `IoJob`，即使调用方传了 sink。
2. 无 sink 的 `fetchStream` 同步错误发生在 prelude 登记 token 之前，Promise 不结束。测试只锁住这个现状，测试名和本节都把它记成既有缺陷。
3. 关闭之后的迟到完成没有可调用的入口，本阶段标为未测。活会话上的未知 token 会静默忽略，这已经有测试。
4. pending 请求、定时器、回调和 root 的计数要改生产结构才能读到。没有加测试钩子，这些计数标为未测。
5. `IoEvent` 没有 `Debug`。测试不为此加 derive。

这些缺陷都在 QuickJS 集成层，不挡住把自有数据类型移到只依赖 `std` 的 crate。阶段 1 不改 `open`、`install_io` 或 prelude。

## 回退

阶段 0 只增加 `crates/session/tests/host_contracts.rs`、`crates/quickjs/src/lib.rs` 里的两个测试，以及本文。撤回时用反向提交，不改生产代码。
