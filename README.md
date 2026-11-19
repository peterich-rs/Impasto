# Impasto

跨平台 UI 框架。上层写细粒度响应式界面，下层用同一份场景描述，交给 Impeller 绘制，让各端像素一致。

应用用 Solid 2.0 TSX 编写。`packages/solid-renderer`、`packages/solid-ui` 和 `packages/solid-router` 提供渲染器、控件与路由。QuickJS 会话更新 `uicore` 的渲染树。

macOS 宿主用 Impeller 的 Metal 后端，Android 宿主用 Vulkan。CI 从同一个锁定的 Flutter 提交构建各目标的 Impeller SDK，本地构建链接对应的 SDK。[Impeller SDK](crates/impeller-sys/README.md)

## 文档

| 文档 | 内容 |
|---|---|
| `docs/architecture.md` | 当前仓库分层和一帧 |
| `docs/platform-hosts.md` | macOS、Android 的线程、表面和输入。iOS 还没写 |
| `docs/toolchain-design.md` | 工具链与原生响应式设计 |
| `docs/toolchain-baseline.md` | gallery 点击和滚动的对照数据 |
| `docs/refactor-plan.md` | 还没做的事 |
| `AGENTS.md` | 目标架构与工作约定 |

## 环境

- Rust stable，版本由 `rust-toolchain.toml` 决定
- [just](https://github.com/casey/just)
- Node.js 22 或更新，测试用 `node --experimental-strip-types`
- macOS 窗口需要 Xcode。Android 包需要 JDK、SDK 和 NDK。
- 原生宿主构建使用 Python 3 准备 SDK。源码构建另需 `depot_tools` 提供的 `gclient` 和 bindgen。

这些内容不在 git 里，要用时再取：

| 路径 | 取法 |
|---|---|
| `third_party/impeller-sdk/` | `just impeller` 准备当前宿主的 SDK |
| `build/impeller-source/`、`third_party/impeller-build/` | `just impeller-source` 同步源码并构建绘制库 |
| `third_party/solid/` | 见 `third_party/README.md`。`just check` 不读该检出 |
| `build/` | 下载缓存。脚本自己创建 |

## 检查

```sh
just check
```

运行样式声明检查、Clippy、Rust workspace 测试、Solid 测试和 JS 标准库契约检查。

QuickJS 测试始终运行 Cargo 构建的 quickjs-jit 0.14.2。渲染引擎的图形测试需要准备 SDK，再用 `cargo test -p engine --features impeller` 运行。

推到 GitHub 后，[`.github/workflows/check.yml`](.github/workflows/check.yml) 跑同一组检查，并多一步 `cargo fmt --all -- --check`。`crates/macos` 只在 macOS runner 上编译。Linux 工作流在测试前用 Node 22、pnpm 9.15.9 安装依赖并构建 gallery；解释器由 Cargo 构建。工作流不下载 Impeller SDK，也不打 Android 包，也不打开窗口。

## 应用命令

```sh
cargo run -p cli -- create my-app
cd my-app
/path/to/impasto analyze
/path/to/impasto run
/path/to/impasto run -d android
/path/to/impasto build bundle
/path/to/impasto build macos
/path/to/impasto build apk --debug
/path/to/impasto test
/path/to/impasto clean
/path/to/impasto packages get
```

在框架仓库根执行 `cargo run -p cli -- run` 会打开 gallery。`run` 默认 debug，`build` 默认 release；`--debug` 和 `--release` 可显式选择。release bundle 是 QuickJS 字节码。生成的平台工程和包在应用的 `build/`；`create` 不生成可提交的平台目录。`app.toml` 的 `[permissions]` 缺省关闭能力模块，可写 `clipboard = ["macos"]` 或 `linking = true`。iOS 没有宿主，构建会返回非零。

`just cli-dist` 在 `target/cli-dist` 生成完整 CLI 发布目录，须将 `impasto` 与旁边的 `impasto-tools/` 一起移动。发布包的打包、检查和无头测试使用内置 Node；原生构建还需要 Cargo 和对应平台工具链。源码开发仍使用本机 Node。`--verbose` 打印工具链路径，`run --trace` 输出帧计数。

Android 构建缺省使用生成工程里的 Gradle wrapper。需要使用本机已安装的 Gradle 时，显式设置 `GRADLE_HOME`；`run -d <设备号>` 使用 adb 安装并启动应用。APK 的 release 变体使用本地调试签名，供真机安装验证。

## 窗口

```sh
just host
IMPASTO_DUMP=/tmp/impasto-frame.png just host
just host-js
just android
just android-release
```

`just host` 打开默认 360×640 的窗口，加载 `just gallery-dist` 生成的 Gallery ESM。`just host-js` 运行同一入口。设置 `IMPASTO_DUMP` 可将首帧写成 PNG。`just android` 构建 debug APK，`just android-release` 构建 release APK。Gradle 会准备该目标的 Impeller SDK，再调用 Cargo 构建 `libimpasto.so`。

## 目录

```
crates/protocol/            DrawList、LayerTree
crates/uicore/              渲染树。Taffy 布局、命中、click 和 drag
crates/engine/              渲染引擎。读绘制列表，调用 Impeller
crates/impeller-sys/        Impeller C 绑定与 SDK 链接
crates/session/             会话。宿主每帧调用 metrics、pointer、frame、wants_frame、lifecycle
crates/shell/           壳。命令队列、帧调度、渲染线程
crates/macos/               macOS 窗口
crates/android/             Android 宿主。`HostActivity`、`HostView`、Choreographer、Vulkan
crates/quickjs/             quickjs-jit 0.14.2 会话
packages/clipboard/         @impasto/clipboard
packages/linking/           @impasto/linking
packages/app-state/         @impasto/app-state
packages/file-system/       @impasto/file-system
packages/async-storage/     @impasto/async-storage
packages/solid-renderer/    接到 uicore 的运行时
packages/solid-ui/          Button、Field、Dialog、theme、overlay
packages/solid-router/      路径、栈和 RouterView
packages/net/               createFetch
packages/form/              createForm
packages/i18n/              formatNumber、formatDate、plural
packages/permissions/       权限 API
apps/gallery-solid/         Solid 模拟盘。行情工作台、Canvas 图表、自选和模拟交易。ESM 产物 dist/quotes.js 本地构建，不提交
tools/                      node 测试的 register.mjs 和 resolve-hook.mjs
```

## 许可证

产品代码使用 [Apache-2.0](LICENSE)。归属声明在 [NOTICE](NOTICE)。`.agents/skills/rust-best-practice` 使用该目录自己的 MIT 许可证。`third_party` 里的上游代码保持各自的许可证。Impeller 的许可证随 SDK 的 `LICENSE` 文件分发。
