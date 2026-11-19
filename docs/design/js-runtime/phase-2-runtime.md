# 阶段 2：进行中的运行时提取

阶段 2 没有完成。下面按 v4 第 19–20 节的 2.0–2.7 写已经落地的事实和还缺的事实。

## 基线

| 项 | 值 |
|---|---|
| main | `174c3663834da99ed972ce6b943ca3604bdcc040`（PR 21 合并提交） |
| 本文件对应的代码 | `0250bd32e4cf4aad8195de84bb5f849062c0c5e2` |
| 2.0 基线测试 | `3590486` |

`67a7c5c` 的 GitHub Actions run `44439226629` 四项成功，含 macOS。那个提交不包含这一轮的模块、编译器和交付改动。`0250bd3` 的 macOS CI 还要等这次推送之后的结果。

## 2.0–2.7

| 步 | 状态 |
|---|---|
| 2.0 基线补证 | 已做。timer kind、负 delay、嵌套 timer、首帧前 `performance.now() == 0`、参数 getter 重入 |
| 2.1 旧 crate 内分家 | 未做完。Promise 表在 `Vm`。UI 引用表、rAF 记录、`ScriptFault` 仍在 `quickjs` |
| 2.2 创建 js 并迁 VM | 部分。`js::quickjs::Vm` 是唯一创建 QuickJS runtime/context、安装 interrupt 与 rejection tracker、执行 32-job drain 的地方。UI `Host` 持有这个 `Vm`，还没有改成持有 `js::Runtime` |
| 2.3 标准绑定 | 部分完成。console、performance、timeout、interval 只有 `js::standard`。rAF 留在 UI。`OperationPolicy` 包住宿主方法；UI 的实现就是原来的 `canvas::side_effect`。console 不走这个策略 |
| 2.4 模块与编译 | 已接到生产路径。内存模块和 `NativeFactory` 在 `js`，不读工作目录，也不内置 scene / reactive / `@impasto/`。`CompiledBytecode` 和 IQJS 封套的编译、打开在 `js::bytecode`。`quickjs::compile` 签名不变，编译前仍调用 `prepare_compile`。`ModuleTicket` 有 Pending / Fulfilled / Rejected；top-level await 要等 `drive` 才离开 Pending。另一份 runtime 不能查询这张票据 |
| 2.5 I/O 与 prelude | 部分。prelude 只有一份。`js::deliver::apply` 同时给定时交付和 UI 的 http / stream / socket 完成使用。可选 `HttpClient` 在调用线程里完成请求，`drive` 才跑 JavaScript。headless 测了普通 fetch、abort flag、同步 client，以及 stream 的 head/chunk。socket 完成没有单独测试。UI 仍在原来的帧位置调用，没有改成一次 `drive` |
| 2.6 清理与守卫 | 部分。旧的 console / timeout / interval 安装器已删。两份边界脚本在 Linux CI。`js-headless` 只依赖 `js`。没有启动失败点、Weak / root drop 探针和分组计数 |
| 2.7 全平台 | 见下一节。没有性能采样 |

## 已验证

命令在 `0250bd32e4cf4aad8195de84bb5f849062c0c5e2` 的工作树上跑，当时工作树与该提交一致。

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
| `just android` | `BUILD SUCCESSFUL in 13s`。dev profile 的 `aarch64-linux-android` `Finished ... in 8.23s`。APK `crates/android/apk/app/build/outputs/apk/debug/app-debug.apk`，126795800 字节。`lib/arm64-v8a/libimpasto.so` 未压缩 261144392 字节，`lib/arm64-v8a/libimpeller.so` 6858536 字节。NDK 27.2.12479018，rustc 1.100.0。APK 没有进 git |

`a11y.rs` 的 `class_name` 和 `is_clickable` dead_code 警告仍在，基线里就有。

## 还缺

1. UI 引用、rAF 存储和 `ScriptFault` 没有从通用 VM 分开。`Host` 直接嵌 `Vm`，不是持有 `js::Runtime`。
2. 没有启动失败点、Weak / root / drop 探针和分组计数。32-job 上限和每批 64 个 timer 没有 headless 测试。
3. socket 完成没有测试。同步 client 的失败路径没有测试。
4. 装机、release APK、`android-x64` 没有跑。`0250bd3` 的 macOS CI 还没有结果。
5. 没有性能对照，也没有泄漏采样。不能说变快，也不能说没有泄漏。

## 保留的旧行为

- `quickjs::open_with_io` 仍先跑入口，再装 I/O。
- 没有 generation id。
- headless 的 `eval_script` 不排空 job。`drive` 才交付完成、跑 timer 并 drain。
- 定时器字段是 `eligible_turn`。UI 仍在 `vm_set_clock` 里把计数加一。
- 普通 `&[u8]` 仍只是源码。读取字节码的入口仍是 unsafe 的 `TrustedBytecode`。

## 回退

普通反向提交。先撤 `js-headless`、边界脚本和 CI 行，再撤 `quickjs` 对 `js::standard`、`js::host`、`js::bytecode`、`js::deliver` 和 `js::Vm` 的调用，最后从 workspace 去掉 `js`，并把 prelude 移回 `crates/quickjs/src/prelude.js`。已合并的 `js-host` 不动。封套格式没有改。
