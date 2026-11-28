# 前置改造 2：Engine View 级重绘追踪与无侵入远程 Overlay

> **适用范围与前置约束**
> - 本文档细化 `crates/engine` 侧对 **View 累计重绘次数、每帧重绘 View 明细（路径与根因）以及设备端无侵入视觉叠加层（Highlight / Pick / Repaint Rainbow）** 的具体实现方案。
> - **依赖隔离边界**：全部追踪与叠加层逻辑留在 `crates/engine` 纯 Rust 树模型内部，不依赖 `raster`、`wgpu`、`rquickjs` 或任何平台 API（[check-javascript-boundaries.py](../tools/check-javascript-boundaries.py)）。
> - **零开销与单路径铁律**：当 DevTools 未连接或未开启重绘追踪时，仅存在一次 `Option::is_some()` 判空，零堆分配与零额外遍历；Overlay 命令直接附着于现有 `LayerTree` 录制主干末尾，不修改业务树结构，不引入第二条渲染管线。

---

## 1. 改造目标与文件清单

| 目标文件 | 动作 | 核心职责 |
|---|---|---|
| [debug.rs](../crates/engine/src/tree/debug.rs) | 修改 | 定义 `RepaintTracker`、`PaintPath`、`RepaintReason`、`NodeRepaintStats`、`FrameRepaintRecord`、`RemoteOverlayState` |
| [mod.rs](../crates/engine/src/tree/mod.rs) | 修改 | 在 `Tree::discard_detached` / `retire` 清理节点追踪统计；导出相关诊断类型 |
| [layout.rs](../crates/engine/src/tree/layout.rs) | 修改 | 在 `mark_repaint`、`repaint_ancestors`、`mark_layout` 等处记录失效触发原因 |
| [retained.rs](../crates/engine/src/tree/record/retained.rs) | 修改 | 在 `update_*_slot` 与 `apply_layer_patches` 记录 `FastLayerPatch` 与 `PicturePatch` 真实发生 |
| [record/mod.rs](../crates/engine/src/tree/record/mod.rs) · [layers.rs](../crates/engine/src/tree/record/layers.rs) | 修改 | 在 `close_run` / `emit_picture_owner` / `fill_children` 记录 `FullPictureRebuild` 与 `CanvasRedraw`；在 `record_scene` 末尾注入 `RemoteOverlay` |
| [session/mod.rs](../crates/engine/src/session/mod.rs) · [player/mod.rs](../crates/engine/src/js/player/mod.rs) | 修改 | `FrameOutput` 增加 `repaints`，`Session` trait 暴露重绘开关、排行榜查询、Overlay 控制与单帧节点拾取 |
| [ui_frame.rs](../crates/embedder/common/src/host/ui_frame.rs) · [hub.rs](../crates/embedder/common/src/diagnostics/hub.rs) | 修改 | 接收 `FrameOutput.repaints` 写入 `TelemetryHub` 帧环与重绘频道 |

---

## 2. 核心数据结构设计（`crates/engine/src/tree/debug.rs`）

### 2.1 重绘路径与触发原因

