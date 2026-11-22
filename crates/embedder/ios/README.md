# iOS 构建与 UIKit 嵌入

宿主最低部署目标为 iOS/iPadOS 15.0。构建需要 macOS、Xcode 和对应 Rust 目标。Apple Silicon 模拟器使用 `aarch64-apple-ios-sim`，Intel 模拟器使用 `x86_64-apple-ios`，设备使用 `aarch64-apple-ios`。

## 构建和安装

在应用根目录执行命令。构建目录中的 Xcode 工程由 CLI 生成。直接运行使用 debug 源码，构建缺省为 release IQJS 字节码。

```sh
impasto run -d ios
impasto run -d ios:<UDID> --watch
impasto build ios --ios-simulator
impasto build ios
impasto build ios-archive
impasto build ipa --export-options ExportOptions.plist
```

- `run -d ios` 选择当前启动的模拟器。指定 `-d ios:<UDID>` 可选择其他模拟器或配对真机。真机安装由 Xcode 的 `devicectl` 执行。
- `--watch` 在 debug 应用启动时建立有独立 UUID 的开发会话。脚本改动使用一次性缓存文件发送 ESM，宿主确认候选会话成功后继续运行。候选报错时保留上一次成功画面。资产、配置和宿主代码变动重新构建并安装。传输通过 Xcode 的模拟器或设备应用容器，真机断开后会等到重连再继续。Release 宿主只执行安装包内已编译的脚本。
- 真机构建需要在 `[ios]` 设置 `development_team`，并在本机准备匹配的签名身份和 provisioning profile。默认由 Xcode 选择已可用的自动签名配置。手动签名可加 `provisioning_profile` 和 `signing_certificate`。
- 归档输出带签名的 `.xcarchive`。同一目录包含 JavaScript source map 和标记 app/dSYM UUID 的 `build.json`。IPA 导出使用 Xcode 本地导出设置，`destination` 省略或设为 `export`。 [Apple 应用分发](https://developer.apple.com/documentation/xcode/distributing-your-app-for-beta-testing-and-releases)
- iOS Release 构建保留编译阶段过程宏的符号，避免 Xcode 27 的 strip 产物在宿主 dyld 上加载失败。应用和 dSYM 仍由 Xcode 按目标配置生成。 [Rust 工具链问题](https://github.com/rust-lang/rust/issues/157750)

## 应用配置

```toml
[permissions]
permissions = ["ios"]
linking = ["ios"]
native-plugins = ["ios"]

[ios]
bundle_id = "com.example.impasto"
development_team = "ABCDE12345"
build_number = 1
device_families = [1, 2]
url_schemes = ["myapp"]
associated_domains = ["applinks:example.com"]
share_window_state = false
present_on_external_display = false

[ios.usage_descriptions]
NSCameraUsageDescription = "用于拍摄用户选择的照片。"
NSMicrophoneUsageDescription = "用于录制用户选择的视频声音。"
NSPhotoLibraryUsageDescription = "用于选取用户选择的照片。"
```

`device_families` 的 1、2 分别代表 iPhone、iPad。可选的 `launch_storyboard` 和 `privacy_manifest` 指向应用自己的 `.storyboard` 和 `.xcprivacy`。CLI 将应用的隐私声明合并到宿主要求的清单。关联域需要与本机签名配置和服务器上的 AASA 配合。 [Apple 隐私清单](https://developer.apple.com/documentation/bundleresources/privacy-manifest-files) · [Apple 关联域](https://developer.apple.com/documentation/xcode/supporting-associated-domains)

开启 `present_on_external_display` 后，生成宿主可为非交互外接屏场景启动独立会话。用 iOS 27 SDK 构建时通过 Scene Accessory 注册，旧系统使用外接屏窗口角色。`share_window_state` 控制这些会话是否与主窗口共享显式 `WindowState`。 [Apple 外接屏场景](https://developer.apple.com/documentation/uikit/presenting-content-on-a-connected-display)

## 消费嵌入 SDK

安装设备和两种模拟器架构的 Rust 目标，使用 `impasto build ios-xcframework` 打包 SDK。SDK 包含设备 arm64 与模拟器 arm64/x86_64 两个平台切片及 Swift 模块接口。Xcode 应用 target 的 Frameworks, Libraries, and Embedded Content 中选择 Embed & Sign。应用提供自己的资源目录及授权配置。

```swift
import UIKit
import ImpastoHost

@MainActor
func installImpasto(in parent: UIViewController, resources: URL) {
    let host = ImpastoHost(resources: resources)
    let content = host.makeViewController()
    parent.addChild(content)
    content.view.translatesAutoresizingMaskIntoConstraints = false
    parent.view.addSubview(content.view)
    NSLayoutConstraint.activate([
        content.view.leadingAnchor.constraint(equalTo: parent.view.leadingAnchor),
        content.view.trailingAnchor.constraint(equalTo: parent.view.trailingAnchor),
        content.view.topAnchor.constraint(equalTo: parent.view.topAnchor),
        content.view.bottomAnchor.constraint(equalTo: parent.view.bottomAnchor)
    ])
    content.didMove(toParent: parent)
}
```

资源目录与独立宿主使用同一格式，包含 `app.js` 或安装包中的 `app.bin`、`assets/` 及可选 `app.modules`。SDK 带有公开 C 头 `Headers/impasto_ios.h` 和构建 Swift 客户可用的模块接口。 [UIKit 接入和线程边界](../../../docs/platform-hosts.md#ios-宿主)

- `try await host.prepare()` 可在挂接 View 之前等待脚本会话就绪。取消当前任务只结束这次等待，`host.close()` 结束整个会话。
- 在主 actor 调用 `try host.captureSnapshot()` 获取当前控制器的 sRGB 图像。画面包含上次提交的原生视图和媒体，尺寸为当前 drawable 像素。可开启 `content.hidesContentInAppSwitcher` 在场景失活时覆盖内容。
- `just ios-sim-test --debug --udid <UDID>` 验证 Gallery 调试包的可见首帧，传 `--release` 验证 IQJS 包。已构建的模拟器应用可用 `--app <路径>` 复验。

### JavaScript 调用宿主插件

在应用的 `[permissions]` 中显式设置 `native-plugins = ["ios"]`，然后在调用
`prepare()` 或显示控制器之前，给对应的 `ImpastoHost` 注册 Swift 插件。直接管理
SDK 资源目录的 UIKit 应用在 `app.modules` 中加入 `native-plugins`。每个宿主拥有
自己的插件注册表。省略授权或设为 `false` 时，应用不加载该模块。

```swift
import Foundation
import ImpastoHost

@MainActor
final class EchoPlugin: ImpastoPlugin {
    let identifier = "echo"

    func handle(_ call: ImpastoPluginCall) async throws -> Data {
        guard call.method == "repeat" else { throw EchoError.unsupportedMethod }
        return call.payload
    }

    enum EchoError: LocalizedError {
        case unsupportedMethod
        var errorDescription: String? { "EchoPlugin 不提供这个方法" }
    }
}

// 在宿主启动前调用。每个 identifier 只能注册一次。
host.registerPlugin(EchoPlugin())
```

```ts
import { invokeNativePlugin } from "@impasto/native-plugins";

const echoed = await invokeNativePlugin("echo", "repeat", { message: "你好" });
```

Swift 在主 actor 执行 `handle`，可以通过 `JSONDecoder` 读取 `call.payload`，
通过 `JSONEncoder` 返回结果。省略参数时传入 JSON `null`。请求和返回分别限制为
32 KiB 和 32 层 JSON 嵌套。每个宿主最多保留 32 个调用。未知插件、未提供方法、
超时或无效 JSON 会拒绝 Promise。10 秒超时、旧场景解除绑定或宿主关闭都会取消
原生任务，迟到的返回会被丢弃。插件在异步执行副作用前应检查任务是否取消。
