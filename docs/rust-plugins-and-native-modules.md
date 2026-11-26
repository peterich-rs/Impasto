# Rust 业务与原生插件

Impasto 应用从 `src/main.tsx` 启动。应用可以用 `rust/` 暴露私有的 `@app/…` 模块，并通过 `app.toml` 的 `[dependencies]` 声明可复用插件。TS 包和官方插件都位于 `packages/`。CLI 用包根的 `plugin.toml` 识别原生插件。

## 应用工程

`impasto create demo` 创建 `app.toml`、`src/`、`assets/`、`tsconfig.json` 和三个平台壳。用 `--with-rust` 同时创建应用 Rust crate，或进入应用目录运行 `impasto init-rust`。

| 目录 | 用途 |
|---|---|
| `android/` | Android Studio 工程。应用可编辑 Gradle、`MainActivity.kt`、Manifest 和资源。 |
| `ios/`、`macos/` | Xcode `Runner.xcodeproj`。应用可编辑 Swift 入口、Info.plist、签名配置和资源。 |
| `.impasto/` | CLI 生成的 Cargo 工作区、平台插件清单、TypeScript 声明、Gradle 注册文件和 SwiftPM 伞包。 |
| `build/` | 脚本、原生构建与发布产物。 |

将源码与平台壳纳入版本控制，忽略 `.impasto/`、`build/` 和 Impasto 自动生成的 `.cargo/config.toml`。迁移旧应用时，把 `lib/main.tsx` 移到 `src/main.tsx`，把应用 crate 放到 `rust/`，在 `app.toml` 的 `[dependencies]` 中声明插件路径，再运行 `impasto sync`。已有平台壳里的入口和系统权限由应用维护。

`impasto sync` 重新解析依赖并同步所有已启用平台。用 `--platform ios` 之类的选项选择 IDE 使用的声明集。应用启用了多个平台且包含 Rust 模块时，`impasto analyze` 和 `impasto build bundle` 需要明确选择平台。`impasto packages get` 解析依赖后同步。`impasto clean` 清理生成文件和构建产物。

## 插件包

`packages/camera` 是含 Android 与 iOS 源码的参考实现。

```text
packages/camera/
├── plugin.toml
├── tsconfig.json
├── src/index.tsx
├── rust/Cargo.toml
├── rust/src/
├── android/
│   ├── settings.gradle.kts
│   ├── build.gradle.kts
│   ├── consumer-rules.pro
│   └── src/main/
├── ios/
│   ├── Package.swift
│   └── Sources/ImpastoCamera/
└── macos/                         # 支持 macOS 时提供 Swift Package
```

```toml
[plugin]
platforms = ["ios", "android"]
min_impasto_version = "0.1.0"

[rust]
path = "rust"

[android]
path = "android"
package = "dev.impasto.plugins.camera"
plugin_class = "CameraPlugin"

[android.permissions]
"android.permission.CAMERA" = ["@impasto/camera/native#openCamera"]

[ios]
path = "ios"
package = "ImpastoCamera"
module = "ImpastoCamera"
plugin_class = "CameraPlugin"
usage_description_keys = ["NSCameraUsageDescription"]
```

`package.json` 的字符串 `exports` 指向公开 TS 入口。Rust 中的原生模块使用独立的 ESM 标识，公开入口导入它，例如 `@impasto/camera/native`。CLI 从应用清单解析 SDK 内置、本地路径和 Git 标签依赖。旧项目也可用现有的 `package.json`、`file:`、`link:` 和已安装包声明。依赖平台由 `[plugin].platforms` 和应用 `targets` 决定。应用若共用一个 TS 入口，其中的静态插件导入要在构建该入口的每个平台上可用。

用 `[android.permissions]` 声明可按方法收敛的 Android 权限。每条引用采用 `原生模块#导出名`，数组列出需要此权限的入口。正式构建只合并存活入口需要的权限。空数组表示该权限用于插件级初始化。对象、命名空间或 Rust 依赖引用按可能使用的方法保留权限。采用此规则的插件从自己的 Android Manifest 中移出同一声明，由生成的权限库参与应用 Manifest 合并。集成 AAR 时把它和其他 `plugin-*.aar` 同时加入依赖。应用自己的 Manifest 仍可声明应用需要的权限。

