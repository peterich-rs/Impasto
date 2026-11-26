# 方向一设计方案：原语模板扩展、Keyed 列表原生调和与标量样式快通道

本文档是 [`docs/native-toolchain-v2-roadmap.md`](native-toolchain-v2-roadmap.md) **方向一**的完整工程落地设计方案，目标是在不破坏 Solid 细粒度响应式语义与现有组件 API 的前提下，消除组件库内部壳层与动态列表调和中的残余运行时开销。

适用模块：[`crates/bundler/src/jsx.rs`](../crates/bundler/src/jsx.rs)、[`crates/bundler/src/pools.rs`](../crates/bundler/src/pools.rs)、[`crates/engine/src/scene/template.rs`](../crates/engine/src/scene/template.rs)、[`crates/engine/src/js/player/`](../crates/engine/src/js/player/mod.rs)、[`packages/reactive/src/flow.ts`](../packages/reactive/src/flow.ts)、[`packages/solid-renderer/runtime/`](../packages/solid-renderer/runtime/index.ts) 与 [`packages/solid-ui/src/`](../packages/solid-ui/src/index.tsx)。

---

## 1. 问题定位与量化根因

在完成 `@impasto/solid-ui` 基础盒模型（`View`/`Row`/`Column`/`Stack`/`Grid`/`Text`）模板化后，剖析当前 Gallery 与 `solid-ui` 编译产物，仍存在四处高频热点：

1. **`Image` 仍游离在模板与静态池之外**：[`packages/solid-renderer/runtime/image.ts`](../packages/solid-renderer/runtime/image.ts) 对每个 `<Image>` 执行 `bindStyleTo`（创建 3 个原生 `StyleSource` + 注册 `onCleanup` + 1 个样式 `RenderEffect`）及 1 个图片源/无障碍 `RenderEffect`。图文卡片中只要含有一个图标，该节点就无法合入父级 `Template`。
2. **`ref` 属性一票否决整棵子树模板化**：[`crates/bundler/src/jsx.rs`](../crates/bundler/src/jsx.rs) 的 `template_host` 在遇到 `ref` 时直接返回 `None`。由于 `Button`、`ScrollArea`、`Field`、`Sheet`、`NavigationBar`、`VirtualList` 内部均通过 `ref` 获取宿主句柄做初始化，这些控件的内部骨架全部退化为逐节点 `createElement` + `insertNode`。
3. **`Pressable` / `Button` 内部的 `{...eventsOf(props)}` 触发全量 `spread()`**：[`packages/solid-ui/src/Pressable.tsx`](../packages/solid-ui/src/Pressable.tsx) 与 [`Button.tsx`](../packages/solid-ui/src/Button.tsx) 向 `<HostView>` 展开了包含 21 个字段的 `eventsOf(props)` 对象，触发 [`runtime/index.ts`](../packages/solid-renderer/runtime/index.ts) 中的 `spread()`：分配 `RenderEffect`、执行 `Object.assign` 并用 `new Set` 遍历合并全部 21 个键。
4. **`<For>` 双重 $O(N^2)$ 调和与动态单属性字符串开销**：
   - [`packages/reactive/src/flow.ts`](../packages/reactive/src/flow.ts) 中 `For` 通过 `remaining.findIndex(...)` + `remaining.splice(at, 1)` 做 $O(N^2)$ 匹配，产出节点数组后再进入 [`crates/engine/src/js/scene.js`](../crates/engine/src/js/scene.js) 的 `_insertRange`，后者又用 `nodes.indexOf(node)` 做第二次 $O(N^2)$ 查表与逆序逐个 `insertNode` FFI 调用。
   - [`crates/engine/src/js/player/vm/renderer.rs`](../crates/engine/src/js/player/vm/renderer.rs) 的 `patchInlineStyle` 每次更新单属性（如动画/手势驱动的 `opacity`、`translateX`、`backgroundColor`）都要跨 FFI 传递属性名字符串并重新查询 `codec::declarations`。

---

## 2. 子方案 A：`Image` 原语化与带 `ref` 节点入模板

### 2.1 扩展 `TemplateKind` 与 `TemplateRow` 二进制契约

