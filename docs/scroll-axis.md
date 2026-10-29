# 声明 Scroll 轴与手势认领

本文是已经落地的行为。阶段是这次改动的边界。当前事实以源码和 `docs/architecture.md` 为准；这里保留取舍，避免以后把轴判定放回指针层。

## 相对原稿的修正

1. 轴住在 `ScrollGeom`，枚举在 `core/src/motion.rs`。节点 `Payload` 只有文本、图片和容器，没有 `ScrollPayload`。
2. `set_scroll_physics` 本来就有。`ScrollAccept` 是 `ScrollPhysics` 的新字段。JS 的第 11 个参数可缺省，缺省 `Content`。
3. `scroll_limits` 把跨轴收成 0，区分不了「`Both` 但这一轴没有内容（溢出交给父级）」和「声明了单轴（丢掉）」。`ScrollController` 读 `ScrollAxis`，单轴的跨轴增量不进拖动累积，也不进 overflow。
4. 锁定之后不把丢掉的跨轴速度接力给垂直的父级。竖向 fling 不会推动横向 pager。纯跨轴拖动在锁定前就不认领，父级可以在自己的轴上赢下这次手势。锁定之后 `DragBy` 仍是原始增量，跨轴分量在 `set_scroll_offset` 丢掉。
5. 裸 `scroll()` 缺省仍是 `Both` + `Content`，`lock_at_edge` 只留给它。`ScrollArea` / `VirtualList` 声明了单轴，不再走这个兜底，所以它们的 `accept` 缺省是 `Always`：到边仍认领，回弹和刷新还在。想在边上把拖拽让给父级时显式传 `content`。
6. `Field` 的滚动按行数声明：单行 `horizontal`，多行 `vertical`。`scroll_into_view` 因此只弹该轴，不会和外面的列表抢另一轴。
7. gallery 里原来那行横向 snap 必须传 `horizontal`。`ScrollArea` 改成缺省 vertical 之后，不传就会停住。
8. Kotlin 还没有滚动 DSL。轴、accept 和过滤都在 `core`，以后的绑定只是属性名。

## 1. 破坏性变更说明

- `Tree::scroll()` 与 `host.scroll()` 不传轴时仍是 `Both`。存量裸滚动零改动。
- `host.scroll(axis)` 与 `setScrollAxis` 的编码是 `0` both、`1` vertical、`2` horizontal。未知编码按 both。
- `setScrollPhysics` 增加可选的 accept：`0` content、`1` always、`2` never。不传仍是 content。
- `ScrollArea` 增加 `axis`（缺省 `vertical`）和 `physics.accept`（缺省 `always`）。这是行为修正：竖向列表不再因为内容比视口宽就横滚。
- `VirtualList` 内部滚动固定 `vertical`。
- `set_scroll_offset`、`scroll_limits`、fling、overscroll、snap、滚轮、`scroll_into_view` 对未声明的轴恒为当前值或 0。消费端读到的跨轴余量是 0。
- `Gesture::DragBy` 的签名不变，自定义拖拽回调仍拿到原始增量。

迁移：

1. 要横滚的 `ScrollArea` 写 `axis="horizontal"`。
2. 嵌套时希望到边把拖拽让给父级，把 `physics.accept` 设成 `"content"`。不可滚动的区域设成 `"never"`。
3. 直接调 `core.scroll()` 且要单轴时，传入轴编码，或随后 `setScrollAxis`。

## 2. 可行性评估

改动集中在已有写入口，不新增识别器，不改 `protocol`。

- `Tree::scroll_for`、`can_consume`、`overflows`（`core/src/tree/hit.rs`）只改认领。
- `Tree::scroll_limits`、`set_scroll_offset`、`arm_fling`、`scroll_to`、`reveal`、`merge_wheel`、`snap_target`、`can_take_velocity`（`core/src/tree/scroll.rs`）按 `ScrollAxis::allows` 过滤。
- `Tree::pointer_wheel`（`core/src/tree/gesture.rs`）只把声明轴上的滚轮分量交给上面的入口。
- `ScrollController::drag_by`、`wheel`、`consume`（`core/src/list/controller.rs`）在写入前丢掉单轴的跨轴增量。`limits == 0` 仍表示「`Both` 且没有内容」，溢出交给父级。
- `DragClaim` 与 `RecognizerSet.drag` 不改。
- `Both` + `Content` 是缺省，既有 `cargo test` 里的双向滚动、贴边锁和同轴接力保持原样。