包名决定 Cargo crate 和 Android Gradle 模块名。SwiftPM 产品名和 Swift 模块名在 `plugin.toml` 中声明。

| Impasto 包 | Cargo crate | Rust 导入 | Gradle 模块 |
|---|---|---|---|
| `@impasto/permissions` | `impasto-permissions` | `impasto_permissions` | `:impasto_permissions` |
| `@acme/ble-peripheral` | `acme-ble-peripheral` | `acme_ble_peripheral` | `:acme_ble_peripheral` |
| `my-plugin` | `my-plugin` | `my_plugin` | `:my_plugin` |

Android 插件用标准 Gradle Library 声明自己的 Manifest、Maven 依赖、资源和 R8 规则。Kotlin 需要宿主类型时，使用 `compileOnly(project(":impasto-host"))`。应用通过 Gradle 的模块依赖和 Manifest Merger 接入插件。嵌入式 Android 宿主用 `impasto build aar` 生成宿主 AAR 和所选插件的伴随 AAR，消费工程将这些库都加入 Gradle 并在对应窗口调用原生插件注册入口。独立打开相机插件时，`local.properties` 的 `impasto.sdk` 或 `IMPASTO_ROOT` 可指定 SDK。

iOS 与 macOS 插件使用 Swift Package，按需依赖对应的 `ImpastoHost`。SwiftPM 管理 Framework 链接、模块命名空间和资源。生成的 `ImpastoGeneratedPlugins` 包引用声明的产品，调用注册类的 `register(with:)`。应用在 `app.toml` 中填写所需的 Apple 隐私说明，并将文案和 entitlement 配置纳入对应 Runner 工程。SwiftPM 是 Apple 插件的构建入口。

## Rust 导出和窗口原语

`crates/javascript::standard` 使用 QuickJS VM 的绑定实现语言和 Web 标准原语。应用 Rust、官方包和第三方插件统一使用 `impasto` SDK。宏生成的模块契约通过 `javascript::host` 自有值接入运行时，类型导出器可以单独运行。

```rust
#[impasto::module(name = "Orders", specifier = "@app/orders")]
struct Orders;

#[impasto::export]
impl Orders {
    pub fn count() -> u32 {
        42
    }
}

pub fn register(registrar: &mut impasto::PluginRegistrar) {
    registrar.add(Orders::plugin());
}
```

用 `object = true` 同时导出和模块同名的对象及具名方法。用 `#[derive(ImpastoType)]` 声明数据类型，用 `#[derive(ImpastoEvents)]` 声明事件，用 `#[impasto::object]` 导出资源对象。`#[impasto::export]` 支持同步、异步、主线程、阻塞操作和带背压的流。描述导出在开发机运行，`init` 在宿主启动窗口时执行。平台实现可用 `cfg(target_os)` 隔离。需要链接 macOS 原生符号的插件可以用 `cfg(impasto_metadata)` 让描述导出独立运行。

| 窗口原语 | 用法 |
|---|---|
| 调度与取消 | `Runtime` 提供异步、阻塞和主线程调度。用 `Task`、`CancelToken` 跟踪任务。 |
| 沙盒目录 | `AppPaths` 提供文档、应用数据、缓存和临时目录。 |
| 状态和事件 | 读取生命周期、窗口焦点、冷启动 URL、恢复状态及窗口组状态。事件订阅随 `Subscription` 回收。 |
| 平台窗口 | Android 插件通过 `ImpastoHost` 和 `impasto::os::android` 访问 Activity 和 View。宿主提供 Activity Result、权限结果和 Intent 回调。Apple 插件通过 `ImpastoHost` 取得当前控制器或窗口，并处理 URL。 |
| 原生合成 | 外部纹理注册表承载相机等图像源，Platform View 协议将原生视图放入合成布局。 |
| 窗口扩展和遥测 | `insert_extension` 和 `extension` 按 Rust 类型共享窗口内服务。插件可将遥测交给 `DiagnosticsSink`。 |

