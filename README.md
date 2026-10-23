# Future

跨平台 UI 框架。上层写细粒度响应式界面，下层用同一份场景描述，交给 Impeller 绘制，让各端像素一致。进程里没有 Dart VM。

两条路线共用场景协议、渲染树和播放器：

| 路线 | 代码 | 现状 |
|---|---|---|
| Solid 2.0 TSX + QuickJS | `frameworks/solid-renderer`、`frameworks/solid-ui` | QuickJS 会话直接改 `core` 的渲染树。编译产物是 `apps/gallery-solid/dist/counter.js` |
| Kotlin lambda DSL | `frameworks/kotlin-ui` | JVM 上的版本 1 编码器，用来锁住夹具字节。信号和 Kotlin/Native 还没接 |

macOS 宿主在 `hosts/macos`，用 Impeller standalone SDK 的 Metal 后端。Android 宿主在 `hosts/android`，用同一份 SDK 的 Vulkan 后端。没有 SDK 时播放器返回 `NotLinked`，测试仍可通过。iOS 的预构建矩阵没有这一列，宿主还没写。

Kotlin 包名 `dev.uifw`、Solid 包名 `@uifw/solid-renderer` 仍是旧前缀，留到改这两层时再换。

## 文档

| 文档 | 内容 |
|---|---|
| `docs/architecture.md` | 仓库分层、一帧、渲染器和控件 |
| `docs/native-phase.md` | 版本 1 字节，以及渲染树、播放器、宿主已经落地的行为 |
| `docs/platform-hosts.md` | macOS、Android、iOS 的线程、表面和输入 |
| `AGENTS.md` | 改代码时的边界。和源码不一致时，以 Rust 源码和测试为准 |

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

依次跑 `cargo test --workspace`、Kotlin 的 `:library:jvmTest` 和 `:gallery:runGallery`、Solid 的字节对比、宿主树测试和 gallery。不打开窗口。

没有 quickjs-ng 源码时，QuickJS 测试走 `NotLinked`。没有 SDK 时，播放器的图形测试不编译。

推到 GitHub 后，[`.github/workflows/check.yml`](.github/workflows/check.yml) 跑同一组检查，并多一步 `cargo fmt --all -- --check`。`hosts/macos` 只在 macOS runner 上编译。工作流不下载 SDK 和解释器源码，也不打 Android 包，也不打开窗口。

## 窗口

```sh
just host
UIFW_DUMP=/tmp/uifw-frame.png just host
just host-js
just android
```

`just host` 打开默认 360×640 的窗口，提交一次夹具绘制列表。`UIFW_DUMP` 把第一帧 Metal 纹理写成 PNG 后退出。比较像素时解码 PNG，不要比较文件字节。`just host-js` 加载 `apps/gallery-solid/dist/counter.js`。`just android` 打 GameActivity APK。

## 目录

```
protocol/                  版本 1 场景字节，以及把它收成绘制列表
core/                      渲染树。Taffy 布局、命中、click 和 drag
engine/                    Impeller 播放器。只读绘制列表
session/                   会话。宿主每帧调用 metrics、pointer、frame、wants_frame、lifecycle
hosts/hostshell/           壳。命令队列、帧调度、渲染线程
hosts/macos/               macOS 窗口
hosts/android/             Android 宿主。GameActivity、AChoreographer、Vulkan
runtimes/quickjs/          quickjs-ng 会话。没有源码时返回 NotLinked
frameworks/solid-renderer/ 版本 1 编码器，以及接到 core 的运行时
frameworks/solid-ui/       Button、ScrollArea、Field
frameworks/kotlin-ui/      Kotlin DSL。目前只开了 JVM 目标
apps/gallery-solid/        Solid 画廊。计数器、呼吸灯、列表和输入。产物 dist/counter.js 在仓库里
apps/gallery-kotlin/       Kotlin 路线的计数器场景
```

仓库根上的 `settings.gradle.kts` 没有 `include`，用来挡住 Gradle 往家目录继续找工程。Kotlin 工程在 `frameworks/kotlin-ui`。

场景字节的金样是 `protocol/tests/fixtures/counter.bin`，208 字节。Rust、Kotlin、TypeScript 三份编码器对这一个文件。格式写在 `docs/native-phase.md`。

## 许可证

产品代码使用 [Apache-2.0](LICENSE)。归属声明在 [NOTICE](NOTICE)。`.agents/skills/rust-best-practice` 使用该目录自己的 MIT 许可证。`third_party` 里的上游代码保持各自的许可证。Impeller SDK 的许可证在取回之后的 `LICENSE.sdk.md`。
