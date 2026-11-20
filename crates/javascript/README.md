# JavaScript

Impasto 的 JavaScript 运行时、IQJS 编译器、标准库和 UI 绑定集中在 `javascript` crate。解释器由 Cargo 通过 `rquickjs` 构建，版本固定为 quickjs-jit 0.14.2。

| 目录 | 职责 |
|---|---|
| [runtime](src/runtime) | VM 生命周期、脚本执行、模块加载、执行预算、定时器、宿主调用和完成事件 |
| [compiler](src/compiler) | 编译脚本和 ESM，检查静态导入与导出，生成 IQJS 字节码 |
| [standard](src/standard) | 标准 API 的 Rust 实现、profile 和 TypeScript 声明 |
| [host](src/host) | 基于 std 的宿主模块、拥有型值、传输事件和队列额度 |
| [ui](src/ui) | 场景和响应式绑定、Canvas、输入及帧推进 |
| [session](src/session) | UI 会话与平台共用的帧、事件和配置类型，由 workspace 的 `session` crate 重导出 |

默认的 `runtime` feature 支持独立运行 JavaScript。平台宿主和 CLI 启用 `ui`，通过 `javascript::ui` 打开应用或编译 UI 入口。单独依赖会话契约时使用 `session` feature。关闭默认 features 后可以只使用 `javascript::host`。

## 字节码与 UI 生命周期

`compiler::compile` 对脚本和模块生成 IQJS 封套。CLI 按应用能力选择模块目录，通过 `ui::compile_with_registry` 编译应用。进程内可用 `ui::open_compiled_with_io` 执行编译结果。加载打包文件使用受信任字节码入口，调用者保证字节来自本工具链。封套验证 magic、引擎版本、模式标签和非空载荷。

`ui/player` 在渲染线程拥有树和响应式图，使用共享的 `runtime::Runtime`。启动先注册原生绑定，再执行生成的 ABI 与场景脚本，随后安装标准库并启动响应式图。关闭时在 VM 存活期间释放图与持久 JS 引用，最后销毁 Context 和 Runtime。场景 ABI 编号集中在 [abi.yml](src/ui/abi.yml)，构建脚本生成 Rust、JavaScript 和 TypeScript 常量。

## 验证

```sh
cargo run --locked -p javascript --bin js-headless
cargo test --locked -p javascript --no-default-features
python3 tools/check-javascript-boundaries.py
just standard-check
just gallery-dist
cargo test --locked -p javascript --features ui
```

UI 测试使用真实解释器和 `MonoShaper`，执行编译过的组件夹具及 Gallery 产物。运行时与平台的接入细节见 [架构](../../docs/architecture.md#js-运行时)。