权限、剪贴板、文件选择、存储和应用网络由各能力包实现。Android 权限在 Manifest 中声明，Apple 的用量说明写入目标 Runner 的 Info.plist。平台系统管理用户授权。

## 跨包依赖

应用使用 TS API 时在 `app.toml` 的 `[dependencies]` 中声明依赖。可复用插件通过自己的 `plugin.toml` 声明扩展依赖。Rust 消费方同时在 `rust/Cargo.toml` 中声明 crate，也可按目标声明依赖。

```toml
[dependencies]
impasto = { version = "0.1.0", registry = "impasto" }

[target.'cfg(any(target_os = "android", target_os = "ios"))'.dependencies]
impasto-permissions = { version = "0.1.0", registry = "impasto" }
```

`impasto add @impasto/http --rust` 在应用清单中声明内置包，更新 Rust 清单并同步。提供 Rust API 的包包含 `plugin.toml` 和 Rust 源码。运行 `impasto sync` 后，CLI 在 `.impasto/cargo/index/` 生成 Cargo 本地索引，在工作区用 `[patch.impasto]` 指向解析到的 SDK 和插件源码。使用 `cargo --offline` 时，其他 crates.io 和 Git 依赖需要已进入 Cargo 缓存。

`impasto create example-plugin --plugin --platforms ios,android` 生成插件骨架、Rust 导出和可运行的无头测试。在插件的 `plugin.toml` 中用 `[dependencies]` 声明传递依赖，路径以该插件为基准。CLI 在 `impasto.lock` 中记录已解析的依赖边和 Git 提交，报告循环和同名包的来源冲突。用 `impasto update [包名]` 刷新 Git 提交，指定单包时可用 `--tag` 更新标签。`impasto remove <包名>` 移除应用直接声明的依赖，其他插件仍使用的包继续留在依赖图中。

IDE 可以加载应用 `rust/Cargo.toml` 或 `.impasto/cargo/Cargo.toml`。应用自带 `.cargo/config.toml` 时，加载生成的工作区以使用 Impasto 自带的注册表配置。Rust 子工程可以省略空 `[workspace]`。

CLI 校验 Rust 依赖对应的 Impasto 插件位于消费方的依赖闭包中，并已启用目标平台。依赖图的拓扑序决定注册顺序。宿主按此顺序构建每个窗口的模块，因此消费方在 `init` 中可以获得依赖包写入的扩展。应用自带的 Rust 模块在插件之后初始化。

```rust
let http = impasto_http::client(ctx)?;
let response = http.send(impasto_http::HttpRequest::get(url)).await?;
```

Android 原生消费方可以引用 `project(":impasto_permissions")`。Apple 原生消费方可以引用 `.impasto/darwin/<platform>/packages/` 下的 SwiftPM 软链接。

## 构建与校验

1. CLI 解析应用清单和包依赖图，校验拓扑和平台，并生成本地索引与所选平台的 registry 和 entry。生成工作区共享应用 `Cargo.lock`。
2. registry 在开发机导出模块方法与类型。CLI 把声明写入 `.impasto/types/`，并将入口与导出写入目标的插件清单。
3. Oxc 降级 TSX，Rolldown 链接应用入口，原生模块保留为 ESM external。Release 编译 IQJS 字节码前会静态核对导出。
4. Rust entry 会注册所选插件并为目标编译宿主。Gradle 将 JNI 库与资源装进 Android 产物，Xcode 将 SwiftPM 伞包与 Rust 静态库装进 Apple 应用。
5. 宿主在运行脚本前核对实际登记的模块描述。Android 和 iOS 调试更新继续使用已部署版本的描述。

`examples/native-order` 覆盖应用内 Rust 对 `impasto-http` 的跨包调用、对象与流。`examples/native-camera` 覆盖相机授权、Swift/Kotlin 桥接、扫码、JPEG 与外部纹理。相机的原生采集验收需要支持相机的 iOS 或 Android 设备。

```sh
cargo test -p cli
just boundaries
just check
cargo fmt --all -- --check
cargo clippy -p embedder-macos --all-targets -- -D warnings
just ios-build-check
just android-build-check
```
