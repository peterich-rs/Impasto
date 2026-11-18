# @impasto/reactive

应用 TSX 继续从 `solid-js` 导入。打包时 `tools/pack.mjs` 将入口解析到本包，`tools/analyze.mjs` 用 [api.d.ts](src/api.d.ts) 检查应用可用的 API。信号、依赖和更新顺序由渲染线程的 `reactive::Graph` 持有，会话在帧内冲刷。应用通常只写信号，需要同步冲刷时可调用 `batch`。

| 当前可用 | API |
|---|---|
| 信号和计算 | `createSignal`、同步 `createMemo`、`untrack`、`batch` |
| Effect | `createRenderEffect`、`createTrackedEffect` |
| Owner 和清理 | `createRoot`、`getOwner`、`runWithOwner`、`onCleanup` |
| Context 与子节点 | `createContext`、`useContext`、`children` |
| 控制流 | `Show`、`Switch`、`Match`、`For`、`Errored`。错误边界当前接同步异常 |

本包也导出供渲染器合并属性的 `merge`，编译产物通过 `@impasto/solid-renderer` 的 `mergeProps` 调用它。

下一阶段为 memo 接上 Promise 和异步迭代器，并补上 `from`、`isPending`、`latest`、`NotReadyError`、`Loading`、`Reveal` 和 `Errored` 的异步拒绝处理。后续还包括 `action`、`createOptimistic`、`createStore`、`reconcile`、`snapshot` 和 `createProjection`。`createEffect`、`onFlush`、`lazy` 和 `Repeat` 目前也不可用。图的诊断码计划通过 `DiagnosticsHub` 接入 devtools。

框架不提供 `@solidjs/web`、SSR、hydration、`seroval`、`ssrSource`、`enableHydration`、`@solidjs/universal`、`@solidjs/router` 或 `@solidjs/diagnostics`（含浏览器桥）。路由使用 `@impasto/solid-router`，编译使用 Babel 的 universal 输出。

`just solid` 中通过 `tools/resolve-hook.mjs` 导入 `solid-js` 的测试运行官方 2.0.0。`crates/reactive/tests/solid_2_0.rs` 验证 Rust 图，并用 Node 重跑官方包对照。`crates/quickjs` 的测试让原生模块运行同一段脚本。官方包和原生模块都与 [order.txt](../../crates/reactive/tests/fixtures/order.txt) 比对。