Impasto 的渲染录制存在明确的 4 条更新路径（见 [retained.rs](../crates/engine/src/tree/record/retained.rs) 与 [record/mod.rs](../crates/engine/src/tree/record/mod.rs)），追踪器精准区分它们以直观反映“本次重绘到底贵不贵”：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PaintPath {
    /// 仅更新已安装 Layer 的标量属性（rect fill/stroke、transform、opacity、clip radius），无 Picture 重录
    FastLayerPatch = 0,
    /// 局部重录单个 PictureOwner 的命令列表
    PicturePatch = 1,
    /// 未命中 Picture 复用缓存或跨层结构变化，触发 Picture 全量重录
    FullPictureRebuild = 2,
    /// Canvas 回调函数重新录制绘制指令
    CanvasRedraw = 3,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct RepaintReason: u16 {
        const STYLE_PAINT          = 1 << 0; // 背景/边框/阴影/渐变等绘制属性变更
        const LAYOUT_GEOMETRY      = 1 << 1; // Taffy 布局尺寸或位置变更
        const TEXT_CONTENT         = 1 << 2; // 文本内容或排版属性变更
        const TRANSFORM_OR_OPACITY = 1 << 3; // Transform 或 Opacity 槽位更新
        const CLIP_OR_RADIUS       = 1 << 4; // 圆角或裁剪属性变更
        const EFFECT_LAYER         = 1 << 5; // Blur / Backdrop / Blend / ColorFilter 变更
        const CANVAS_CALLBACK      = 1 << 6; // Canvas 主动重绘或几何依赖失效
        const ANCESTOR_INVALIDATED = 1 << 7; // 子节点层结构翻转导致祖先 run 重建
        const TREE_STRUCTURE       = 1 << 8; // 节点增删或兄弟顺序变动
        const INITIAL_MOUNT        = 1 << 9; // 首帧挂载录制
    }
}
```

### 2.2 节点累计统计与单帧明细

```rust
/// 单个 NodeId 的生命周期累计重绘画像（按 NodeId 的 index + generation 唯一标识，防止 slot 复用串号）
#[derive(Clone, Debug, Default)]
pub struct NodeRepaintStats {
    pub node_raw: u64,
    pub generation: u32,
    pub total_repaints: u64,
    pub total_layouts: u64,
    pub fast_patch_count: u64,
    pub picture_patch_count: u64,
    pub full_rebuild_count: u64,
    pub canvas_redraw_count: u64,
    pub last_frame: u64,
    pub last_path: Option<PaintPath>,
    pub last_reason: RepaintReason,
    /// 最近 64 帧是否发生重绘的滑动位图（bit 0 为最新帧），O(1) 计算 `recent_mask.count_ones()`
    pub recent_mask: u64,
}

/// 单帧内单个被重绘 View 的完整记录
#[derive(Clone, Debug)]
pub struct FrameRepaintRecord {
    pub node_raw: u64,
    pub kind: &'static str,
    pub rect: [f32; 4], // [x, y, w, h]
    pub path: PaintPath,
    pub reason: RepaintReason,
    pub caused_layout: bool,
    pub picture_id: Option<u64>,
    pub command_count: u32,
    pub component: Option<String>,
    pub source_path: Option<String>,
    pub source_line: u32,
    pub source_column: u32,
}

pub const FRAME_REPAINT_CAP: usize = 256;