完全可行。

## 3. 当前表面

- `Tree::scroll()` — 创建 `Both` 视口。
- `Tree::scroll_with(axis)` — 创建时带上轴。
- `Tree::set_scroll_axis` / `scroll_axis` — 改轴，并清掉另一轴的偏移、overscroll 和运动。
- `Tree::set_scroll_physics` — 整份替换；`accept` 跟着这份物理走。
- `Tree::set_scroll_offset` — 唯一的偏移写入口。未声明轴写成 0，overscroll 也是 0。
- `Tree::scroll_limits` — 未声明轴为 0。
- `Tree::arm_fling` / `fling_target` / `scroll_to` / `scroll_into_view` — 只动声明轴。
- `Tree::scroll_for` — 认领。`lock_at_edge` 只记录 `Both`。
- `Tree::pointer_wheel` — 手势里的滚轮增量已按轴裁过。
- `ScrollController` — 夹紧和父级溢出。单轴不把跨轴增量报成 overflow。
- `core.scroll(axis?)`、`core.setScrollAxis`、`core.setScrollPhysics(..., accept?)` — QuickJS。`glue.c` 与 `linked.rs` 的函数表一起改。
- `<scroll axis>`、`<ScrollArea axis>`、`VirtualList` 内部滚动、`Field` 里的滚动。

## 4. 设计

轴从「每次 move 猜主轴」改成节点上的声明。

| | 现状 | 目标 |
|---|---|---|
| 认领 | `horizontal = \|dx\| >= \|dy\|`，任意轴溢出都能认 | 只看声明轴。纯跨轴拖动不认领 |
| 锁定后 | `DragBy(dx, dy)` 全量进偏移 | `DragBy` 仍全量。`set_scroll_offset` 丢掉未声明轴，含 overscroll |
| limits | 两轴都是 content − view | 未声明轴恒 0 |
| fling / snap / 滚轮 / reveal | 两轴各自算 | 只动声明轴。接力只在同轴祖先还有余量时发生 |
| 到边 | `lock_at_edge` 锁最内层溢出者 | 单轴用 `accept`。`Both` 保留 `lock_at_edge` |

嵌套：手指斜拖 `(dx = +6, dy = +5)`，命中链是内层竖向列表、外层横向 pager。竖向列表在声明轴上有余量就认领。锁定后 dy 生效，dx 丢掉，pager 不动。纯横向拖动（`dy = 0`）竖向列表不认领，pager 在自己的轴上认领。这和「各自只竞争自己的轴，内层先赢」一致。竖向 fling 到边只接力给还能吃 Y 的祖先；横向 pager 吃不下 Y，所以不动。

### 关键决策

1. 轴语义住在 `Tree`，不住在指针层。`set_scroll_offset` 和 `scroll_limits` 是偏移的写入口，在这里过滤后，命中、录制、语义动作和 JS 的 `setScrollOffset` 同一套结果。不在 `drag_by` 里裁 `Gesture::DragBy`：选择拖拽和自定义拖拽回调要原始增量。被拒绝的做法是在手势层拆掉 dx。
2. 三值轴，缺省 `Both`。`ScrollAxis::{Vertical, Horizontal, Both}`。双向画布继续用 `Both`，行为与今天相同。`ScrollArea` 缺省 vertical，对齐常见列表；横向调用方显式传 horizontal。被拒绝的做法是把缺省改成 vertical，那会改掉裸 `scroll()` 和现有测试。
3. `ScrollAccept::{Content, Always, Never}`，缺省 `Content`。`Content` 是今天的「有余量才认领」。`Always` 在声明轴上到边也认领，橡胶带和刷新靠它。`Never` 不认领，父链继续。被拒绝的做法是用 `lock_at_edge` 表达到边回弹：那个兜底没有回弹语义，而且会锁住不该锁的兄弟。
4. 单轴不用 `lock_at_edge`。`ScrollArea` 和 `VirtualList` 因此缺省 `Always`，否则列表滑到底会变成点击，下拉刷新也没有 overscroll。想嵌套让位时传 `content`。被拒绝的做法是单轴也走 `lock_at_edge`，那会让 `accept` 失去「到边认不认」的唯一来源。
5. 控制器单独读轴。`limits == 0` 对 `Both` 仍是「没有内容，溢出给父级」。单轴的跨轴增量在 `drag_by` / `wheel` / `consume` 里先变成 0，所以不会变成 overflow。被拒绝的做法是只改 `scroll_limits` 就当控制器自动正确。
6. 接力不跨轴。`can_take_velocity` 先看 `ScrollAxis::allows`。被拒绝的做法是松手时把竖向列表丢掉的 vx 送给横向 pager：列表中段的斜向松手也会带动 pager，和「锁定后 pager 不参与」矛盾。
7. `Field` 按单行 / 多行声明轴。被拒绝的做法是让输入框继续 `Both`：一行比视口宽时，斜向拖动会横滚输入框，`scroll_into_view` 也会去弹不该动的那一轴。

