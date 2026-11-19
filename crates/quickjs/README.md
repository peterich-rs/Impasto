# QuickJS 会话

包名是 `quickjs`。根 `Cargo.toml` 以依赖名 `rquickjs` 钉住 quickjs-jit 0.14.2，Cargo 构建唯一解释器。`linked/rt::Engine` 拥有 `Runtime`、`Context` 与 `Host`，并持有 `uicore::Tree` 和 `reactive::Graph` 的会话状态。所有调用与销毁都在渲染线程。宿主通过 `session::Session` 泵帧，`JsSession::take_script_error` 取出异常文本。

`src/linked/rt` 注册 `scene`、`reactive`、`#reactive` 与 `__host`。`js::Runtime` 安装 console、performance 和定时器。启动执行 `abi.js` 和 `scene.js` 后，由 `js::standard::install_web` 注册 `legacy` profile 的 Web API，再执行 ESM `reactive-boot.js`。捕获并删除 `__reactive` 后执行应用并排空 jobs。

节点是数字 `NodeId::to_raw()`，`0` 表示没有节点。场景属性在 `src/linked/scene.rs` 映射到 `abi.yml` 字段号，树更新继续通过 `src/linked/mutation.rs`。`src/scene.js` 提供查询、命令和插入范围归一化。布局和样式应用仍由 `uicore` 负责。Rust 绑定先完整解码 JS 样式为拥有型 `uicore::style::codec::Value`，再校验与提交；getter 抛错不提交部分来源。`build.rs` 只生成 `abi.rs`、`abi.js` 和 `packages/solid-renderer/src/mutation.gen.ts`。同步 `rect` 等查询冲刷 mutation、样式和所需布局。

响应式值与回调使用 `Persistent`。`ListDriver` 和 `ScrollController` 是 rquickjs class，持有 index 与 generation，析构调用现有驱动释放函数。`Engine::shutdown` 在 VM 存活时先 `reactive::release_all`，再清空全部定时器、监听器、HTTP 与其他 Persistent，移除 userdata，最后释放 Context 和 Runtime。失败启动与正常 Drop 共用此路径。

内存上限为 16 MiB，栈上限为 8 MiB。`scene.debugMemory()` 查询时采样 VM 内存。中断使用 `CLOCK_MONOTONIC`，eval 预算 1000 ms，其余 turn 128 ms；错误保留入口类型。job drain 最多 32 轮，定时器每次最多触发 64 个。模块文件先按应用根做词法归一化，`..` 留在根内则接受，越出根才拒绝；`AppLoader` 再检查符号链接解析后的真实路径。单个文件不超过 16 MiB。`@impasto/<单段包名>` 从宿主注册表加载。

`compile` 对脚本和模块均生成 IQJS envelope，并返回只有它能构造的 `CompiledBytecode`。`bytecode.rs` 检查 magic、引擎版本、模式标签和非空载荷。这些检查不能证明载荷是对应类型的字节码。进程内用 `open_compiled_with_io`。磁盘文件用 `unsafe` 的 `open_trusted_bytecode_with_io`，调用者保证字节来自本工具链的 `compile`。脚本走 `JS_ReadObject` / `JS_EvalFunction`，模块走 `Module::load`。`open_program_with_io` 只接受源码。`IMPASTO_TRACE` 或 `Session::set_trace` 开启帧计数。

`createStyleSource`、`setStyleSource`、`removeStyleSource` 使用 Tree 分配的来源句柄。`runtime/style.ts` 的 bindStyle 隔离组件默认、class 与 inline 的 owner。原生样式 fixture 在 `tests/fixtures/style.tsx`。

测试始终运行真实解释器。Gallery 产物 `apps/gallery-solid/dist/quotes.js` 由 `just gallery-dist` 本地构建，不进 git。测试使用 `MonoShaper`，验证首屏、市场页、实际组件、控制流、字节码和销毁行为。
