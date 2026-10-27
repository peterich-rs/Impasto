# Future

跨平台 UI 框架。上层写细粒度响应式界面，下层用同一份场景描述，交给 Impeller 绘制，让各端像素一致。

两条路线共用场景协议、渲染树和渲染引擎：

| 路线 | 代码 | 现状 |
|---|---|---|
| Solid 2.0 TSX + QuickJS | `frameworks/solid-renderer`、`frameworks/solid-ui`、`frameworks/solid-router` | QuickJS 会话直接改 `core` 的渲染树。编译产物是 `apps/gallery-solid/dist/counter.js` |
| Kotlin lambda DSL | `frameworks/kotlin-ui` | 暂缓。Gradle 工程还在，界面 DSL、信号和原生嵌入未接，见 `docs/refactor-plan.md` |

macOS 宿主在 `hosts/macos`，用 Impeller standalone SDK 的 Metal 后端。Android 宿主在 `hosts/android`，用同一份 SDK 的 Vulkan 后端。没有 SDK 时渲染引擎返回 `NotLinked`，测试仍可通过。iOS 的预构建矩阵没有这一列，宿主还没写。未落地的迭代规划在 `docs/refactor-plan.md`。

Kotlin 包名 `dev.uifw`、Solid 包名 `@uifw/solid-renderer` 仍是旧前缀，留到改这两层时再换。

## 文档

| 文档 | 内容 |
|---|---|
| `docs/architecture.md` | 当前仓库分层、一帧、渲染器和控件 |
| `docs/platform-hosts.md` | macOS、Android、iOS 的线程、表面和输入 |
| `docs/refactor-plan.md` | 生产级迭代规划。未落地 |
| `docs/protocol-v2.md` | Picture 字节。未排期 |
| `AGENTS.md` | 改代码时的边界。当前行为以源码为准，目标以规划为准 |

## 环境

- Rust stable，版本由 `rust-toolchain.toml` 决定
- [just](https://github.com/casey/just)
- JDK 21
- Node.js 22 或更新，测试用 `node --experimental-strip-types`
- macOS 窗口另需 Xcode，以及本机 arm64
- Android 包另需 NDK，以及 `android-arm64` 的 Impeller SDK

这些内容不在 git 里，要用时再取：

| 路径 | 取法 |
|---|---|
| `third_party/impeller-sdk/` | `engine/fetch-impeller-sdk.sh` |
| `third_party/quickjs/` | `just quickjs` |
| `third_party/solid/`、`third_party/kiteui/` | 见 `third_party/README.md`。`just check` 不读它们 |
| `build/` | 下载缓存。脚本自己创建 |

## 检查

```sh
just check
```

依次跑 `cargo test --workspace`、Kotlin 的 `:library:jvmTest`、Solid 的宿主树测试。不打开窗口。

没有 quickjs-ng 源码时，QuickJS 测试走 `NotLinked`。没有 SDK 时，渲染引擎的图形测试不编译。

推到 GitHub 后，[`.github/workflows/check.yml`](.github/workflows/check.yml) 跑同一组检查，并多一步 `cargo fmt --all -- --check`。`hosts/macos` 只在 macOS runner 上编译。工作流不下载 SDK 和解释器源码，也不打 Android 包，也不打开窗口。

## 窗口

```sh
just host
UIFW_DUMP=/tmp/uifw-frame.png just host
just host-js
just android
```

`just host` 打开默认 360×640 的窗口，加载 `apps/gallery-solid/dist/counter.js`。`just host-js` 是同一条命令。`UIFW_DUMP` 把一帧 Metal 纹理写成 PNG 后退出，用来看画面。`just android` 打 APK。需要 SDK 和 quickjs-ng 源码。

## 目录

```
protocol/                  版本 1 场景字节，以及把它收成绘制列表
core/                      渲染树。Taffy 布局、命中、click 和 drag
engine/                    渲染引擎。读绘制列表，调用 Impeller
session/                   会话。宿主每帧调用 metrics、pointer、frame、wants_frame、lifecycle
hosts/hostshell/           壳。命令队列、帧调度、渲染线程
hosts/macos/               macOS 窗口
hosts/android/             Android 宿主。`HostActivity`、`HostView`、Choreographer、Vulkan
runtimes/quickjs/          quickjs-ng 会话。没有源码时返回 NotLinked
frameworks/modules/        宿主对象类型。权限、文件和存储走这里的接口
frameworks/solid-renderer/ 接到 core 的运行时
frameworks/solid-ui/       Button、Field、Dialog、theme、overlay
frameworks/solid-router/   路径、栈和 RouterView
frameworks/net/            createFetch
frameworks/form/           createForm
frameworks/i18n/           formatNumber、formatDate、plural
frameworks/permissions/    权限 API
frameworks/kotlin-ui/      Kotlin DSL。目前只开了 JVM 目标
apps/gallery-solid/        Solid 画廊。调度、导航、列表和对话框。产物 dist/counter.js、dist/counter.esm.js
```

仓库根上的 `settings.gradle.kts` 没有 `include`，用来挡住 Gradle 往家目录继续找工程。Kotlin 工程在 `frameworks/kotlin-ui`。

## 许可证

产品代码使用 [Apache-2.0](LICENSE)。归属声明在 [NOTICE](NOTICE)。`.agents/skills/rust-best-practice` 使用该目录自己的 MIT 许可证。`third_party` 里的上游代码保持各自的许可证。Impeller SDK 的许可证在取回之后的 `LICENSE.sdk.md`。