```rust
pub enum ScrollAxis {
    Vertical,
    Horizontal,
    Both,
}

pub enum ScrollAccept {
    Content,
    Always,
    Never,
}

impl Tree {
    pub fn scroll_with(&mut self, axis: ScrollAxis) -> Result<NodeId, Error>;
    pub fn set_scroll_axis(&mut self, id: NodeId, axis: ScrollAxis) -> Result<(), Error>;
    pub fn scroll_axis(&self, id: NodeId) -> Result<ScrollAxis, Error>;
    pub fn set_scroll_physics(&mut self, id: NodeId, physics: ScrollPhysics) -> Result<(), Error>;
}
```

```tsx
<scroll axis="vertical" />
<ScrollArea axis="horizontal" physics={{ accept: "content", overscroll: "clamp" }} />
```

`VirtualList` 不把 `axis` 暴露给调用方，内部写死 `vertical`。

## 5. 分阶段实施

### Phase 1：core 轴语义

**File: `core/src/motion.rs`**

- `ScrollAxis`、`ScrollAccept`。`allows` / `filter` / `from_code`。
- `ScrollPhysics.accept`，缺省 `Content`。

**File: `core/src/tree/node.rs`**

- `ScrollGeom.axis`，新建为 `Both`。

**File: `core/src/tree/store.rs`**

- `scroll()` 转到 `scroll_with(Both)`。

**File: `core/src/tree/scroll.rs`**

- `scroll_limits`、`set_scroll_offset`、`arm_fling`、`scroll_to`、`reveal`、`merge_wheel`、`snap_target`、`can_take_velocity`、`apply_scroll`、`apply_overscroll`、`arm_overscroll_spring` 按轴过滤。
- `set_scroll_axis` 清掉另一轴的运动，再写一次偏移，把跨轴 overscroll 收到 0。

**File: `core/src/tree/hit.rs`**

- `can_consume` 按轴和 accept。`dy == 0` 的竖向滚动不认领。
- `lock_at_edge` 只记下 `Both` 且 accept 不是 `Never` 的溢出者。

**File: `core/src/scroll_test.rs`**

- `Both` 仍能两轴滚动、贴边仍锁。
- vertical：超宽不横移、斜拖只留下 y、fling 不 arm x、跨轴 overscroll 为 0。

### Phase 2：physics accept

**File: `core/src/tree/hit.rs`**

- `Always` 在声明轴上有分量就认领。`Never` 整段跳过，不进入 `lock_at_edge`。

**File: `core/src/scroll_test.rs`**

- `Always` 在起点向下拖仍认领，随后的越界只出现在 y。
- `Content` 的竖向滚动在起点不认领。
- `Never` 不锁内层。

### Phase 3：滚轮、接力、reveal、控制器

**File: `core/src/tree/gesture.rs`**

- `pointer_wheel` 先认领，再 `filter`。竖向列表上的横向滚轮没有 `Wheel`。

**File: `core/src/tree/scroll.rs`**

- 同轴 `chain` 保持。`can_take_velocity` 拒绝未声明轴，横向 pager 接不住竖向速度。
- `reveal` 只弹簧声明轴。

**File: `core/src/list/controller.rs`**

- `refresh_boxes` 记住轴，并把 limits 裁到声明轴。
- `drag_by`、`wheel`、`consume` 先 `filter`。

**File: `core/src/list/tests.rs`**

- 竖向控制器吃掉 dx，`overflow_x` 为 0，y 仍前进。
- `Both` 控制器仍能横移。

### Phase 4：TS 表面与组件

**File: `runtimes/quickjs/src/glue.c`**、**`runtimes/quickjs/src/linked.rs`**

