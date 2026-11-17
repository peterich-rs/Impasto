# @impasto/solid-ui

应用从这里导入组件。`@impasto/solid-renderer` 保留编译、挂载和 `scene`。Text 与 Image 只有一份实现，由本包重导出。

展示用 `style`、`class`、`classList`。`disabled`、`variant`、`controller` 是领域属性。复合组件的内部目标使用 `labelProps`、`contentProps` 这类完整样式对象。`ref` 指向样式所在的节点，卸载时收到 `null`。浮层的 `ref` 指向可见面板，退出动画结束后清空。

```tsx
import { Button, Column, Image, Row, ScrollArea, Text } from "@impasto/solid-ui";

<Column style={{ width: "100%", height: "100%", minHeight: 0 }}>
  <Row style={{ gap: 12, alignItems: "center" }}>
    <Text style={{ fontSize: 20 }}>自选</Text>
    <Button label="添加" variant="primary" onClick={add} labelProps={{ style: { fontWeight: 600 } }} />
  </Row>
  <ScrollArea style={{ flex: 1 }} contentProps={{ style: { padding: 16 } }} />
  <Image source="asset:avatar" intrinsicSize={{ width: 128, height: 128 }} style={{ width: 32, height: 32 }} />
</Column>
```

`pnpm --filter @impasto/solid-ui typecheck` 检查组件源码及 `type-tests/` 中的正反例。`pnpm -r test` 包含这两项类型检查。默认主题与 locale 方向同步到原生环境，Row 继承方向，局部 style 可以显式覆盖。

测试重置在 `@impasto/solid-ui/testing`，不进入应用 bundle。
