# Impeller SDK

`impeller-sys` 提供 Impeller 的原始 C 绑定和原生链接。绘制代码通过 Cargo 依赖名 `impeller` 调用。

- [include/impeller.h](include/impeller.h) 保存锁定 Flutter 提交的公开 C 头文件。[Flutter 许可证](include/LICENSE)
- [src/bindings.rs](src/bindings.rs) 和 [src/bindings_msvc.rs](src/bindings_msvc.rs) 是由头文件生成的 Rust FFI。Cargo 按目标 ABI 选择文件，开启 `link` 特性时链接对应 SDK 的原生库。

`sdk.lock` 的 `source` 记录完整的 Flutter 源码提交，`version` 记录 Impasto 的 SDK 包版本，用于包内元数据、本地缓存和 GitHub Release。锁文件同时记录各目标归档的 SHA-256。[Impeller SDK 工作流](../../.github/workflows/impeller-sdk.yml) 使用 Flutter 的 `DEPS` 同步依赖，再用 GN 和 Ninja 编译 standalone interop。构建时核对上游头文件及各目标生成的绑定与源码树一致。每个 SDK 包包含头文件、绑定、原生库、许可证和版本信息，工作流会解压归档并验证链接，也支持将整套包发布为版本化 GitHub Release。

仓库按 Flutter 提交组织预构建包。[prebuilt/flutter-77f043bc6319/](prebuilt/flutter-77f043bc6319/) 收录了从该提交构建并通过 CI 验证的 11 个 SDK 归档，覆盖 macOS、Android、iOS、Linux 和 Windows 的目标矩阵。[Flutter 源码提交](https://github.com/flutter/flutter/commit/77f043bc63196b9979cc09c5b97bb6d9b00be34d) · [构建与验证记录](https://github.com/peterich-rs/Impasto/actions/runs/44658054748)

```sh
just impeller                          # 准备当前桌面目标的 SDK
just impeller aarch64-linux-android    # 准备 Android arm64 的 SDK
```

首次使用时，`just impeller` 从仓库读取目标归档，校验 SHA-256 后解压到 `third_party/impeller-sdk/<版本>/<Rust 目标>/`。后续构建复用这个目录。`just host`、macOS 应用打包和 Android Gradle 构建会自动准备各自的 SDK。直接调用 Cargo 并开启 Impeller 链接前，先为对应目标运行一次 `just impeller`。

更新预构建包时，从已通过验证的 CI 运行下载各目标产物，保留其中的内层 SDK ZIP。用 `sdk.lock` 中 `source` 的前 12 位命名 `prebuilt/flutter-<提交>/`。运行 `python3 tools/impeller_sdk.py index <归档所在目录>` 生成校验值，再把包的相对路径和校验值写入仓库的 `sdk.lock`。

## 本地源码构建

准备 Python 3、Chromium `depot_tools` 中的 `gclient`、libclang 和对应平台工具链。Apple 目标还需要 Xcode Metal Toolchain。[Flutter Engine 开发环境](https://github.com/flutter/flutter/blob/77f043bc63196b9979cc09c5b97bb6d9b00be34d/docs/engine/contributing/Setting-up-the-Engine-development-environment.md) · [Xcode 组件安装](https://developer.apple.com/documentation/xcode/downloading-and-installing-additional-xcode-components)

```sh
cargo install --locked bindgen-cli --version 0.72.1
just impeller-source
just impeller-source aarch64-apple-ios-sim
```

脚本使用 `build/impeller-source/` 缓存源码，使用 `third_party/impeller-build/` 缓存原生构建结果，并将最终 SDK 安装到日常构建使用的目录。CI 目标列表由 `python3 tools/impeller_sdk.py matrix` 输出。SDK 构建目标与已接入的应用宿主分别维护，当前应用宿主为 macOS 和 Android。

iOS SDK 使用 Metal，最低部署版本为 iOS 15.0。[`patches/ios-interop.patch`](patches/ios-interop.patch) 补齐锁定版本对 GLES 和 Vulkan 的条件编译，使这些后端禁用时 C API 仍可链接。

Windows 构建通过 [`patches/windows-modf.patch`](patches/windows-modf.patch) 显式丢弃 `std::modf` 返回的小数部分，保留直线像素对齐使用的整数部分。

更新 Flutter 提交时，从对应源码更新 `include/impeller.h`，再运行 `python3 tools/impeller_sdk.py bindings` 重新生成 Rust 绑定。
