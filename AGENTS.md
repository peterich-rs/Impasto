# AGENTS.md

## 目标架构与目录

Impasto 是跨平台 UI 框架。下表按职责与现有目录对应；具体接口和行为以源码与测试为准。

| 部分 | 路径 | 所有权 |
|---|---|---|
| `driver` | `crates/cli`、`tools/` | 创建应用工程，编译和打包，初始化平台壳，同步生成配置，部署应用，以及为热重载监听变更、构建并发送更新产物。 |
| `embedder` | `crates/embedder/{common,macos,ios,android}` | 创建平台窗口和框架执行环境，启动应用并管理生命周期，提供原生绘制表面、vsync、输入、IME、无障碍和平台插件。 |
| `engine` | `crates/engine` | 接收应用框架提交的 UI 树变更，管理 UI 状态、样式、脏标记和帧内阶段，完成命中测试、手势、测量、Taffy 布局和绘制录制，产出层树。 |
| `raster` | `crates/raster` | 提供 Parley 排版与 wgpu 渲染，消费层树并上屏。UI 队列拥有排版上下文，Raster 队列光栅化已排版的字形，两者共享窗口的字体库。 |
| `javascript` | `crates/javascript` | 独立 JavaScript 运行时、IQJS 字节码编译器、Web 标准 API 子集及纯 `std` 宿主交换契约。 |
| 应用层 | `packages/*` | Solid 2.0 通用渲染器、`@impasto/reactive` 垫片、UI 组件、路由、平台能力封装与调试面板。 |
| 示例应用 | `examples/*` | 可独立构建的框架示例与平台验收应用。 |

### 核心边界与数据流

- **运行时与宿主分离**：`crates/javascript` 默认提供独立运行时；关闭默认 feature 后仅保留纯 `std` 的 `javascript::host`。运行时仅内置语言与 Web 基础原语（`console`、定时器、`performance`、`events`、`encoding`、`url`、`streams`），不包含网络实现；`engine::js` 通过可选 `js` feature 接入该运行时，提交树变更并将 UI 事件交给脚本。`embedder` 负责窗口、渲染资源与遥测。
- **原生扩展与宿主原语**：官方包、第三方插件和应用 Rust 通过 `#[impasto::module]` 及导出宏绑定 JavaScript。包使用 `PluginContext` 的调度、目录、窗口事件、平台容器、合成和泛型扩展原语。应用网络、权限、剪贴板等能力由 `packages/*` 中的插件实现。
- **线程与帧流水线**：平台线程通过 `TaskRunner` 队列交付自有数据（输入、生命周期、视口、vsync）。UI 队列通过 `engine::session::Session` 同步推进脚本、响应式图、布局与录制，产出不可变 `engine::scene::LayerTree`。Raster 队列消费层树并实现 `engine::text::Shaper`。线程绑定以可切换的 `TaskRunner` 为抽象边界。Raster 独立运行，UI/JS 在后台冷启动并提交首帧后迁移至平台主线程。跨线程传递一律使用自有数据。
- **TSX 编译与响应式**：[Bundler](crates/bundler) 通过 Oxc 降级显式导入的 JSX 组件，由 Rolldown 将 `solid-js` 解析到 `@impasto/reactive`，保留 `scene`、`reactive` 与插件原生模块。编译器和测试驱动仅用于开发宿主。

### 应用与插件

- 应用使用 `src/`、可选 `rust/` 和可编辑的 `android/`、`ios/`、`macos/`。平台壳纳入版本控制，`.impasto/` 由 CLI 同步，`build/` 存放构建产物。
- 官方插件和纯 TS 包统一放在 `packages/`。插件通过应用 `app.toml` 的 `[dependencies]` 启用，以 `plugin.toml` 声明目标与平台工程，使用 `src/` 和 `rust/`。Android 使用 Gradle Library 与 Manifest Merger，Apple 使用 SwiftPM 伞包。官方 Cargo crate 使用 `impasto-<name>` 命名，跨包 Rust 依赖使用 `registry = "impasto"`。CLI 同步本地索引和源码路径，校验包依赖闭包，并按拓扑序注册。 [Rust 业务与原生插件](docs/rust-plugins-and-native-modules.md)

## 硬性约束

### 依赖隔离（`just boundaries`）

修改 `Cargo.toml` 或跨 crate 引用时，必须守住以下边界：

