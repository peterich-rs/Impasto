# 阶段 2：进行中的运行时提取

本文记录已经落地的部分和还没做的部分。阶段 2 没有完成。不要把这份记录读成“通用标准库已经独立”。

## 基线

| 项 | 值 |
|---|---|
| main | `174c3663834da99ed972ce6b943ca3604bdcc040`（PR 21 合并提交，fetch 后没有更新的提交） |
| 本分支代码 | `11fd407ccc317c2c496b3ce8d04761d447f9ff70` |
| 阶段 2.0 测试 | `3590486`，在提取之前锁住定时器 kind、时钟原点和宿主 getter 重入 |

## 2.0–2.7 完成度

| 步 | 状态 |
|---|---|
| 2.0 基线补证 | 已做。新增 timer kind 取消、负 delay、嵌套 timer 下一帧、`performance.now` 在首帧前为 0、参数 getter 在解码期间调用另一个宿主方法。既有打开顺序和未知 token 测试仍在 |
| 2.1 旧 crate 内分家 | 未做完。UI 引用表、rAF 记录、诊断映射仍在 `quickjs` |
| 2.2 创建 js 并迁 VM | 部分。`js::quickjs::Vm` 是唯一创建 QuickJS runtime/context、安装 interrupt 与 rejection tracker、执行 32-job drain 的地方。UI `Host` 持有这个 `Vm`。无 UI 的 `js::Runtime` 也用它 |
| 2.3 标准绑定 | 未完成。headless 自己安装 console、performance 和 timeout/interval。UI 仍用 `bindings.rs` 里的那一份。rAF 仍只在 UI |
| 2.4 模块与编译 | 未做。`quickjs::compile` 仍在原处，并继续声明 scene/reactive。没有 `CompileEnvironment` |
| 2.5 I/O 与 prelude | 部分。`prelude.js` 的正文只在 `crates/js/src/standard/legacy-prelude.js`。UI 和无 UI 都求值这份字符串。HTTP、socket、Settle 交付仍在 `quickjs`。headless 只放了一个空的 `__host`，不能收发 `IoEvent` |
| 2.6 清理与守卫 | 部分。`tools/check-js-runtime-boundary.py` 已接入 Linux CI。`crates/js-headless` 的生产依赖只有 `js`。UI 绑定和 I/O 还没删旧实现，因为新实现还没接上 |
| 2.7 全平台 | 未作为阶段完成来做。下面只记录这次检查点实际跑过的命令 |

## 已验证

在 `11fd407` 的源码上：

| 命令 | 结果 |
|---|---|
| `cargo test -p js -p js-headless` | 通过。headless 能 eval、drive 一次到期 timer、关闭后再 eval 失败；`scene` 与未注册模块被拒绝 |
| `cargo test -p quickjs` | 114 passed，0 failed，6 个既有 ignored |
| `cargo clippy -p js -p js-headless -p quickjs --all-targets -- -D warnings` | 通过 |
| `just android` | `BUILD SUCCESSFUL in 22s`。`cargo build --target aarch64-linux-android -p android` 的 dev profile `Finished ... in 15.06s`。APK `crates/android/apk/app/build/outputs/apk/debug/app-debug.apk`，126295757 字节。包内有 `lib/arm64-v8a/libimpasto.so`（未压缩 259614512 字节）和 `lib/arm64-v8a/libimpeller.so`（6858536 字节）。NDK 27.2.12479018，ABI `arm64-v8a`，rustc 1.100.0。APK 没有进 git |

`a11y.rs` 的 dead_code 警告仍在，基线里就有。

## 未验证

- `cargo test --workspace`、style-types、全仓 clippy、pnpm、Kotlin
- macOS CI（要等这个分支的 PR）
- 装到设备、release APK、`android-x64`
- 性能对照和泄漏采样。没有这些数据，不能说变快或没有泄漏
- headless 的 fetch / socket / 取消 / 关闭后迟到事件
- 编译封套和 top-level await 票据

## 偏差

- 定时器选择函数改名为 `eligible_turn`，UI 仍在原来的 `vm_set_clock` 里把计数加一。选择规则没改。
- headless 的 `eval_script` 不排空 job；`drive` 才排空。这是新 API。旧 `quickjs::open` 仍在求值后 drain。
- 没有 generation id，也没有改 `IoEvent`。
- 入口脚本仍先于 `install_io`。这是保留的旧缺陷。

## 回退

普通反向提交即可：先撤 `js-headless`、边界脚本和 CI 行，再撤 `quickjs` 对 `js::Vm` 的引用，最后从 workspace 去掉 `js` 并把 prelude 移回 `crates/quickjs/src/prelude.js`。已合并的 `js-host` 不动。