修改 [`crates/engine/src/scene/template.rs`](../crates/engine/src/scene/template.rs)，在保持 `postcard` 编码紧凑性的前提下纳入 `Image` 与静态图片资产引用：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemplateKind {
    View,
    Stack,
    Scroll,
    Text,
    Row,
    Column,
    Grid,
    RepaintBoundary,
    Image,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateImage {
    pub uri: String,
    /// 以毫点（1/1000 pt）或位模式编码的固有宽高，None 表示未指定。
    pub intrinsic_size_bits: Option<(u32, u32)>,
    pub required: bool,
    pub alt: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateRow {
    pub kind: TemplateKind,
    pub parent: u8,
    pub text: Option<String>,
    pub style: Option<u8>,
    pub class: Option<u8>,
    pub image: Option<u8>,
}
```

**校验不变式（`template::validate`）**：
- `(row.kind == TemplateKind::Text) == row.text.is_some()` 保持不变；
- `row.image.is_some()` 仅当 `row.kind == TemplateKind::Image` 时合法（动态 `source` 的 `Image` 节点允许 `row.image == None`，在 `bindings` 阶段动态绑定）；
- `pools::PoolModules::template` 将静态 `TemplateImage` 随模板描述符一并编码进 `postcard` 载荷（ bump `static_pool` 版本前缀为 `"2:"` 以防旧缓存混用）。

### 2.2 编译器 `Primitive::Image` 降级规则

在 [`crates/bundler/src/jsx.rs`](../crates/bundler/src/jsx.rs) 中：
1. `Primitive::imported` 识别 `("@impasto/solid-ui" | "@impasto/solid-renderer" | "@impasto/solid-renderer/host", "Image") => Primitive::Image`。
2. 允许的属性集：`style`、`class`、`classList`、`ref`、`source`、`intrinsicSize`、`alt`、`required`（且无子节点）。
3. **静态与动态分流**：
   - 当 `source` 为编译期已知字符串字面量或由 [`assets.rs`](../crates/bundler/src/assets.rs) 生成的静态图片导入（通过模块导出常量识别 `{ uri, width, height }`），且 `intrinsicSize`、`alt`、`required` 均为静态字面量时：直接填入 `TemplateRow.image`，`mount_template` 在 Rust 侧一步完成 `Create::Image`、固有尺寸设置与 `Role::Image` 无障碍标签写入，**零 JS Effect**。
   - 当 `source` 或 `alt` 动态时：节点本身仍进入 `TemplateRow`（享受静态 `style`/`class` 池与批量建树），仅在 `template.bindings` 中追加一条窄绑定 `__impasto_jsx.bindImageSource(nodes[i], () => source, () => intrinsicSize, () => alt)`。

### 2.3 安全放行带 `ref` 的节点进入 `Template`

**语义不变式核对**：Solid 中 `<View ref={fn}><Child /></View>` 的契约是——当 `fn(node)` 执行时，`node` 的直接静态属性、静态样式及模板内子节点均已挂载到 `node` 下。

在 `jsx.rs` 的 `template_node` 中：
1. 移除 `template_host` 对 `"ref"` 属性的拒绝逻辑（仍拒绝 `SpreadAttribute` 与重复同名属性）。
2. 在 `Template` 结构中新增 `refs: String` 缓冲区（独立于 `bindings`）：
   - 属性绑定（`style_patches`、事件 `setProp`、无障碍 `semantics`）写入 `template.bindings`；
   - 子节点动态区间（`insert`）写入 `ranges` 并紧跟在 `template.bindings` 之后；
   - **所有节点的 `ref` 回调按子树后序（Post-order：先子后父）写入 `template.refs`，排在 `ranges` 之后执行**。
3. 最终生成的代码顺序严格保证：
   ```js
   __impasto_jsx.bindTemplate(tmpl, nodes => {
     // 1. 静态属性与响应式属性 Effect 挂载
     // 2. 动态子表达式区间 insert 挂载
     // 3. 子节点 ref -> 父节点 ref 依次 untrack 调用（此时整棵子树 DOM 结构已完整）
   })
   ```

---

## 3. 子方案 B：消除 `Pressable` / `Button` 内部的运行时 `spread()`

在 [`packages/solid-ui/src/element.tsx`](../packages/solid-ui/src/element.tsx) 中新增无展开开销的事件直绑辅助函数 `bindViewEvents(node: NodeHandle, props: ViewEvents, overrides?: Partial<ViewEvents>)`：

1. 将原本 `<HostView {...eventsOf(props)} onPress={press.onPress} ...>` 改为显式干净的 `<HostView ref={...}>`（不带任何 JSX `{...spread}`！）。
2. 在 `bindViewEvents` 中：
   - 对于由 `pressable.ts` 接管的固定闭包（`onPress`、`onRelease`、`onClick`、`onFocus`、`onBlur`、`onKey`、`onText`），在创建期直接一次性调用对应的 `scene.onPress(node, press.onPress)` 等原生方法（零 `setProp` 字符串分发、零 `RenderEffect`）；
   - 对于透传自作者 `props` 的其余低频手势属性（`onDrag*`、`onHover`、`onLeave`、`onWheel`、`onLongPress`、`onZoom`、`onSecondaryClick`、`recognizers`），仅建立单条轻量级监听或按需挂载，彻底消灭每次挂载 `Button`/`Pressable` 时构造 21 键对象与 `Set` 差集计算的开销。
3. 叠加 2.3 节的“带 `ref` 节点入模板”后，`Button.tsx` 的 `<HostView ref={...}>{props.icon}<Text ...>{props.label}</Text></HostView>` 将直接合成为一个双节点 `Template`！

---

## 4. 子方案 C：`<For>` 编译期识别与原生 $O(N)$ Keyed List 调和

### 4.1 现有瓶颈拆解

当前 `<For each={items()} keyed={x => x.id}>{item => <Row .../>}</For>` 经历两层 JS 数组操作：
1. `flow.ts` 的 `For` 在每次 `items()` 更新时复制 `remaining = rows.slice()`，对每个新元素调 `remaining.findIndex(...)`（$O(N^2)$）与 `remaining.splice(...)`（$O(N^2)$ 数组搬移），最后 `rows.map(r => r.value)` 生成新数组；
2. 外层宿主父节点的 `_insertRange` 收到新节点句柄数组后，遍历旧数组调 `nodes.indexOf(node)`（又是 $O(N^2)$）删除消失节点，再从后往前逐个查询 `scene.parent(node)` / `scene.nextSibling(node)`（$2N$ 次跨 FFI 读取）并逐个调 `_insertNode` FFI。

### 4.2 设计：编译器直连单层 Keyed Range 调和器

1. **JS 侧 $O(N)$ Map 索引与懒数组消除（`packages/solid-renderer/runtime/list.ts`）**：
   - 为宿主节点下直接挂载的 `<For>` 提供专用运行时原语 `insertKeyedList(parent, each, keyFn, renderRow, fallback, marker)`；
   - 内部使用 `Map<unknown, RowRecord>`（当同一 key 重复出现时退化挂载溢出链表以保持行为兼容）替代 `findIndex` + `splice`，查找与复用复杂度严格降至 **$O(N)$**；
   - 每个 `RowRecord` 直接记录其产出的原生 `NodeHandle`（单个句柄或小型片段句柄切片）。当列表顺序未变（最常见的尾部追加、头部插入或局部状态刷新）时，完全跳过未移动项的树查询。
2. **Engine 原生批量兄弟重排指令 `scene.reconcileChildren`**：
   - 在 [`crates/engine/src/js/player/vm/renderer.rs`](../crates/engine/src/js/player/vm/renderer.rs) 与 [`mutation.rs`](../crates/engine/src/js/player/mutation.rs) 新增单次批量调和接口：
     ```ts
     scene.reconcileRange(
       parent: NodeHandle,
       previous: readonly NodeHandle[],
       next: readonly NodeHandle[],
       beforeMarker: NodeHandle,
     ): void;
     ```
   - **Rust 侧执行算法（[`crates/engine/src/tree/`](../crates/engine/src/tree/mod.rs)）**：
     - 1 次 FFI 进入 Rust `Tree` 借用；
     - 用栈上/复用缓冲的 `FxHashSet`（或密集位集）标记 `next` 集合，线性扫描 `previous` 一次性 `remove` 被淘汰节点；
     - 对 `next` 计算最长递增子序列（LIS，或双端同步收敛 fast-path 处理 95% 的头尾增删与单元素移动），**仅对真正改变相对次序的节点执行链表摘挂**，且全程在单个 `Tree` 借用内完成，FFI 次数从 $O(3N)$ 降至 **$O(1)$（固定 1 次）**！
3. **编译器自动桥接（`crates/bundler/src/jsx.rs`）**：
   - 在 `jsx.rs` 追踪从 `solid-js` / `@impasto/reactive` 导入的 `For` 符号；
   - 当 `<For>` 作为宿主元素（`Primitive`）的直接子节点出现时，直接编译为 `{rt}.insertKeyedList(parent, () => each, keyFn, renderFn, fallbackFn, marker)`，绕开通用 `createComponent(For, ...)` + `memo` + `_insertRange` 三层包装；当 `<For>` 出现在非宿主位置（如作为自定义组件的 `children` prop 传递）时，平滑回退至优化为 `Map` 索引的 `flow.ts` `For` 实现。

---

## 5. 子方案 D：单属性标量与颜色样式整数 Opcode 快通道

### 5.1 协议设计

在混合样式拆分（`jsx.rs` `StylePatchSyntax::Property`）中，80% 以上的动态单属性集中在数值标量与颜色两类属性。为此在 `engine::style` 暴露稳定的单字节编译期通道码 `ScalarStyleOp`：

| Opcode (`u8`) | 对应样式属性 | 参数契约（JS → Rust 直接 Coerce，零 `codec::Value` 构造） |
|---:|---|---|
| `0` | `opacity` | `f32`（非有限值抛 `StyleError`） |
| `1` | `translateX` | `f32`（points） |
| `2` | `translateY` | `f32`（points） |
| `3` | `scale` | `f32`（同时写 `ScaleX` 与 `ScaleY`） |
| `4` | `scaleX` | `f32` |
| `5` | `scaleY` | `f32` |
| `6` | `rotate` | `f32`（radians/deg 按现有 `codec` 单位一致） |
| `7` | `width` (number) | 当运行时 `typeof v === "number"` 走 `SizeValue::points(v)` 快路径，否则回退 `patchInlineStyle` |
| `8` | `height` (number) | 同上 |
| `9` | `borderRadius` (number) | `Length::points(v)` 快路径 |
| `10` | `zIndex` | `i32` |
| `11` | `backgroundColor` | 支持打包 `u32` RGBA、`[r,g,b,a]` 或字符串快速解析 |
| `12` | `color` | 同上 |
| `13` | `borderColor` | 同上 |

### 5.2 引擎与编译器协同实现

1. **Engine 侧（`Mutation::ScalarInlinePatch`）**：
   - 在 `renderer.rs` 新增 `patchScalarStyle(node: u64, op: u8, value: Value)`：当 `op` 为 `0..=6` 且 `value.as_number()` 命中时，**零字符串分配、零 `Vec<Key>` 堆分配、零 `codec` 哈希表查找**，直接在栈上构造对应的 `Property` 写入节点的 `Inline` 样式层并精确标记脏位（例如 `opacity`/`translateX` 仅脏 Paint/Compositor，不触发 Taffy Layout！）。
   - 当传入 `null` / `undefined`（用于重置该内联属性）或复杂对象单位（如 `{ rem: 2 }`、`"50%"`）时，在 `patchScalarStyle` 内部平滑回退至该 `op` 对应的 `Key` 重置或 `codec::declarations` 解析，保证 **100% 语义等价与类型兼容**。
2. **Bundler 侧（`jsx.rs`）**：
   - `StylePatchSyntax::Property(key, expr)` 检查 `key` 是否映射到 `ScalarStyleOp`；命中则发射 `{rt}.bindScalarStyle(node, OP_CODE, () => expr)`（静态一次性补丁则调 `patchScalarStyle`），生成的 JS 体积更小（数字字面量 `0` 替代字符串 `"opacity"`）。

---

## 6. 实施步骤与验证门禁

| 步骤 | 改动范围 | 验证用例与门禁 |
|---|---|---|
| **Step 1：`ref` 入模板 + `Image` 原语化** | `template.rs`、`pools.rs`、`jsx.rs`、`runtime/index.ts` | 新增编译器快照测试验证后序 `ref` 调用顺序（子 `ref` 先于父 `ref`，且调用时 `firstChild` 已存在）；验证静态与动态 `<Image>` 像素渲染与无障碍角色一致。 |
| **Step 2：`Pressable`/`Button` 去 `spread`** | `element.tsx`、`Pressable.tsx`、`Button.tsx` | 更新 [`packages/solid-ui/src/toolchain.test.tsx`](../packages/solid-ui/src/toolchain.test.tsx)，断言挂载 10 个 `<Button>` 的 `ffiCalls` 与 `styleResolves` 大幅下降，键盘回车/空格激活与 `disabled` 行为不变。 |
| **Step 3：标量样式 Opcode 快通道** | `style/mod.rs`、`mutation.rs`、`renderer.rs`、`jsx.rs` | 覆盖 `number`、`null`/`undefined` 重置、`NaN` 报错、`%` 字符串回退四类边界测试；对比动画更新帧 `script_us`。 |
| **Step 4：原生 `<For>` Keyed List 调和** | `tree/`、`renderer.rs`、`runtime/list.ts`、`jsx.rs` | 构造头部插入、尾部追加、逆序翻转、随机洗牌、重复 Key、空列表 `fallback` 切换 6 组用例，断言 DOM 顺序与焦点保留；200 行重排基准 FFI 调用降为单次批量提交。 |

最终统一运行 `just check`、`just boundaries`、`just host` 与 `just toolchain-baseline` 归档前后对比数据。
