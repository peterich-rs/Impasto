# 阶段 2：运行时提取

本轮列出的硬性项已经落地：宿主模块监听在 `js::Runtime`，同步宿主结果是调用内的 `PluginValue`，启动失败有 `cfg(test)` 注入点，并做了同环境的小测量。阶段 3 的 `ImpastoExtension` 清理不在这里。装机、release APK、`android-x64` 和进程 RSS 没有测，不能用它们代替上面的结果，也不能从这次计时推出泄漏结论。

## 基线

| 项 | 值 |
|---|---|
| main | `174c3663834da99ed972ce6b943ca3604bdcc040`（PR 21 合并提交） |
| 本文件对应的代码 | `72cb53f62d51ea0e1a998eb145b9d4527bfb5dc8` |
| 2.0 基线测试 | `3590486` |

`6941005` 的四项 CI 已全部成功，含 macOS。那个提交含到所有权分离，不含这一轮的监听表和启动失败注入。`72cb53f` 的 CI 由这次推送开始，不在本文写结果。

## 2.0–2.7

| 步 | 状态 |
|---|---|
| 2.0 基线补证 | 已做。timer kind、负 delay、嵌套 timer、首帧前 `performance.now() == 0`、参数 getter 重入 |
| 2.1 旧 crate 内分家 | 部分完成。普通 timer 在 `js::Runtime`。rAF、`wants_frame`、UI 引用表留在 `Host`。`ScriptFault` 由 `JsDiagnostic` 在 `quickjs` 里映射，不进通用 VM |
| 2.2 创建 js 并迁 VM | 部分完成。UI `Host` 持有 `js::Runtime::bare` 的同一份 VM。帧仍在原来的 `vm_set_clock`、`vm_fire_timers`、`vm_fire_raf` 和 `drain` 上推进，没有改成一次 `drive`。`WeakRuntime` 不持有 Context |
| 2.3 标准绑定 | 部分完成。console、performance、timeout、interval 只有 `js::standard`。rAF 留在 UI。`OperationPolicy` 包住宿主方法；UI 的实现就是原来的 `canvas::side_effect`。console 不走这个策略 |
| 2.4 模块与编译 | 已接到生产路径。内存模块和 `NativeFactory` 在 `js`，不读工作目录，也不内置 scene / reactive / `@impasto/`。`CompiledBytecode` 和 IQJS 封套的编译、打开在 `js::bytecode`。`quickjs::compile` 签名不变，编译前仍调用 `prepare_compile`。`ModuleTicket` 有 Pending / Fulfilled / Rejected；top-level await 要等 `drive` 才离开 Pending。另一份 runtime 不能查询这张票据 |
| 2.5 I/O 与 prelude | 已接到生产路径。prelude 只有一份。`js::deliver::apply` 给定时 runtime 和 UI 的 http / stream / socket 完成共用。可选 `HttpClient` 在调用线程完成，`drive` 才跑 JavaScript。测了 fetch、abort、同步 client、stream head/chunk 和 socket open/message。UI 仍在原来的帧位置调用 |
| 2.6 清理与守卫 | 已接到生产路径。宿主模块监听在 `Runtime`，关闭时在 VM 仍打开的 `with` 里丢掉。同步宿主结果和 plugin settle 用调用内的 `PluginValue`，不再经过 UI 的 `values` 栈。`cfg(test)` 的 `BootStop` 覆盖 runtime、context、bindings、prelude 失败；入口抛错后 runtime 仍在，直到 `shutdown` 把计数归零并让 `WeakRuntime` 失效。迟到 `emit_listener` 在关闭后失败。UI 引用、rAF、`ScriptFault` 映射留在 `quickjs` |
| 2.7 全平台 | 见下一节。同环境 release 计时见「测量」。没有 RSS 采样 |

## 已验证

命令在 `72cb53f62d51ea0e1a998eb145b9d4527bfb5dc8` 的工作树上跑，当时工作树与该提交一致。

| 命令 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 通过 |
| `python3 tools/check-js-host-boundary.py` | 通过 |
| `python3 tools/check-js-runtime-boundary.py` | 通过 |
| `cargo run -p uicore --bin style-types -- --check` | 通过 |
| `RUSTFLAGS=-D warnings cargo check --workspace --exclude macos` | 通过。`android` 未使用 `session`、`engine` 未使用 `tracing` 仍是基线里的 manifest 警告 |
| `cargo clippy --workspace --exclude macos --all-targets -- -D warnings` | 通过 |
| `cargo test --workspace --exclude macos` | 通过。quickjs 109 passed、0 failed、6 ignored。封套拒绝测试改在 `js` 里跑，所以 quickjs 比上一轮少 5 个。uicore `record_bench` 1 个 ignored |
| `pnpm -r test` | 通过，含 gallery 构建 |
| `kotlin/gradlew -p kotlin :library:jvmTest` | `BUILD SUCCESSFUL`。`:library:jvmTest` 是 `NO-SOURCE` |
| `just android` | `BUILD SUCCESSFUL in 12s`。dev profile 的 `aarch64-linux-android` `Finished ... in 7.61s`。APK `crates/android/apk/app/build/outputs/apk/debug/app-debug.apk`，127147483 字节。`lib/arm64-v8a/libimpasto.so` 未压缩 262125112 字节，`lib/arm64-v8a/libimpeller.so` 6858536 字节。NDK 27.2.12479018，rustc 1.100.0。APK 没有进 git |

`a11y.rs` 的 `class_name` 和 `is_clickable` dead_code 警告仍在，基线里就有。

## 测量

同一台机器、同一个 `cargo test -p js --release --offline --test timing`。脚本是 20 个已完成的 Promise 加一个 0ms timer，然后 `drive` 和 `shutdown`，共 21 次，取中位数。

| 提交 | min | median | max |
|---|---|---|---|
| `6941005` | 880 µs | 902 µs | 1238 µs |
| `72cb53f` | 865 µs | 879 µs | 1309 µs |

两次中位数只差 23 µs，落在各自 max−min 的范围内。这不是加速结论，也不是泄漏结论。没有采 RSS。5 秒只是测试里的挂起上限。

## 没有测

装机、release APK、`android-x64`。这些不是本轮硬门槛。quickjs 仍有 6 个既有 ignored，uicore 有 `record_bench` ignored。

## 保留的旧行为

- `quickjs::open_with_io` 仍先跑入口，再装 I/O。
- 没有 generation id。
- headless 的 `eval_script` 不排空 job。`drive` 才交付完成、跑 timer 并 drain。
- 定时器字段是 `eligible_turn`。UI 仍在 `vm_set_clock` 里把计数加一。
- 普通 `&[u8]` 仍只是源码。读取字节码的入口仍是 unsafe 的 `TrustedBytecode`。

## 回退

普通反向提交。先撤 `js-headless`、边界脚本和 CI 行，再撤 `quickjs` 对 `js::standard`、`js::host`、`js::bytecode`、`js::deliver` 和 `js::Vm` 的调用，最后从 workspace 去掉 `js`，并把 prelude 移回 `crates/quickjs/src/prelude.js`。已合并的 `js-host` 不动。封套格式没有改。
