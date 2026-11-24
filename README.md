# Impasto

Impasto 是一个用 Rust 和 TypeScript 构建的跨平台 UI 框架。界面使用 Solid 2.0 TSX 编写，在内置 QuickJS 运行时中执行；UI 树、样式、Taffy 布局、原生响应式图与层树录制在 Rust 引擎完成，最终由 wgpu 渲染到各平台原生表面。

目前支持 macOS（Metal）、iOS 15+（UIKit / Metal）和 Android API 24+（GLES / Vulkan）。Parley 在 UI 队列排版，Raster 根据同一份字形数据绘制组件和 Canvas 文字。

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
- Node.js 22+ 与 pnpm 9
- Python 3（用于依赖边界与打包检查）
- Linux 工作区构建需要 Fontconfig 开发包。无窗口 GPU 验收需要 Vulkan 驱动，Ubuntu 可安装 `libfontconfig-dev` 和 `mesa-vulkan-drivers`
- 平台工具链：macOS / iOS 需要 Xcode 及对应 Rust target；Android 需要 JDK 17、Android SDK 35、NDK 及 `aarch64-linux-android`、`x86_64-linux-android` target

## 快速开始

### 运行内置示例

仓库自带基于 Solid 2.0 的行情模拟盘应用（[`apps/gallery-solid`](apps/gallery-solid)）：

```sh
# macOS 桌面窗口（首次运行会构建 Gallery）
just host

# Android 调试 APK
just android

# iOS 模拟器（构建、安装并校验首帧）
just ios-sim-test --debug --udid <SIMULATOR_UDID>
```

### 创建并运行应用

`crates/cli` 提供 `impasto` 命令行工具，用于创建工程、打包、生成平台工程、运行与测试。可通过 `cargo build -p cli` 构建二进制，或运行 `just cli-dist` 在 `target/cli-dist` 生成可独立分发的目录。

```sh
cargo run -p cli -- create my-app
cd my-app

# 以下假设 impasto 已加入 PATH（run 默认 debug ESM，build 默认 release QuickJS 字节码）
impasto run                         # 在 macOS 启动（可用 -d android / -d ios 指定设备）
impasto build bundle                # 打包独立字节码
impasto build macos                 # 生成 macOS .app
impasto build apk --debug           # 生成 Android APK（另支持 appbundle / aar）
impasto build ios --ios-simulator   # 生成 iOS 模拟器包（另支持 ios-archive / ipa / ios-xcframework）
impasto analyze                     # TypeScript 类型检查
impasto test                        # 在 QuickJS 中运行无头测试
```

在仓库根目录直接执行 `cargo run -p cli -- run` 会启动内置的 Gallery 示例。

## 仓库结构

```text
crates/
├── engine/             UI 树、样式与工具类、Taffy 布局、手势、原生响应式图、层树录制及可选 JS 绑定
├── raster/             提供 Parley 排版和 wgpu 渲染，消费层树并上屏
├── javascript/         独立 QuickJS 运行时、IQJS 字节码编译器与 Web 标准 API 子集
├── embedder/
│   ├── common/         公共宿主：任务队列、帧调度、生命周期、网络与平台模块
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
                        file-system、form、http、i18n、linking、native-plugins、permissions、websocket）

apps/
├── gallery-solid/      模拟盘示例应用
└── android-acceptance/ 移动端验收应用与原生 Fragment 嵌入示例
```

## 开发与检查

常用构建与检查入口定义在 [`justfile`](justfile)：

```sh
just check               # 样式声明检查、依赖边界、Clippy、Rust/JS 测试与标准库契约
just gen                 # 重新生成样式类型与原生模块声明
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
- [移动端验收应用](apps/android-acceptance/README.md) — Android / iOS 深链、恢复、权限与原生 Fragment 验收流程
- [Flutter 性能对照参考](tools/android-flutter-reference/README.md) — Android 行情参考应用的构建与回放
- [AGENTS.md](AGENTS.md) — 目标架构分工与仓库约定

## 许可证

产品代码基于 [Apache-2.0](LICENSE) 发布，版权归属见 [NOTICE](NOTICE)。第三方依赖沿用各自许可证。
