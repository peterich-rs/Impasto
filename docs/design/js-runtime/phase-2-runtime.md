# 阶段 2：进行中的运行时提取

阶段 2 没有完成。下面按 v4 第 19–20 节的 2.0–2.7 逐条写已经落地的事实和还缺的事实。不要把这份记录读成“通用标准库已经独立”。

## 基线

| 项 | 值 |
|---|---|
| main | `174c3663834da99ed972ce6b943ca3604bdcc040`（PR 21 合并提交） |
| 本文件对应的代码 | `1092be44f22d0e7e552d17c0e6468ea0ca6da8d6` |
| 2.0 基线测试 | `3590486`，在提取之前锁住定时器 kind、时钟原点和宿主 getter 重入 |

`1092be4` 之前的检查点 `8d25b41` 上，GitHub Actions 的 Linux cargo test、macOS cargo test、Kotlin、Solid 都成功。那些结果不属于 `1092be4`。

## 2.0–2.7

| 步 | 状态 |
|---|---|
| 2.0 基线补证 | 已做。timer 按 kind 取消、负 delay、嵌套 timer 的下一帧、首帧前 `performance.now() == 0`、参数 getter 在解码期间调用另一个宿主方法。既有打开顺序和未知 token 测试仍在 |
| 2.1 旧 crate 内分家 | 未做完。Promise 表在 `js::quickjs::Vm`。UI 引用表、rAF 记录、`ScriptFault` 映射仍在 `quickjs` |
| 2.2 创建 js 并迁 VM | 部分。`js::quickjs::Vm` 是唯一创建 QuickJS runtime/context、安装 interrupt 与 rejection tracker、执行 32-job drain 的地方。UI `Host` 持有这个 `Vm`，还没有改成持有 `js::Runtime` |
| 2.3 标准绑定 | 部分完成。console、performance、timeout、interval 只有 `js::standard` 这一份，headless 和 UI 都调用它。rAF 仍只在 UI 的 native 表。没有 `OperationPolicy` |
| 2.4 模块与编译 | 部分。`js::CompileEnvironment` 已接入。`quickjs::compile` 签名不变，编译前调用 UI 的 `prepare_compile`。封套实现、`CompiledBytecode`、内存模块 provider 和 native module factory 仍在 `quickjs`，没有迁到 `js`。`ModuleTicket` 只在 `evaluate` 返回后给 `Fulfilled`，不会表示挂起的 top-level await |
| 2.5 I/O 与 prelude | 部分。prelude 正文只在 `crates/js/src/standard/legacy-prelude.js`。两边安装同一个 `js::host`。headless 把请求记成 `IoJob`，`enqueue` 只入队，`drive` 才用 `Vm::settle` 交付。取消会把请求上的 flag 设上并记 `IoJob::Abort`。流的 head/chunk 会调 prelude 的 `_httpStream`。同步 `HttpClient` 没有接。UI 仍在原来的帧位置用 `turns.rs` 交付，没有改成一次 `drive` |
| 2.6 清理与守卫 | 部分。旧的 console / timeout / interval 安装器已从 `bindings.rs` 删掉。`tools/check-js-runtime-boundary.py` 在 Linux CI 里。`crates/js-headless` 的生产依赖只有 `js`。`js` 没有会拉起 UI 的 feature。启动失败点、root / Weak / drop 探针和监听计数还没有 |
| 2.7 全平台 | 见下一节。macOS 还没有在 `1092be4` 上跑过。没有性能采样 |

## 已验证

命令都在 `1092be44f22d0e7e552d17c0e6468ea0ca6da8d6` 的工作树上跑。当时工作树与该提交一致。