- `scroll(session, axis, out)`。`setScrollAxis`。`setScrollPhysics` 多一个 accept。

**File: `frameworks/solid-renderer/src/host.ts`**、**`tree.ts`**、**`jsx.ts`**

- `scroll(axis?)`、`setScrollAxis`。`<scroll axis>`。

**File: `frameworks/solid-ui/src/ScrollArea.tsx`**

- `axis` 缺省 vertical。`accept` 缺省 always。夹紧和 `onDragBy` 不改，核心已经裁过。

**File: `frameworks/solid-ui/src/VirtualList.tsx`**

- 传 `axis="vertical"`。

**File: `frameworks/solid-ui/src/field/shell.tsx`**

- 单行 horizontal，多行 vertical。

**File: `apps/gallery-solid/counter.tsx`**

- 列表页的横向 snap 行标成 `axis="horizontal"`，标题「横向分页」。

### Phase 5：文档

**File: `AGENTS.md`**、**`docs/architecture.md`**、**`docs/driver.md`**

- 轴、accept、跨轴丢弃、控制器与 `Both` 溢出的差别。手势段写明锁轴后跨轴增量在 `set_scroll_offset` 丢掉。

验证：`cargo test -p core --lib` 里的滚动用例，`just solid`，以及 gallery 的 `counter.node.test.ts`。真机手势（竖、横、斜、到边、嵌套、滚轮）不在 `just check` 里，窗口路径仍用 `just host`。

## 6. 架构说明

- 不改 `protocol`。轴是树属性，不进 `DrawList`。
- 不做场景 diff，不加新的手势识别器。
- `DragClaim` 与 `RecognizerSet.drag` 不动。它们认领的是内容拖拽，和滚动轴无关。
- `Both` 留下。画布类双向滚动与今天相同。
- Kotlin 无新增文件。以后的属性名对着 `set_scroll_axis` 和 `ScrollPhysics.accept`。
- QuickJS 的 C 仍只在 `runtimes/quickjs/src/glue.c`。函数表多一个 `set_scroll_axis`，`scroll` 与 `set_scroll_physics` 的参数在同一张表里一起改。
- 语义动作 `Scroll { dx, dy }` 走 `set_scroll_offset`，自动丢掉未声明轴。

## 7. 文件变更清单

- `AGENTS.md` — Scroll 轴、accept、跨轴丢弃
- `apps/gallery-solid/counter.tsx` — 横向分页声明 horizontal
- `core/src/lib.rs` — 导出 `ScrollAxis`、`ScrollAccept`
- `core/src/list/controller.rs` — 单轴增量不进 overflow
- `core/src/list/tests.rs` — 控制器轴矩阵
- `core/src/motion.rs` — `ScrollAxis`、`ScrollAccept`
- `core/src/scroll_test.rs` — 等价基线与轴矩阵
- `core/src/tree/gesture.rs` — 滚轮按轴路由
- `core/src/tree/hit.rs` — 按声明轴认领
- `core/src/tree/node.rs` — `ScrollGeom.axis`
- `core/src/tree/scroll.rs` — 偏移、fling、reveal、接力按轴过滤
- `core/src/tree/store.rs` — `scroll_with`
- `docs/architecture.md` — 当前滚动语义
- `docs/driver.md` — 控制器丢掉单轴的跨轴增量
- `docs/scroll-axis.md` — 本说明
- `frameworks/solid-renderer/src/host.ts` — `scroll` / `setScrollAxis` / accept
- `frameworks/solid-renderer/src/jsx.ts` — `scroll` 的 `axis`
- `frameworks/solid-renderer/src/tree.ts` — 属性写入
- `frameworks/solid-renderer/src/tree.test.ts` — mock
- `frameworks/solid-ui/src/ScrollArea.tsx` — 缺省 vertical 与 always
- `frameworks/solid-ui/src/ScrollGroup.tsx` — 注释与控制器分工一致
- `frameworks/solid-ui/src/VirtualList.tsx` — 固定 vertical
- `frameworks/solid-ui/src/field/shell.tsx` — 单行横向、多行纵向
- `frameworks/solid-ui/testing/renderTest.ts` — mock
- `runtimes/quickjs/src/glue.c` — `scroll` 轴参、`setScrollAxis`、accept
- `runtimes/quickjs/src/linked.rs` — 同上，接到 `Tree`