- `javascript --no-default-features` 保持纯 `std`，不引入任何外部依赖。
- `javascript --features runtime` 不得依赖 `engine`、`raster`、`embedder*` 或图形、排版依赖，源码中不得出现 UI 模块路径。
- `engine` 默认构建不得依赖 `rquickjs` / `quickjs-jit`、`raster`、`wgpu` 或任何平台/渲染库。
- `raster` 不得依赖 `rquickjs` / `quickjs-jit`。
- `packages/*/rust` 使用 `impasto` SDK、其他插件包及包自身依赖，与 `engine`、`raster`、`embedder*` 和 QuickJS VM 隔离。

### 渲染管线与线程模型铁律

`crates/raster` 与宿主帧调度以 Zed gpui 的极简主干为标杆，仅局部借鉴 Skia Graphite 的紧凑数据索引与按需瞬态纹理思想，严禁叠加重型竞争机制：

- **一类图元只留 1 条主力路径**：宁可放弃微基准里 3% 的理论分数，也不得为同一类图元同时维护多套互斥渲染路径或环境变量开关（组合 bug 不可控）。文字仅保留 Swash 位图字形图集（禁止运行期 CPU MSDF、矢量轮廓回退与 LCD 子像素双源分支）；矩形/阴影/渐变仅保留顺序 Instanced 合批（禁止空间重排器、Early-Z 深度拆批与驻留 RenderBundle）；Canvas 路径收口到单条主力光栅路径；禁止堆叠多套帧间离屏缓存。
- **以真机端到端帧率与输入延迟为准绳**：在移动端 GPU 渲染耗时远低于单帧预算（如 1.4–1.8 ms vs 120 Hz 8.33 ms）的前提下，禁止为了压缩零点几毫秒 GPU 时间而增加 CPU 侧复杂度、引入新缓存层或抽象包装。优化优先级永远是：宿主调度跟手度 > CPU 编码与分配开销 > GPU 耗时。
- **保持 4 阶段一眼看懂的渲染链路**：
  1. **准备（Prepare）**：图片解码上传 + 视口剔除后的并行 Swash 位图字形图集准备；必须设单帧 CPU 时间上限，超限转后台异步完成并保留上一帧画面。
  2. **收集（Collect）**：单次遍历视口剔除，产出零锁、少分配的紧凑扁平图元/绘制列表（重对象进小表、命令以紧凑整数索引引用，减少堆分配与 `Arc` 克隆）。
  3. **编码（Encode）**：无离屏效果的普通帧走零包装直通路径，1 个主 `RenderPass` 直接顺序合批上屏；仅当帧内真实存在毛玻璃、模糊、离屏透明度/混合组时，才按需从瞬态纹理池借用并编码离屏 Pass，不让普通帧为重型图调度买单。
  4. **宿主调度（Host Scheduling）**：稳住 vsync 节奏、输入对齐、帧背压与 Surface 生命周期，禁止跨线程无 token 校验地竞态取消 vsync。
- **TaskRunner 驱动的动态线程模型**：禁止将线程拓扑写死为静态枚举分支。Raster 固定运行在独立 `TaskRunner` 线程；UI/JS 在冷启动阶段于后台 `TaskRunner` 并行完成框架加载与初始脚本执行，启动完成后通过可切换 `TaskRunner` 合并至平台主线程（与 Flutter UI/Platform 线程合并一致），业务层与会话层通过 `TaskRunner` 抽象屏蔽线程迁移细节。

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
| `.impasto/types/`、`.impasto/plugins-manifest.<platform>.json` | `rust/` 和插件的 `#[impasto::module]` 描述 | `impasto sync --platform <platform>`、`impasto analyze --platform <platform>`、`cargo test -p cli` |
| JS 标准库声明与运行时一致面 | `crates/javascript/src/standard/` | `just standard-check` |

`examples/gallery-solid/dist/quotes.js` 不提交 git，但部分 Rust 测试会读取该产物；`just test` 会自动执行 `just gallery-dist`，单独跑 `cargo test` 前需先确保已构建。

## 验证入口

完整构建与测试配方见 [`justfile`](justfile)，CI 门禁见 [`.github/workflows/check.yml`](.github/workflows/check.yml)。修改后按影响范围运行对应检查：

- **常规门禁**：`just check`、`cargo fmt --all -- --check`
- **注意 `just check` 未覆盖的 CI 项**：macOS 下的 `embedder-macos` 检查（`just clippy` 与 Linux CI 保持一致，默认排除 `embedder-macos`）
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