#[derive(Clone, Debug, Default)]
pub struct FrameRepaintBatch {
    pub frame_number: u64,
    pub total_repainted_views: u32,
    pub overflow_count: u32,
    pub fast_patch_views: u32,
    pub picture_patch_views: u32,
    pub full_rebuild_views: u32,
    pub canvas_redraw_views: u32,
    pub entries: Vec<FrameRepaintRecord>,
}
```

### 2.3 追踪器状态机（`RepaintTracker`）

```rust
pub(crate) struct RepaintTracker {
    pub(crate) enabled: bool,
    pub(crate) frame_number: u64,
    /// 节点累计表，与 Tree 生命周期同步并在 discard 时清理
    pub(crate) nodes: HashMap<u32, NodeRepaintStats>,
    /// 本帧录制开始前收集到的“待解释原因”暂存表（dirty reason staging）
    pending_reasons: HashMap<u32, (RepaintReason, bool)>, // (reasons, caused_layout)
    /// 本帧实际完成重绘的节点去重汇总（一个节点同帧可能经过多次合并，保留最高代价路径）
    frame_events: HashMap<u32, PendingRepaintHit>,
    /// `LayerId -> NodeId` 反向映射，供 `apply_layer_patches` 归属节点
    layer_owners: HashMap<u32, NodeId>,
}
```

> **核心设计要点**：**区分“标脏”和“真正发生绘制”**。在 JS 响应式批处理中，同一个节点一帧可能被多次 `set_background` / `mark_repaint`。因此：
> 1. `mark_repaint` / `update_*_slot` / `mark_layout` 只向 `pending_reasons` 合并写入 `RepaintReason` 与 `caused_layout`；
> 2. 只有当 `record_scene` / `apply_layer_patches` **真正生成了新层补丁、重建了 Picture 或重绘了 Canvas** 时，才调用 `commit_repaint(id, path, picture_id, cmd_count)` 消耗原因并计入本帧 `frame_events` 与 `nodes` 累计表！

---

## 3. 精确引擎插桩点与逻辑流转

### 3.1 原因登记点（Staging）

1. **[layout.rs](../crates/engine/src/tree/layout.rs)**：
   - `mark_repaint(id)`：向 `id` 登记 `STYLE_PAINT`（或由调用方传入具体 `RepaintReason` 枚举，默认细分 `set_text` → `TEXT_CONTENT`、`set_gradient`/`set_shadow`/`set_stroke` → `STYLE_PAINT`、`add_child`/`remove` → `TREE_STRUCTURE`）；
   - `repaint_ancestors(id)`：对 `id` 登记结构/效果原因，对沿途祖先标记 `ANCESTOR_INVALIDATED`；
   - `mark_layout()` 及 Taffy 回写几何坐标处：当某个节点 `x/y/w/h` 实际发生变化时，向该节点登记 `(RepaintReason::LAYOUT_GEOMETRY, caused_layout = true)`。
2. **[retained.rs](../crates/engine/src/tree/record/retained.rs)**：
   - `update_rect_slot(id)` → 登记 `STYLE_PAINT`；
   - `update_transform_slot(id)` / `update_opacity_slot(id)` → 登记 `TRANSFORM_OR_OPACITY`；
   - `update_clip_radius(id)` → 登记 `CLIP_OR_RADIUS`。

### 3.2 真实重绘确认点（Commit）

1. **增量层补丁路径（[retained.rs](../crates/engine/src/tree/record/retained.rs) `apply_layer_patches`）**：
   - 遍历 `picture_patches` 时，仅当 `prior.commands().as_ref() != commands.as_slice()` 真正产生新 `Picture` 时，对 `node` 调用 `commit_repaint(node, PaintPath::PicturePatch, Some(prior.id.0), commands.len())`。
   - 遍历 `self.retained_layers.pending.drain()` 时，仅当 `layer.kind != kind` 真正改写了层值，通过 `layer_owners.get(&id.0)` 找到归属 `NodeId`，调用 `commit_repaint(node_id, PaintPath::FastLayerPatch, None, 0)`。
2. **全量图层重建路径（[record/mod.rs](../crates/engine/src/tree/record/mod.rs) 与 [layers.rs](../crates/engine/src/tree/record/layers.rs)）**：
   - 在 `push_layer(id, ...)` 时顺便记录 `layer_owners.insert(layer_id.0, id)`。
   - 在 `close_run()` 中，当未命中 `matched.commands().as_ref() == captured.as_slice()`（第 680-696 行分支）而分配新 `Picture` 时，对 `scope.owner`（以及该 run 内存在 `pending_reasons` 的成员节点）记 `PaintPath::FullPictureRebuild`。
   - 在 `emit_picture_owner(id, ...)` 中，当 `!self.picture_fresh(id)` 走 `self.fill_owner(id, abs)` 时，对 `id` 记 `PaintPath::FullPictureRebuild`。
3. **Canvas 重绘路径（`crates/engine/src/canvas/` & `record`）**：
   - Canvas 回调重新执行并写入新绘制序列时，通过 `commit_repaint(id, PaintPath::CanvasRedraw, ...)` 升级覆盖同帧标记（因 `CanvasRedraw > PicturePatch`）。
4. **帧末结算（`Tree::record_scene` 尾部）**：
   - 遍历所有存活的 `nodes` 将 `recent_mask` 左移 1 位（若本帧有事件则置 `bit 0 = 1`）；
   - 将 `frame_events` 按优先级（`CanvasRedraw` / `FullPictureRebuild` / `PicturePatch` / `FastLayerPatch`）与节点绘制面积降序排列，截断保留前 `FRAME_REPAINT_CAP`（256）条生成 `FrameRepaintBatch`；
   - 绑定 `self.debug.sources.get(&node.index)` 中的 `component`、`path`、`line`、`column`，同时清零 `pending_reasons`。
5. **节点销毁清理（[tree/mod.rs](../crates/engine/src/tree/mod.rs) `retire` / `discard_detached`）**：
   - 节点被 `retire` 时同步从 `nodes`、`pending_reasons` 与 `debug.sources` 移除；`nodes` 命中时校验 `stats.generation == id.generation`，彻底杜绝 slot 复用的历史累计串号。

### 3.3 防内存膨胀与无连接零成本

- **按帧切换开关**：`Session::set_repaint_tracking(bool)`，仅在 DevTools Client 订阅 `repaints` 通道或开启 `Repaint Rainbow` 时激活；客户端断开后自动关闭。
- **调试树隔离**：所有 `is_debug_owned(id)` 节点（含内置旧版 devtools 层）直接跳过追踪。

---

## 4. 无侵入远程 Overlay（Highlight / Pick / Repaint Rainbow）

### 4.1 为什么不在目标 App 里插 JSX 节点？

如果为了画选中框或重绘彩虹而在目标 App 的 `Tree` 里插入节点：
- 会触发业务根节点的重排/重建；
- 会污染 `debug_snapshot` 与命中测试（虽然 `debug_owned` 可过滤，但仍占用了 `NodeId` slot 与 JS 桥接开销）；
- 若目标 App 本身是一个未引入 `@impasto/devtools` 的空白应用，也能开箱使用高亮与拾取。

因此，采用 **Engine 录制尾部瞬时图层注入** 方案。

### 4.2 状态定义与淡出驱动

```rust
#[derive(Clone, Debug, Default)]
pub struct RemoteOverlayState {
    pub highlight_node: Option<u64>, // NodeId::to_raw()
    pub rainbow_enabled: bool,
    pub pick_mode: bool,
    pub(crate) rainbow_boxes: Vec<RainbowBox>,
}

