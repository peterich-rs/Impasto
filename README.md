# Impasto

Impasto 是一个用 Rust 和 TypeScript 构建的跨平台 UI 框架。界面使用 Solid 2.0 TSX 编写，在内置 QuickJS 运行时中执行；UI 树、样式、Taffy 布局、原生响应式图与层树录制在 Rust 引擎完成，最终由 wgpu 渲染到各平台原生表面。

目前支持 macOS 13+（Metal）、iOS 15+（UIKit / Metal）和 Android API 24+（GLES / Vulkan）。Parley 在 UI 队列排版，Raster 根据同一份字形数据绘制组件和 Canvas 文字。

```tsx
import { createSignal } from "solid-js";
import { boot } from "@impasto/solid-renderer";
import { Button, Column, Text } from "@impasto/solid-ui";

function Main() {
  const [count, setCount] = createSignal(0);

  return (
    <Column style={{ width: "100%", height: "100%", padding: 24, gap: 12 }}>
      <Text style={{ fontSize: 20 }}>计数：{count()}</Text>
      <Button label="+1" variant="primary" onClick={() => setCount((n) => n + 1)} />
    </Column>
  );
}

boot(Main);
```

滚动 API 的选择、合成回退原因和验收入口见 [滚动与合成线程](docs/scroll-compositor.md)。

## 环境要求

