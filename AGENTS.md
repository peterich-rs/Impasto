# AGENTS.md

## 目标架构

Impasto 是跨平台 UI 框架。下表按目标职责命名。现有目录、接口和行为以源码与测试为准。

| 部分 | 所有权 |
|---|---|
| `driver` | 创建应用工程，编译和打包，生成平台工程，部署应用，以及为热重载监听变更、构建并发送更新产物。 |
| `embedder` | 创建平台窗口和框架执行环境，启动应用并管理生命周期，提供原生绘制表面、vsync、输入、IME、无障碍和平台插件。 |
| `engine` | 接收应用框架提交的 UI 树变更，管理 UI 状态、脏标记和帧内阶段，完成命中测试、手势、测量、布局和绘制录制，产出层树。 |
| `raster` | 接入成熟的绘制库，消费层树并上屏，管理绘制资源和后端表面。文字测量和绘制共用排版实现。 |

JavaScript 运行时负责脚本执行、模块加载、定时器和标准 API，可以独立于 UI 运行。`crates/javascript` 提供独立运行时。`engine::js` 通过可选的 `js` feature 接入该运行时，提交树变更并把 UI 事件交给脚本。以 `fetch` 为例，运行时实现 API 语义，`embedder` 提供网络传输。

平台线程通过队列交付输入和生命周期事件，渲染线程通过 `engine::session::Session` 同步推进 UI 帧。跨线程交付使用自有数据。`raster` 消费 `engine::scene::LayerTree`，并实现 `engine::text::Shaper`。独立运行时、默认引擎和光栅后端的依赖隔离由 `just boundaries` 校验。

## 工作约定

- 文档和回复使用中文。文档写清适用范围和前提，让读者可以独立理解。修改实现时核对相关说明，直接替换过时内容。
- 写 Rust 时使用 `/rust-best-practice`。设计控件或 Gallery 界面时使用 `/unslop`，并用对应宿主验证画面。
- 优先复用许可证合适的发布依赖。产品代码放在 `third_party/` 以外。CI 验证的 Impeller SDK ZIP 归档纳入 `crates/impeller-sys/prebuilt/`，解压后的 SDK 和本地构建产物留在本地。
- 删除实现前核对调用者，修改后运行受影响的检查。构建和检查入口见 [justfile](justfile)。
