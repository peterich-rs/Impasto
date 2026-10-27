# @uifw/solid-renderer

`src/tree.ts` 的 `hostRendererOptions()` 把 `@solidjs/universal` 的 `RendererOptions` 接到 `core`，不导入 Solid。宿主标签和属性见 `docs/architecture.md` 的「渲染器」。节点是 `{ id }`。

`runtime/index.ts` 是编译入口。`Text` 是组件。

`just solid` 跑 `src/tree.test.ts` 和 `runtime/frame.test.ts`。编译计数器是 `just solid-app`。
