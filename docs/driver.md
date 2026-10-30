# 组合控件驱动

`VirtualList`、`ScrollArea`、`RefreshControl` 的 props 不变。窗口、测量、夹紧和刷新阈值在 `uicore`。语言侧是模板、信号和渲染回调。QuickJS 用 class 暴露这两台状态机。

## 破坏性变更

应用代码不改 props。内部从「JS 里读 `rect`、算窗口、写 signal」改成 driver 加皮。

`createSlot` 仍是 Solid 的 owner（`createRoot` / `dispose`）。调用时机由 `ListDriver` 的 `Adopt` / `Detach` / `Set` / `Place` 决定。

`glue.c` 增加 `ListDriver` 和 `ScrollController` 两个 class，以及一个 `onDriver` 回调。C 只在这个文件里。

## 可行性

原先仓库里没有 `list.rs`、也没有 `JS_NewClass`。已有的是 `qjs_scroll_state` 那种单槽加 id 的分发，以及 `Tree` 上的 `rect`、`scroll_offset`、`set_scroll_offset`、`fling_target`、`set_size`、`set_min_size`、`set_transform`。driver 用这些，不新增布局能力。读滚动相位加了 `Tree::scroll_phase`，给刷新状态机用。

`linked/` 的帧顺序本来就是布局、布局回调、必要时再布局、录制。tick 插在布局之后、`onLayout` 之前，在 Rust 里跑完再通知脚本。帧语义不变。

条目不在 tick 里回调 `count` / `keyAt` / `typeAt`。那会把每帧的 ABI 带回来。语言侧在 signal 变化时推一次 key 表。

## 设计

```text
皮（各语言各写）
  Solid: VirtualList / ScrollArea / RefreshControl = JSX + signal + 渲染回调
  Kotlin（未做）: 同一对 class，DSL 换掉 JSX
响应式绑定
  bindListRenderer: ops → createSlot
  ScrollController 包装: phase / overscroll 信号，命令转到 class
native 边界（glue.c）
  事件上行: onDriver(kind, index, generation, tag, …)，不持 JSValue
  命令下行: class 方法
Rust driver（uicore，一份）
  ListDriver: 窗口滞回、rect 测量、Fenwick、滑块、吸顶、可见集 diff
  ScrollController: 夹紧、拖动续算、越界交给父控制器、两种刷新阈值。声明了单轴时丢掉跨轴增量，不交给父级
uicore 底座
  Tree、ScrollPhysics、手势
```

今天测量在 JS `onLayout` 里对每个可见行 `host.rect()`。目标是 tick 在 Rust 里读 `Tree::rect`，可见集不变时不进脚本。变了才发 ops，脚本只做 adopt / detach / set / place。

1. driver 是状态机加 tick，不是组件。它交出渲染指令和查询，不长 JSX、样式或应用状态。
2. tick 在帧流程里，不在 JS 回调里。未测完的行高由 driver 直接请求下一帧。
3. `setItems` 推入 `{ key, type, sticky }`。条目本体留在语言侧。ops 带 index，皮用 index 取条目。
4. 行节点 `noteRow(key, node)` 登记一次。之后测量不再过 ABI。
5. 刷新有两种，都在 `ScrollController` 上，一个 controller 同时只用一种。`Overscroll` 是 `RefreshControl`：拖动中 `max(0, -overscroll.y)` 越过阈值则武装，相位回到 idle 才提交。`Edge` 是 `VirtualList` 的 `onRefresh`：累加顶部被夹掉的距离，松手时提交。
6. `createSlot` 保持语言侧。`bindListRenderer` 把 ops 变成 Slot 调用。池的上限在皮上。
7. Rust 不保存 `JSValue`。class 的 opaque 是序号和代。终结器调用 release。代不对的调用失败。
8. 自定义 `onDragOverflow` 只在 controller 没有父控制器接住余量时才进脚本。`consumeScrollOverflow` 调 controller 的 `consume`。
9. `createScrollLink` 的映射是任意 JS 函数，留在皮上。它不在列表稳态路径上。
10. 独立 `Recycler` 仍对调用方已经切好的 `each` 做语言侧池化。虚拟列表的窗口 diff 只在 `ListDriver`。

```text
core.listDriver(overscan, estimate) → ListDriver
  bind(viewport, content, top, bottom, rows, vThumb, hThumb, pin, vSize, hSize)
  configure(estimate, overscan, blocked)
  setItems([{ key, type, sticky }])
  noteRow(key, node)
  release()

core.scrollController() → ScrollController
  bind(viewport, content)
  configure(contentMin, pullMode, threshold, rubber, factor, parent)
  dragBy(dx, dy) → { ox, oy, overX, overY, pending }
  finishDrag(taken)
  dragEnd()
  wheel(dx, dy)
  query(0 | 1 | 2)   // offset, limits, overscroll
  consume(ox, oy)
```

`onDriver` 的 tag：`1` 是列表 op（`i0` 为 detach / adopt / set / place），`2` 是 chrome（滑块可见性和长度、吸顶下标、内容总高），`3` 是接近末尾，`5` 是刷新提交。滑块位移和吸顶平移直接写节点，不进这条回调。

## 分阶段

这些都已经在树里。

1. `uicore/src/list/`：Fenwick、窗口、chrome、diff、`ListDriver`、`ScrollController`。`cargo test -p uicore --lib list::`。数学与原先 `list-window.ts` 的样例同输入同输出。
2. `linked/` 在布局后 tick，再 `onLayout`。`glue.c` 注册两个 class。`cargo test -p quickjs --lib list_driver_methods_are_functions`。
3. `packages/solid-renderer/runtime/list-binding.ts` 把 ops 交给 Slot。`VirtualList.tsx` 只留模板和渲染回调。`list-window.ts` 已删。`node … list-binding.test.ts`。
4. `ScrollArea` / `RefreshControl` 把夹紧和刷新读 controller。gallery 的 `/list` 滚动和下拉刷新由 `cargo test -p quickjs --lib` 里那三条列表测试覆盖。
5. 本文、`docs/architecture.md` 和 `AGENTS.md` 的分层表与帧顺序按上面的事实写。

## 不做什么

Rust 里没有组件、没有样式、没有条目数据。不生成通用绑定。三个以上的 class 仍是手写 glue；再多就退回句柄加函数表，不把 setter 逐个做成产品头文件。

`Field`、overlay、键盘和路由留在语言侧。它们要么绑输入法，要么绑 Solid 的 owner，不是这台列表状态机。

## 文件

- `uicore/src/list/` — `ListDriver`、`ScrollController`、Fenwick、窗口、diff
- `uicore/src/tree/scroll.rs` — `Tree::scroll_phase`
- `crates/quickjs/src/linked/`、`linked/driver.rs` — 帧内 tick 和 class 方法
- `crates/quickjs/src/glue.c` — 两个 class 和 `onDriver`
- `packages/solid-renderer/runtime/driver.ts` — 单槽分发
- `packages/solid-renderer/runtime/list-binding.ts` — ops 到 Slot
- `packages/solid-ui/src/VirtualList.tsx`、`ScrollArea.tsx`、`RefreshControl.tsx`、`ScrollController.ts` — 皮
- `packages/solid-ui/src/list-window.ts` — 已删除
