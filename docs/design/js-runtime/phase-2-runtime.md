# 阶段 2：进行中的运行时提取

阶段 2 没有完成。下面按 v4 第 19–20 节的 2.0–2.7 写已经落地的事实和还缺的事实。

## 基线

| 项 | 值 |
|---|---|
| main | `174c3663834da99ed972ce6b943ca3604bdcc040`（PR 21 合并提交） |
| 本文件对应的代码 | `d9b755de45d8699415ed1a4c64974008256446fa` |
| 2.0 基线测试 | `3590486` |

`0250bd3` 没有单独的检查记录。包含它的 `a5f7564` 上，GitHub Actions run `44444264497`：`cargo test (macos)`、`kotlin`、`solid` 成功。`cargo test (linux)` 失败在 `crates/cli/tests/commands.rs:18`，两个沙箱在同一纳秒撞了同一个临时目录（`File exists`）。`d9b755d` 给沙箱加了序号。这次本地全仓测试通过。`d9b755d` 自己的 CI 还要等这次推送。

## 2.0–2.7

| 步 | 状态 |
|---|---|
| 2.0 基线补证 | 已做。timer kind、负 delay、嵌套 timer、首帧前 `performance.now() == 0`、参数 getter 重入 |
| 2.1 旧 crate 内分家 | 部分完成。普通 timer 在 `js::Runtime`。rAF、`wants_frame`、UI 引用表留在 `Host`。`ScriptFault` 由 `JsDiagnostic` 在 `quickjs` 里映射，不进通用 VM |
| 2.2 创建 js 并迁 VM | 部分完成。UI `Host` 持有 `js::Runtime::bare` 的同一份 VM。帧仍在原来的 `vm_set_clock`、`vm_fire_timers`、`vm_fire_raf` 和 `drain` 上推进，没有改成一次 `drive`。`WeakRuntime` 不持有 Context |
| 2.3 标准绑定 | 部分完成。console、performance、timeout、interval 只有 `js::standard`。rAF 留在 UI。`OperationPolicy` 包住宿主方法；UI 的实现就是原来的 `canvas::side_effect`。console 不走这个策略 |
| 2.4 模块与编译 | 已接到生产路径。内存模块和 `NativeFactory` 在 `js`，不读工作目录，也不内置 scene / reactive / `@impasto/`。`CompiledBytecode` 和 IQJS 封套的编译、打开在 `js::bytecode`。`quickjs::compile` 签名不变，编译前仍调用 `prepare_compile`。`ModuleTicket` 有 Pending / Fulfilled / Rejected；top-level await 要等 `drive` 才离开 Pending。另一份 runtime 不能查询这张票据 |
| 2.5 I/O 与 prelude | 部分。prelude 只有一份。`js::deliver::apply` 同时给定时交付和 UI 的 http / stream / socket 完成使用。可选 `HttpClient` 在调用线程里完成请求，`drive` 才跑 JavaScript。headless 测了普通 fetch、abort flag、同步 client，以及 stream 的 head/chunk。socket 完成没有单独测试。UI 仍在原来的帧位置调用，没有改成一次 `drive` |
| 2.6 清理与守卫 | 部分。headless 测了 32-job 上限、每批 64 个 timer、socket open/message、关闭后迟到事件、失败脚本不关掉 VM、`WeakRuntime`、跨 runtime 拒绝，以及关闭后 promise/timer/ticket/rejection 计数归零。清理仍先丢掉 timer 和 UI 引用，再关 VM。宿主模块监听和临时传值栈仍在 UI 引用表 |
| 2.7 全平台 | 见下一节。没有性能采样 |

## 已验证

命令在 `d9b755de45d8699415ed1a4c64974008256446fa` 的工作树上跑，当时工作树与该提交一致。Android 的 native 库来自其中的 runtime 源码；这次 `cargo` 因为只改了 CLI 测试而没有重编 `.so`。

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
| `just android` | `BUILD SUCCESSFUL in 1s`（native 已是当前 runtime 源码，`Finished ... in 0.11s`）。dev profile，`aarch64-linux-android`。APK `crates/android/apk/app/build/outputs/apk/debug/app-debug.apk`，127170168 字节。`lib/arm64-v8a/libimpasto.so` 未压缩 262139768 字节，`lib/arm64-v8a/libimpeller.so` 6858536 字节。NDK 27.2.12479018，rustc 1.100.0。APK 没有进 git |

`a11y.rs` 的 `class_name` 和 `is_clickable` dead_code 警告仍在，基线里就有。

## 还缺

1. 宿主模块监听和临时传值栈仍在 UI 的 `References`，没有单独的 js 监听表。它们不在通用 VM 里。
2. 不能在不加入测试缝的情况下让 `Vm::new` 失败。prelude 是现有脚本，不会主动抛错。入口抛错和清理先于关 VM 仍由原有 UI 测试覆盖。
3. 装机、release APK、`android-x64` 没有跑。
4. 没有性能对照，也没有泄漏采样。不能说变快，也不能说没有泄漏。
5. `d9b755d` 的 CI 还没有结果。`a5f7564` 的 macOS 成功只覆盖到 `0250bd3` 的代码。

## 保留的旧行为

- `quickjs::open_with_io` 仍先跑入口，再装 I/O。
- 没有 generation id。
- headless 的 `eval_script` 不排空 job。`drive` 才交付完成、跑 timer 并 drain。
- 定时器字段是 `eligible_turn`。UI 仍在 `vm_set_clock` 里把计数加一。
- 普通 `&[u8]` 仍只是源码。读取字节码的入口仍是 unsafe 的 `TrustedBytecode`。

## 回退

普通反向提交。先撤 `js-headless`、边界脚本和 CI 行，再撤 `quickjs` 对 `js::standard`、`js::host`、`js::bytecode`、`js::deliver` 和 `js::Vm` 的调用，最后从 workspace 去掉 `js`，并把 prelude 移回 `crates/quickjs/src/prelude.js`。已合并的 `js-host` 不动。封套格式没有改。
