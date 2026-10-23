# @uifw/solid-renderer

两种用法，同一组 `@solidjs/universal` 的 `RendererOptions`。

`src/scene.ts` 把调用记成版本 1 场景格式，标签是 `layer`、`rect`、`text`、`image`。字节必须和 `protocol/tests/fixtures/counter.bin` 一致。格式见 `docs/native-phase.md`。

`src/tree.ts` 的 `hostRendererOptions()` 把同一组方法接到 `core`，不导入 Solid。宿主标签和属性见 `docs/architecture.md` 的「渲染器」。节点是 `{ id }`。

`runtime/index.ts` 是编译入口。`Text` 是组件。

`just solid` 跑字节对比和 `src/tree.test.ts`。编译计数器是 `just solid-app`。