- Rust stable（见 [`rust-toolchain.toml`](rust-toolchain.toml)）
- [just](https://github.com/casey/just)
- Python 3（用于依赖边界与打包检查）
- Linux 工作区构建需要 Fontconfig 开发包。无窗口 GPU 验收需要 Vulkan 驱动，Ubuntu 可安装 `libfontconfig-dev` 和 `mesa-vulkan-drivers`
- 平台工具链：macOS / iOS 需要 Xcode 及对应 Rust target；Android 需要 JDK 17、Android SDK 35、NDK 及 `aarch64-linux-android`、`x86_64-linux-android` target

## 快速开始

### 运行内置示例

仓库自带基于 Solid 2.0 的行情模拟盘应用。 [Gallery 示例](examples/gallery-solid)

```sh
# macOS 桌面窗口（首次运行会构建 Gallery）
just host

# Android 调试 APK
just android

# iOS 模拟器（构建、安装并校验首帧）
just ios-sim-test --debug --udid <SIMULATOR_UDID>
```

在仓库根目录执行 `just solid` 或 `just solid-app`，在无头 QuickJS Session 中运行框架包或 Gallery 测试。运行 `impasto analyze --platform macos` 会生成应用声明并用原生 TypeScript 检查应用源码。

### 创建并运行应用

`crates/cli` 提供 `impasto` 命令行工具，用于初始化应用和平台壳、同步插件、构建与运行。可通过 `cargo build -p cli` 构建二进制，或运行 `just cli-dist` 在 `target/cli-dist` 生成可独立分发的目录。

```sh
cargo run -p cli -- create my-app
cd my-app

# 以下假设 impasto 已加入 PATH（run 默认 debug ESM，build 默认 release QuickJS 字节码）
impasto run                         # 在 macOS 启动调试会话，保存后自动刷新
impasto devices                     # 列出 Android 和 iOS 目标及启动命令
impasto doctor                      # 检查本机开发环境
impasto build bundle --platform macos --analyze-size
impasto build macos                 # 生成 macOS .app
impasto build apk --debug           # 生成 Android APK（另支持 appbundle / aar）
impasto build ios --ios-simulator   # 生成 iOS 模拟器包（另支持 ios-archive / ipa / ios-xcframework）
impasto sync --platform macos       # 生成该平台的插件清单与 IDE 类型
impasto analyze --all-targets --deny-warnings
impasto test --watch -k "payment"   # 监听相关文件并按用例名过滤
# impasto init-rust                   # 按需添加应用 Rust crate
```

应用源码放在 `src/`，应用内 Rust crate 放在 `rust/`。用 Android Studio 打开 `android/`，用 Xcode 打开 `ios/Runner.xcodeproj` 或 `macos/Runner.xcodeproj`。CLI 将插件聚合代码、类型和平台链接配置写入 `.impasto/`。新应用在 `app.toml` 的 `[dependencies]` 中声明依赖。使用 `impasto add @impasto/http` 添加内置插件，也可使用本地 `path` 或 HTTPS Git 标签。Git 依赖的提交记录在 `impasto.lock`。 [Rust 业务与原生插件](docs/rust-plugins-and-native-modules.md)

从仓库根目录运行 Gallery 时，执行 `cargo run -p cli -- --project examples/gallery-solid run`。在应用目录中可直接执行 `impasto run`，用 `-d android`、`-d ios` 或 `-d ios:<UDID>` 选择移动设备。调试会话会自动应用兼容的脚本与资产更新，将设备日志输出到终端，并在设备上显示构建或脚本错误。原生或平台工程变更会重新构建和部署。使用 `--no-watch` 单次启动调试版本，或用 `--release` 启动发布版本。

应用的 TSX 由 Oxc 降级并由 Rolldown 链接。显式导入 PascalCase JSX 组件，例如 `import { View } from "@impasto/solid-ui"`。Release 构建生成 `app.bin` 和 `app.sym`。字面量 `import()` 对应的字节码按需从 `app.bin` 加载。路由可用 `lazy: () => import("./settings.tsx")` 加载默认导出的组件。用 `impasto symbolicate <app.sym> --stack <堆栈>` 解析带模块名的 QuickJS 调用栈。`--analyze-size` 生成 `bundle-size.json`，列出实际分包大小、按模块分摊的字节码大小与共享样式池归因。用 `--size-json <路径>` 指定报告位置。

`impasto analyze` 首次运行会安装并校验固定版本的原生 TypeScript 7。可使用 `--tsc` 或 `IMPASTO_TSC` 指定已有的原生检查器。`--all-targets` 检查应用声明的各平台，`--deny-warnings` 将项目编译警告视为错误。CLI 发布包附带检查器、框架源码和 DevTools Wasm 编译器。调试时默认生成 Inspector 源码位置，在独立客户端用 `impasto devtools --source-root <项目路径> --editor cursor` 指定编辑器和本地源码。将 `bundle-size.json` 导入独立客户端的 Bundle size 面板可查看模块体积图。

`impasto test` 发现 `src/` 与 `tests/` 中的 `*.test.ts` 和 `*.test.tsx`。从 `impasto:test` 导入 `test`、`screen` 与 `fire`，用 `await screen.mount(...)` 挂载组件，再等待查询或输入驱动。用 `clock.install()` 接管会话时间，再用 `await clock.advance(ms)` 驱动定时器和帧。测试宿主提供隔离的临时目录，支持链接应用自己的 Rust 插件。`just solid` 和 `just solid-app` 分别运行框架包和 Gallery 的无头测试。

`impasto sync` 为 `locales/*.json` 和自定义主题 Token 生成类型声明。词条可嵌套，插值使用 `{name}`。用 `t("app.welcome", { name })` 读取词条，在 `[i18n]` 中用 `default_locale` 设置回退语言。Release 按引用保留静态词条。键带有可静态识别的命名空间时，动态查询仅保留对应命名空间。主题可在 `[theme.tokens]` 或 `src/theme.ts` 中声明 `colors` 和 `lengths` 键列表。TS 文件须导出 `const themeTokens` 字面量。通过 `extendThemeTokens` 提供当前值，再用 `style={{ gap: { token: "space.trade" } }}` 绑定原生样式。

## 仓库结构

```text
crates/
├── engine/             UI 树、样式与工具类、Taffy 布局、手势、原生响应式图、层树录制及可选 JS 绑定
├── raster/             提供 Parley 排版和 wgpu 渲染，消费层树并上屏
├── javascript/         独立 QuickJS 运行时、IQJS 字节码编译器与 Web 标准 API 子集
├── embedder/
│   ├── common/         公共宿主：任务队列、帧调度、生命周期、网络与平台服务
│   ├── macos/          macOS 宿主（AppKit / CVDisplayLink / Metal）
│   ├── ios/            iOS 宿主（UIKit / CADisplayLink / Metal）
│   └── android/        Android 宿主（Kotlin / JNI / Choreographer / GLES & Vulkan）
└── cli/                impasto CLI 驱动工具

packages/
├── reactive/           接入原生响应式图的 solid-js 垫片
├── solid-renderer/     面向引擎 scene 的 Solid 通用渲染器
├── solid-ui/           基础布局、控件、Canvas、主题与浮层组件
├── solid-router/       路由栈与转场
├── devtools/           应用内调试面板
└── *                   平台能力与工具包（app-state、async-storage、clipboard、
                        camera、file-system、form、http、i18n、linking、permissions、websocket）

examples/
├── gallery-solid/      模拟盘示例应用
├── native-order/       应用内 Rust 验收
├── native-camera/      Android 与 iOS 相机插件验收
└── android-acceptance/ 移动端验收应用与原生 Fragment 嵌入示例
```

## 开发与检查

常用构建与检查入口定义在 [`justfile`](justfile)：

```sh
just check               # 样式声明检查、依赖边界、Clippy、Rust/JS 测试与标准库契约
just gen                 # 重新生成样式类型
just android-build-check # 构建、lint 并校验 Android APK
just ios-build-check     # 构建 Gallery 的 iOS 模拟器包
```

在支持 Metal 或 Vulkan/GLES 的设备上运行绘制后端测试：

```sh
cargo test -p raster
```

## 相关文档

- [样式属性与工具类清单](docs/style-utilities.gen.md) — `just gen` 生成的受支持样式属性与工具类列表
- [wgpu 渲染管线](docs/wgpu-rendering-pipeline.md) — 渲染与排版设计、平台合成和验收路径
- [iOS 构建与 UIKit 嵌入](crates/embedder/ios/README.md) — `[ios]` 配置、签名归档、XCFramework 与 Swift 插件
- [移动端验收应用](examples/android-acceptance/README.md) — Android / iOS 深链、恢复、权限与原生 Fragment 验收流程
- [Flutter 性能对照参考](tools/android-flutter-reference/README.md) — Android 行情参考应用的构建与回放
- [AGENTS.md](AGENTS.md) — 目标架构分工与仓库约定

## 许可证

产品代码基于 [Apache-2.0](LICENSE) 发布，版权归属见 [NOTICE](NOTICE)。第三方依赖沿用各自许可证。