#[derive(Clone, Debug)]
pub(crate) struct RainbowBox {
    pub rect: [f32; 4],
    pub path: PaintPath,
    pub ttl_frames: u8,     // 例如初始 24 帧（120Hz 约 200ms，60Hz 约 400ms）
    pub max_ttl: u8,
    pub hue_seed: u8,
}
```

- **保留高位 PictureId**：在 `crates/engine/src/tree/picture_id.rs` 保留专用高位 `PictureId`（例如 `u64::MAX - 1`）专供远程 Overlay 图层使用，避免与业务节点的 `self_picture_id` 冲突。
- **渲染注入时机**：在 `Tree::record_scene()` 完成全部业务录制（`self.finish_compositor_record()`）之后、写入 `self.layers.epoch` 之前：
  1. 若 `rainbow_enabled`，从本帧 `FrameRepaintBatch` 取最多 32 个非零矩形写入 `rainbow_boxes`（上限 64 个，FIFO 覆盖）；
  2. 为 `highlight_node`（校验存活并取绝对场景边界，含滚动偏移）生成 1 个半透明蓝色填充（`rgba(56, 139, 253, 0.16)`）+ 1.5pt 蓝色描边矩形，以及外侧尺寸标签提示条（可选）；
  3. 为每个活跃 `RainbowBox` 按 `ttl_frames / max_ttl` 衰减 alpha 绘制 1.5pt 彩色描边矩形（颜色随 `hue_seed` 轮转，或按 `PaintPath` 语义着色）；
  4. 将这些 `Command::Rect` 打包成一个根级 `LayerKind::Offset { x: 0.0, y: 0.0 }` → `LayerKind::Picture` 挂载到 `self.layers` 链表尾部；
  5. 若 `rainbow_boxes` 中仍有活跃项，每帧递减 `ttl_frames`，并使 `Session::wants_frame()` 返回 `true` 以自动推进淡出动画，直到全部消散后自动停止请求帧（恢复 0 GPU/CPU 功耗）。
- **快速补丁兼容**：当 `remote_overlay` 激活（或刚刚清空）时，`can_patch_scene()` 返回 `false`（或单独附着在增量路径末尾），保证高亮框与彩虹边框永不被旧帧跳过。

### 4.3 Pick Mode（设备端交互拾取）链路

1. 桌面端点击 `🎯 Pick` → 发送 `view.set_overlay({ pick_mode: true })`。
2. `UiHost` 开启引擎 `Tree::set_pointer_trace(true, true)`（复用 [debug.rs:303](../crates/engine/src/tree/debug.rs#L303) 已有能力，`suppress = true` 确保触摸/点击手机屏幕不触发业务按钮跳转）。
3. 当用户在目标设备屏幕上按下/移动指针（`Phase::Down` / `Phase::Move`）时，`Tree::note_pointer` 触发实时 `debug_probe(x, y)`，更新 `highlight_node = chain.last()` 并向 `TelemetryHub` 推送一条 `view.picked { node_id, chain, x, y, final }` 事件；
4. 若为 `Phase::Up` 且完成锁定，可选择自动关闭 `pick_mode`（或保持连续拾取直至用户在桌面端取消），并将选中节点同步高亮在桌面端 View 树中。

---

## 5. `Session` 扩展与 `TelemetryHub` 联动

- **扩展 [crates/engine/src/session/mod.rs](../crates/engine/src/session/mod.rs)**：
  - `FrameOutput` 新增字段 `pub repaints: Option<crate::tree::FrameRepaintBatch>`；
  - `Session` trait 新增默认实现方法：
    - `fn set_repaint_tracking(&mut self, _enabled: bool) {}`
    - `fn set_remote_overlay(&mut self, _config: RemoteOverlayConfig) {}`
    - `fn repaint_leaderboard(&self, _limit: usize) -> Vec<NodeRepaintStats> { Vec::new() }`
    - `fn take_picked_view(&mut self) -> Option<PickedViewEvent> { None }`
- **在 [ui_frame.rs](../crates/embedder/common/src/host/ui_frame.rs) 与 [hub.rs](../crates/embedder/common/src/diagnostics/hub.rs) 汇入**：
  - `TelemetryHub` 新增有界环形缓冲 `repaints: Mutex<Ring<FrameRepaintBatch>>`（cap `240`，与 `frames` 对齐）。
  - `FrameSample` 新增摘要计数 `repainted_views: u32`、`full_rebuild_views: u32`。

---

## 6. 测试与验证清单

1. **Engine 单元测试（`cargo test -p engine`）**：
   - 验证修改背景色走 `FastLayerPatch`、局部重录走 `PicturePatch`、子层结构变更走 `FullPictureRebuild`、Canvas 重绘走 `CanvasRedraw`；
   - 验证同帧内同一节点先触发 `STYLE_PAINT` 再触发 `LAYOUT_GEOMETRY` 后原因位标志合并且保留最高优先级路径；
   - 验证节点 `retire` + 新节点分配复用同一 slot index 后，`NodeRepaintStats` 从 0 重新统计而不继承旧节点计数；
   - 验证 `repaint_tracker == None` 时无分配且行为与改造前 100% 一致；
   - 验证开启 `highlight_node` 后，`debug_snapshot` 与 `debug_probe` 不包含 Overlay 节点。
2. **宿主集成测试（`cargo test -p embedder`）**：验证 `FrameOutput.repaints` 流入 `TelemetryHub` 并可通过 RPC `frame.repaints` 与 `view.set_overlay` 正常交互。
