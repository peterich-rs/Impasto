# QuickJS 会话

包名是 `quickjs`。它拥有一个 quickjs-ng 运行时和一棵 `uicore::Tree`。宿主只通过 `session::Session` 看见它。`JsSession::take_script_error` 取出监听器或 `onFrame` 留下的异常文本。

`src/glue.c` 把全局对象 `core` 上的函数交给 Rust。句柄是 `u64`。`0` 只表示没有节点。函数表见 `docs/architecture.md` 的「运行时会话」。

版本钉在 `quickjs.pin`，现在是 quickjs-ng v0.17.0。`just quickjs` 把源码解到 `third_party/quickjs`，不进 git。没有这份源码时 `open` 返回 `NotLinked`，crate 仍然能编译。

Gallery 产物是 `apps/gallery-solid/dist/quotes.js`，由 `just gallery-dist` 从 TSX 本地构建，不进 git。链了 quickjs 源码时，gallery 测试要求这份产物存在，缺失即失败并提示构建命令。无窗口测试用 `MonoShaper`：首屏出现「自选」和价格，点「市场」后出现涨幅榜。
