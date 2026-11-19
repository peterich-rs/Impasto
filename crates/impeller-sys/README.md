# Impeller SDK

`impeller-sys` 提供 Impeller 的原始 C 绑定和原生链接。绘制代码通过 Cargo 依赖名 `impeller` 调用。

- [include/impeller.h](include/impeller.h) 保存锁定 Flutter 提交的公开 C 头文件。[Flutter 许可证](include/LICENSE)
- [src/bindings.rs](src/bindings.rs) 和 [src/bindings_msvc.rs](src/bindings_msvc.rs) 是由头文件生成的 Rust FFI。Cargo 按目标 ABI 选择文件，开启 `link` 特性时链接对应 SDK 的原生库。

`sdk.lock` 固定 Flutter 源码提交和 SDK 版本。[Impeller SDK 工作流](../../.github/workflows/impeller-sdk.yml) 使用 Flutter 的 `DEPS` 同步依赖，再用 GN 和 Ninja 编译 standalone interop。构建时核对上游头文件及各目标生成的绑定与源码树一致。每个 SDK 包包含头文件、绑定、原生库、许可证和版本信息，工作流从归档重新链接并验证后可将整套包与 SHA-256 清单发布为版本化 GitHub Release。

```sh
just impeller                          # 准备当前桌面目标的 SDK
just impeller aarch64-linux-android    # 准备 Android arm64 的 SDK
```

首次使用时下载对应版本的 SDK 并校验 SHA-256。已准备好的包从 `third_party/impeller-sdk/<版本>/<Rust 目标>/` 复用。`just host`、macOS 应用打包和 Android Gradle 构建会自动准备各自的 SDK。版本首次发布前，可以在本地从源码准备。

## 本地源码构建

准备 Python 3、Chromium `depot_tools` 中的 `gclient`、libclang 和对应平台工具链。Apple 目标还需要 Xcode Metal Toolchain。[Flutter Engine 开发环境](https://github.com/flutter/flutter/blob/77f043bc63196b9979cc09c5b97bb6d9b00be34d/docs/engine/contributing/Setting-up-the-Engine-development-environment.md) · [Xcode 组件安装](https://developer.apple.com/documentation/xcode/downloading-and-installing-additional-xcode-components)

```sh
cargo install --locked bindgen-cli --version 0.72.1
just impeller-source
just impeller-source aarch64-apple-ios-sim
```

脚本使用 `build/impeller-source/` 缓存源码，使用 `third_party/impeller-build/` 缓存原生构建结果，并将最终 SDK 安装到日常构建使用的目录。CI 目标列表由 `python3 tools/impeller_sdk.py matrix` 输出。SDK 构建目标与已接入的应用宿主分别维护，当前应用宿主为 macOS 和 Android。

iOS SDK 使用 Metal，最低部署版本为 iOS 15.0。[`patches/ios-interop.patch`](patches/ios-interop.patch) 补齐锁定版本对 GLES 和 Vulkan 的条件编译，使这些后端禁用时 C API 仍可链接。

更新 Flutter 提交时，从对应源码更新 `include/impeller.h`，再运行 `python3 tools/impeller_sdk.py bindings` 重新生成 Rust 绑定。