| 命令 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 通过 |
| `python3 tools/check-js-host-boundary.py` | 通过。`js-host` 的 normal/build 闭包是 std，依赖方是 `js` 和 `session` |
| `python3 tools/check-js-runtime-boundary.py` | 通过。`js` 的 normal/build 闭包不到 session、uicore、reactive、engine、shell、quickjs |
| `cargo run -p uicore --bin style-types -- --check` | 通过 |
| `RUSTFLAGS=-D warnings cargo check --workspace --exclude macos` | 通过。`android` 未使用 `session`、`engine` 未使用 `tracing` 仍是 cargo manifest 警告，基线就有，不使 `-D warnings` 失败 |
| `cargo clippy --workspace --exclude macos --all-targets -- -D warnings` | 通过 |
| `cargo test --workspace --exclude macos` | 通过。quickjs 114 passed、0 failed、6 ignored。uicore `record_bench` 1 个 ignored。没有失败 |
| `pnpm -r test` | 通过。包含 `apps/gallery-solid` 的 `dist/quotes.js` 构建和 quotes 测试 |
| `kotlin/gradlew -p kotlin :library:jvmTest` | `BUILD SUCCESSFUL`。`:library:jvmTest` 是 `NO-SOURCE`。这不是 Android 原生验证 |
| `just android` | `BUILD SUCCESSFUL in 15s`。`cargo build --target aarch64-linux-android -p android` 的 dev profile `Finished ... in 9.51s`。APK `crates/android/apk/app/build/outputs/apk/debug/app-debug.apk`，126660990 字节。包内 `lib/arm64-v8a/libimpasto.so` 未压缩 260727392 字节，`lib/arm64-v8a/libimpeller.so` 6858536 字节。NDK 27.2.12479018，ABI `arm64-v8a`，rustc 1.100.0。APK 没有进 git |

`crates/android/src/a11y.rs` 的 `class_name` 和 `is_clickable` dead_code 警告仍在，基线里就有。

headless 测试：`headless_script_promise_and_timer_share_the_runtime`、`headless_fetch_submits_an_owned_job_and_settles_on_drive`、`headless_abort_marks_the_request_and_queues_abort`、`headless_refuses_scene_and_unknown_modules`。`crates/js-headless` 的 `consumer_runs_without_a_ui_crate` 仍在。

quickjs 的 6 个 ignored 是既有项：5 个 gallery 行为在 `b48bb8a` 漂移后被忽略，加上 `toolchain_baseline_counts`。

## 还缺的硬性项

1. UI 引用、rAF 存储和 `ScriptFault` 还没有从通用 VM 分开。UI `Host` 直接嵌 `Vm`，不是持有 `js::Runtime`。
2. 没有 `OperationPolicy`。原来 `canvas::side_effect` 的检查点没有收成通用策略。
3. `CompiledBytecode` 和 IQJS 封套的实现还在 `crates/quickjs/src/bytecode.rs`。`CompileEnvironment` 只把 UI 声明注进这次编译，编译器本身没有搬到 `js`。
4. 没有内存模块 provider，也没有不硬编码 scene / reactive / `#reactive` 的 native module factory。headless 用一张 `HashMap` 拒绝这三个名字。
5. `ModuleTicket` 不能表示 Pending 的 top-level await，也不会在后续 `drive` 里把票据改成完成或拒绝。
6. 没有可选的同步 `HttpClient`。
7. headless 的 socket 完成和流式 head/chunk 有交付代码，没有测试锁住。
8. 没有启动失败点、重复 `shutdown` 以外的清理探针、root / Weak / drop 探针、跨 runtime restore 拒绝，也没有分组计数。
9. 32-job 上限和每批 64 个 timer 在实现里，headless 没有对应测试。
10. UI 完成交付仍在 `turns.rs`。headless 在 `runtime.rs` 的 `deliver`。两边都进 `Vm::settle` 或 prelude 的同名 hook，但不是同一个交付函数。
11. macOS CI 还没有在 `1092be4` 上跑。装机、release APK、`android-x64` 没有跑。
12. 没有性能对照，也没有泄漏采样。不能说变快，也不能说没有泄漏。

## 保留的旧行为

- `quickjs::open_with_io` 仍先跑入口，再装 I/O。顶层 fetch 在第一帧以 `TypeError` 失败。这是锁住的缺陷，不是本批修掉的标准行为。
- 没有 generation id。token 仍是 `u64`。
- headless 的 `eval_script` 不排空 job。`drive` 才交付完成、跑 timer 并 drain。旧 UI 仍在原来的帧位置推进，没有改成一次 `drive`。
- 定时器选择字段是 `eligible_turn`。UI 仍在 `vm_set_clock` 里把计数加一。

## 回退

普通反向提交。先撤 `js-headless`、边界脚本和 CI 行，再撤 `quickjs` 对 `js::standard`、`js::host` 和 `js::Vm` 的调用，最后从 workspace 去掉 `js`，并把 prelude 移回 `crates/quickjs/src/prelude.js`。已合并的 `js-host` 不动。封套格式没有改，不需要迁移已有字节码。
