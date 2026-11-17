# QuickJS 会话

包名是 `quickjs`。它拥有一个 quickjs-ng 运行时和一棵 `uicore::Tree`。宿主只通过 `session::Session` 看见它。`JsSession::take_script_error` 取出监听器或 `onFrame` 留下的异常文本。

`src/glue.c` 注册具名模块 `scene`、`reactive` 和能力模块。场景属性名在 `src/linked/scene.rs` 映射到 `abi.yml` 的字段号，原生渲染原语直接调用 `uicore::Tree`。`src/scene.js` 包装查询与命令。节点是数字 `NodeId::to_raw()`，`0` 表示没有节点；没有全局 `core`。响应式 arena 在 `crates/reactive`，QuickJS 的引用计数适配在 `src/linked/reactive.rs`。帧监听前和布局监听后由会话冲刷非空队列，滚动弹簧仍由 `Tree` 采样。

`compile` 生成字节码；`open_program_with_io` 加载 ESM 或同一工具链生成的可信字节码。`IMPASTO_TRACE` 或 `Session::set_trace` 开启帧计数。实际的 Solid 对照、控制流和字节码往返测试在 `src/lib.rs`。
版本钉在 `quickjs.pin`，现在是 quickjs-ng v0.17.0。`just quickjs` 把源码解到 `third_party/quickjs`，不进 git。没有这份源码时 `open` 返回 `NotLinked`，crate 仍然能编译。

Gallery 产物是 `apps/gallery-solid/dist/quotes.js`，由 `just gallery-dist` 从 TSX 本地构建，不进 git。链了 quickjs 源码时，gallery 测试要求这份产物存在，缺失即失败并提示构建命令。无窗口测试用 `MonoShaper`：首屏出现「自选」和价格，点「市场」后出现涨幅榜。

样式使用 `abi.yml` 生成的 `StyleValueRecord` 传输，不序列化成换行文本。C 端先遍历并复制完整 JS 值，Rust 再转换成拥有型 `style::codec::Value`，最后借用 Tree 校验与提交。getter 抛错不会提交部分来源。支持片段数组、undefined、网格轨道及绘制效果对象。同步 `rect` 等查询冲刷待处理 mutation、样式和布局。

`createStyleSource`、`setStyleSource`、`removeStyleSource` 使用 Tree 分配的来源句柄。`runtime/style.ts` 的 bindStyle 用它们隔离组件默认、class 与 inline 的 owner。原生回归入口是 `tests/fixtures/style.tsx`。
