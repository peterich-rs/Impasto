# @impasto/solid-renderer

`runtime/index.ts` 是 Babel 的编译入口，再导出 QuickJS `scene` 模块的原生渲染原语。`src/tree.ts` 中的 `Node` 是 `engine::tree::NodeId::to_raw()` 对应的数字句柄。`Text` 和 `Image` 是组件。测量、布局、命中和录制都在 `engine`。

`tools/pack.mjs` 将应用的 `solid-js` 导入指到 `@impasto/reactive`，运行时使用 `engine::reactive` 中的图。bundle 不包含 `@solidjs/universal` 或 Solid 的调度器。Node 测试替身在 `testing/`，实际原生行为由 `engine/js` 的测试验证。

`just solid` 在仓库根安装依赖并运行全部 JS 包测试，`just solid-app` 运行 gallery 的构建与测试。标签和属性见 `docs/architecture.md`。

作者标签为 `view`、`stack`、`scroll`、`image`。布局、颜色、字体与效果使用 `style`；工具类通过 `class` 和 `classList` 写入同一声明体系。普通 View 默认纵向、min 0、shrink 0，不带父级 100% 上限；填充需要显式声明。

直接调用原生场景时用 `scene.createElement("view", { style })` 或相应标签构造节点，用 `scene.patchStyle(node, style)` 更新样式。底层文字节点使用 `scene.createTextNode`。从旧构造函数和逐字段 setter 迁移的脚本需重新打包。

```tsx
<view class="flex-row gap-2" style={{ padding: 16 }}>
  <Text style={{ fontSize: 18, color: "#112233" }}>内容</Text>
</view>
```

作者组件从 `@impasto/solid-ui` 导入。本包的小写标签留给控件实现。

`style` 接受对象和片段数组。对象中的 undefined 被忽略，null 将字段恢复到节点初始值。`scene.setStyle` 替换 imperative 来源，`patchStyle` 合并，`clearStyle` 撤销；class、inline 和其他 owner 的来源保留。`bindStyle(node, { defaults, class, classList, style })` 响应 getter 内的信号，并在 owner 清理时移除自己的来源。组件默认值、class、inline、imperative、animation 依次覆盖。
