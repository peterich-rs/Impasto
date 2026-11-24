# AGENTS.md

## 目标架构与目录

Impasto 是跨平台 UI 框架。下表按职责与现有目录对应；具体接口和行为以源码与测试为准。

| 部分 | 路径 | 所有权 |
|---|---|---|
| `driver` | `crates/cli`、`tools/` | 创建应用工程，编译和打包，生成平台工程，部署应用，以及为热重载监听变更、构建并发送更新产物。 |
| `embedder` | `crates/embedder/{common,macos,ios,android}` | 创建平台窗口和框架执行环境，启动应用并管理生命周期，提供原生绘制表面、vsync、输入、IME、无障碍和平台插件。 |
| `engine` | `crates/engine` | 接收应用框架提交的 UI 树变更，管理 UI 状态、样式、脏标记和帧内阶段，完成命中测试、手势、测量、Taffy 布局和绘制录制，产出层树。 |
| `raster` | `crates/raster` | 提供 Parley 排版与 wgpu 渲染，消费层树并上屏。UI 队列拥有排版上下文，Raster 队列光栅化已排版的字形，两者共享窗口的字体库。 |
| `javascript` | `crates/javascript` | 独立 JavaScript 运行时、IQJS 字节码编译器、Web 标准 API 子集及纯 `std` 宿主交换契约。 |
| 应用层 | `packages/*`、`apps/*` | Solid 2.0 通用渲染器、`@impasto/reactive` 垫片、UI 组件、路由、平台能力封装、调试面板与示例/验收应用。 |

### 核心边界与数据流

- **运行时与宿主分离**：`crates/javascript` 默认提供独立运行时；关闭默认 feature 后仅保留纯 `std` 的 `javascript::host`。运行时仅内置语言与 Web 基础原语（`console`、定时器、`performance`、`events`、`encoding`、`url`、`streams`），不包含网络实现；`engine::js` 通过可选 `js` feature 接入该运行时，提交树变更并将 UI 事件交给脚本。HTTP（`@impasto/http`）与 WebSocket（`@impasto/websocket`）等网络与平台能力由 `embedder` 原生模块提供，并通过 `packages/*` 导出给应用层。
- **线程与帧流水线**：平台线程通过队列交付自有数据（输入、生命周期、视口、vsync）。UI 队列通过 `engine::session::Session` 同步推进脚本、响应式图、布局与录制，产出不可变 `engine::scene::LayerTree`。Raster 队列消费层树并实现 `engine::text::Shaper`。公共宿主支持 `shared`（默认）、`split`、`platform-ui` 三种线程拓扑（环境变量 `IMPASTO_THREAD_TOPOLOGY`），跨线程传递一律使用自有数据。
- **TSX 编译与响应式**：[`tools/pack.mjs`](tools/pack.mjs) 将 `solid-js` 解析到 `@impasto/reactive`，`@solidjs/babel-plugin` 生成 universal 输出，保留 `scene` 和 `reactive` 为运行时模块。应用 bundle 严禁混入 Solid 调度器或 `@solidjs/universal`（由 `assertNoScheduler` 检查）。

## 硬性约束

### 依赖隔离（`just boundaries`）

修改 `Cargo.toml` 或跨 crate 引用时，必须守住以下边界：

- `javascript --no-default-features` 保持纯 `std`，不引入任何外部依赖。
- `javascript --features runtime` 不得依赖 `engine`、`raster`、`embedder*` 或图形、排版依赖，源码中不得出现 UI 模块路径。
- `engine` 默认构建不得依赖 `rquickjs` / `quickjs-jit`、`raster`、`wgpu` 或任何平台/渲染库。
- `raster`不得依赖 `rquickjs` / `quickjs-jit`。

### Rust Lint 与安全规则

workspace lint 见 [`Cargo.toml`](Cargo.toml) 与 [`clippy.toml`](clippy.toml)：

- 生产代码禁止 `unsafe`（`unsafe_code = "deny"`）、`unwrap()` 与 `expect()`（`unwrap_used` / `expect_used = "deny"`，仅测试放行）以及冗余路径限定（`unused_qualifications = "deny"`）。
- 确需局部放行时必须附带 `reason`；所有 `unsafe` 块必须写清安全不变式注释。
- 提交前保持 `RUSTFLAGS="-D warnings"` 与 `cargo fmt --all -- --check` 零告警。

### 生成文件与单一事实源

以下产物由工具生成并受 CI 校验，不要手改，改完源定义后运行对应生成与检查命令：

| 生成产物 | 单一事实源 | 生成与校验 |
|---|---|---|
| `packages/solid-renderer/src/style.gen.ts`、[`docs/style-utilities.gen.md`](docs/style-utilities.gen.md) | `crates/engine/src/style/` | `just gen` / `just style-check` |
| `packages/*/src/native.d.ts` | `crates/embedder/common` 中的 `Module` 元数据 | `just gen` / `cargo run --locked -p embedder --bin module-stubs -- --check`、`node tools/check-native-module-types.mjs` |
| JS 标准库声明与运行时一致面 | `crates/javascript/src/standard/` | `just standard-check` |

`apps/gallery-solid/dist/quotes.js` 不提交 git，但部分 Rust 测试会读取该产物；`just test` 会自动执行 `just gallery-dist`，单独跑 `cargo test` 前需先确保已构建。

## 验证入口

完整构建与测试配方见 [`justfile`](justfile)，CI 门禁见 [`.github/workflows/check.yml`](.github/workflows/check.yml)。修改后按影响范围运行对应检查：

- **常规门禁**：`just check`、`cargo fmt --all -- --check`
- **注意 `just check` 未覆盖的 CI 项**：`cargo run --locked -p embedder --bin module-stubs -- --check`、`node tools/check-native-module-types.mjs`，以及 macOS 下的 `embedder-macos` 检查（`just clippy` 与 Linux CI 保持一致，默认排除 `embedder-macos`）
- **光栅后端**：在可用 Metal 或 Vulkan/GLES 设备上运行 `cargo test -p raster`
- **macOS 宿主**：`cargo clippy -p embedder-macos --all-targets -- -D warnings`、`cargo test -p embedder-macos`；界面或渲染改动用 `just host`（可设 `IMPASTO_DUMP=/tmp/frame.png` 导出首帧）验证画面
- **iOS 宿主**：`just ios-build-check` 构建模拟器包。本地启动模拟器后，用 `just ios-sim-test --debug --udid <UDID>` 验证首帧。构建与嵌入细则见 [iOS README](crates/embedder/ios/README.md)
- **Android 宿主**：`just android-build-check` 构建、lint 并校验 APK。本地连接真机或模拟器时，用 `just android-device-test` 运行仪器测试
- **JS / Solid 包**：`just solid`、`just solid-app`

CI 的运行测试限于逻辑用例与 `raster` 的 headless 渲染，移动端任务只构建和校验产物。真机和模拟器的运行结果及性能样本仅用于本地平台验收和调优。

## 工作约定

- 文档写清适用范围和前提，让读者可以独立理解。修改实现时同步核对相关说明，直接替换过时内容；不为历史设计或一次性排查保留冗余文档。
- 写 Rust 时使用 `/rust-best-practice`；设计控件或 Gallery 界面时使用 `/unslop`，并用对应宿主验证画面；整理提交历史时使用 `/git-commit`。
- 优先复用许可证合适的发布依赖。产品代码放在 `third_party/` 以外。本地依赖与构建产物留在本地。
- 删除实现前核对全部调用者与生成/测试依赖，修改后运行受影响的检查。
