# JavaScript

Impasto 的 JavaScript 运行时、IQJS 编译器和标准库集中在 `javascript` crate。解释器由 Cargo 通过 `rquickjs` 构建，版本固定为 quickjs-jit 0.14.2。

| 目录 | 职责 |
|---|---|
| [runtime](src/runtime) | VM 生命周期、脚本执行、模块加载、执行预算、定时器、宿主调用和完成事件 |
| [compiler](src/compiler) | 编译脚本和 ESM，检查静态导入与导出，生成 IQJS 字节码 |
| [standard](src/standard) | 标准 API 的 Rust 实现、profile 和 TypeScript 声明 |
| [host](src/host) | 基于 std 的宿主模块、拥有型值、传输事件和队列额度 |

默认的 `runtime` feature 支持独立运行 JavaScript。关闭默认 feature 后使用只依赖标准库的 `javascript::host`。`runtime::extension` 为可信 Rust 扩展提供受控的 QuickJS 接口。

## 字节码与 UI 接入

`compiler::compile` 对脚本和模块生成 IQJS 封套。CLI 通过 `engine::js::compile_with_registry` 根据应用能力选择模块目录并编译入口。进程内可以使用 `engine::js::open_compiled_with_io` 执行结果。打包文件经受信任入口加载，调用者保证字节来自本工具链。封套验证 magic、引擎版本、模式标签和非空载荷。

UI 扩展在渲染线程管理树和响应式图，使用同一套 `runtime::Runtime`。启动先注册原生绑定、执行场景脚本、安装标准库，然后启动响应式图。关闭时在 VM 存活期间释放图与持久 JS 引用，最后销毁 Context 和 Runtime。场景原生绑定由 `engine::js` 注册。 [架构](../../docs/architecture.md)

## 验证

```sh
cargo run --locked -p javascript --bin js-headless
cargo test --locked -p javascript --no-default-features
python3 tools/check-javascript-boundaries.py
just standard-check
just gallery-dist
cargo test --locked -p engine --features js
```

UI 测试使用真实解释器和 `MonoShaper`，执行编译过的组件夹具及 Gallery 产物。运行时与平台的接入细节见 [架构](../../docs/architecture.md#js-运行时)。
