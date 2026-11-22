# 为 Impasto 增加 Canvas 2D

> 文中的实施阶段与代码引用反映设计或审计时的仓库结构。当前 crate 职责、源码入口和验证命令见 [架构](architecture.md)。

## 破坏性变更与迁移

本文描述已落地的屏幕 Canvas。离屏画布、像素读回、pattern、阴影、滤镜、strokeText、路径布尔运算，以及 copy / source-in / destination-in 仍留待后续。公共组件采用 `Canvas` 与 `draw(context, size)`，统一承载图表、自绘控件和二维图形。挂载时设置 `deferWhenOffscreen` 可在内容盒进入视口后绘制最新数据，视口外期间的绘制和错误通知可以延后。`polyline`、`lineSegments` 和 `fillRects` 接收 `Float64Array`，分别按坐标对、线段和矩形分组，并沿用当前路径或画笔。

拟新增 `protocol::Command::Canvas`；文字阶段给 `protocol::TextStyle` 增加具有默认值的 `FontSlant`。两者分别影响枚举穷尽匹配和完整结构体字面量。迁移步骤：

1. 更新 `engine` 的命令播放、资源扫描与诊断，以及 `uicore` 的命令平移与叶绘制判断，覆盖 Canvas。
2. 更新 TextStyle 字面量、相等/哈希、排版缓存键与 SDK 字体样式调用；普通文字默认仍是 Normal。
3. 原有应用逐个迁移节点图表到 Canvas。绘制尺寸放在 style；删除旧图表前搜索全部引用，不保留两套实现开关。
4. 重新构建配套框架和应用 ESM/字节码。Canvas 上下文、Path2D 和渐变由 scene 导出的 Rust 原生类承载；录制数据留在 Rust，不形成应用可序列化的命令包。

## 可行性评估

当前运行时由根 `Cargo.toml` 的 `rquickjs = { package = "quickjs-jit", version = "=0.14.2", ... }` 提供，绑定与会话在 Rust 的 `src/ui/player/vm`。现有 ListDriver/ScrollController 已使用 rquickjs class，响应式回调已由 Persistent 持有，因此 Canvas 上下文、Path2D 和渐变可以沿同一方式直接录制到 Rust。现有 Taffy、picture owner、Graph、Typography 与图片缓存继续复用。SDK 已有路径、矩阵、裁剪、SaveLayer、纹理裁切和段落绘制；通用离屏纹理渲染与读回尚未贯通。**屏幕 Canvas 可行；原生方法调用开销需要实测，离屏导出与像素访问另行交付。**

## 当前接口与消费点

- `packages/solid-ui/src/props.ts:23`、`src/element.tsx:43`：ViewProps、bindBox；样式、ref、语义与事件应复用。
- `packages/solid-ui/src/Icon.tsx:7`：Icon 只组合现有子节点；Canvas 可以成为其图形内容，但不替换 Icon 的方向/语义职责。
- `packages/solid-renderer/src/scene.ts:417`：metrics；目前只有窗口及 viewInsets，不足以表达任意节点变换后的局部点。
- `packages/solid-renderer/runtime/frames.ts:1`：onFrame 与 onLayout；前者每帧执行，后者只在确实布局的帧执行，均不能单独充当 Canvas 的数据重绘阶段。
- `packages/solid-renderer/runtime/index.ts:232`：boot 安装帧与布局监听，不应再在这里无条件遍历所有 Canvas。
- `packages/reactive/src/api.d.ts:6`：现有 memo/effect API；没有 createReaction，不能在设计中假定已有延后跟踪接口。
- `crates/reactive/src/lib.rs:208`、`:293`：Graph 的 memo/effect、has_updates、run_updates；现有 scheduled 参数只控制首次执行，不是布局后的独立队列。
- `Cargo.toml:29`、`crates/javascript/Cargo.toml`：依赖名 rquickjs 对应钉住的 quickjs-jit 0.14.2，启用 macro/loader/bindgen；解释器始终由 Cargo 构建。
- `crates/javascript/src/ui/player/vm/mod.rs:438`：`rt::Engine` 拥有 Host、Context、Runtime 与 Inner；Host::with 复用同步入口的活动 Ctx。
- `crates/javascript/src/ui/player/vm/mod.rs:29`、`:553`：References 中的 Persistent 与 Engine::shutdown；先清理图和全部持久值，再释放 Context/Runtime，Host/Inner 活过 class finalizer。
- `crates/javascript/src/ui/player/vm/drivers.rs:7`：ListDriver/ScrollController 的 Trace、JsLifetime、class/methods、index/generation 和释放路径，是 Canvas 原生类的直接接入依据。
- `crates/javascript/src/ui/player/vm/bindings.rs:267`：style_snapshot 先解码为拥有型 Rust 值，再由 OwnedRec 提交。Canvas 的录制对象原本就在 Rust，提交时不需要重做 JS 数组解码。
- `crates/javascript/src/ui/player/vm/modules.rs:51`：scene_names 与 SceneModule；Path2D 构造器需要声明、初始化和导出，不能只在 TS 增加类型。
- `crates/javascript/src/ui/player/vm/turns.rs:265`、`src/ui/player/vm/mod.rs`：vm_run_updates、Host::budget 与中断入口。Canvas 绘制需要同类 Rust turn，继承单调时钟预算与入口归因。
- `crates/javascript/src/ui/player/mod.rs:831`、`:865`、`:890`：layout、vm_layout、最终 layout 与 record；绘图回调插在最终 layout 和 record 之间，并先结束 Tree 借用。
- `crates/javascript/src/ui/player/reactive.rs:9`：Value 的 Persistent 与 JsEval；复用同一张图与 owner，Canvas 局部绘制错误需要在专用入口截获，不能直接触发普通 reactive_error 的整帧失败状态。
- `crates/javascript/src/ui/player/mutation.rs:266`、`abi.yml`、`build.rs`：OwnedRec 持有 Rust payload；abi.yml 生成 Rust/JS/TS 常量，build.rs 不生成类型解码器或 C 头文件。
- `crates/uicore/src/tree/node.rs`、`tree/mod.rs`：节点 payload 与 Tree；Canvas 可作为普通 View 的可选绘图内容，不增加 Kind。
- `crates/uicore/src/tree/layout.rs:178`：picture owner 布尔标志；需要区分 ListDriver 与 Canvas 的使用来源。
- `crates/uicore/src/tree/record/mod.rs:1163`：translate_command；Canvas 只平移外层原点，内部局部命令不逐点改写。
- `crates/uicore/src/tree/hit.rs:24`：现有命中与坐标反变换；复用它导出节点局部点，不能仅做 pointer 减 rect。
- `crates/uicore/src/tree/store.rs:729`：image_generation、image_census、note_image；必须计入 Canvas 内的图片。
- `crates/uicore/src/tree/text.rs:539`：measure_text 自动乘 text_scale；Canvas 的显式字号不能直接通过这里再乘一次。
- `crates/uicore/src/shaper.rs:166`、`crates/embedder/common/src/shaper.rs:94`：Shaper 与 ImpellerShaper；测量继续使用引擎同一份 Typography。
- `crates/protocol/src/draw_list.rs:421`、`src/layer.rs`：DrawList 与 LayerTree；尚无 Canvas、通用路径与任意矩阵绘制内容。
- `crates/engine/src/player/ops.rs:8`、`player/paint.rs:493`：播放与已有路径句柄；继续使用 SDK，而非引入平台 Canvas。
- `crates/engine/src/pictures.rs:64`、`textures.rs:45`：picture 复用及纹理 touch；只扫描顶层 Image 会漏掉 Canvas 图片。
- `crates/engine/src/typography/key.rs`、`shape.rs`、`paint_store.rs`：缓存键、度量与绘制；斜体、Canvas 文字画笔和基线都需纳入。
- `crates/embedder/common/src/diagnostics/hub.rs:130`：FrameSample；补充 Canvas 录制/准备成本，继续复用 script/record/submit 指标。
- `apps/gallery-solid/src/pages/stocks.tsx`、`widgets/stocks/`：行情小图、分时、K 线、深度与价格区间。
- `../nsh/crates/nsh-js/src/canvas.rs:1485`、`../nsh/crates/nsh-core/src/canvas.rs`：Canvas 2D 作者接口与录制数据的本地对照；其平台重放、动画槽和离屏实现不进入 Impasto。

## 设计

### 1. 关键决策

1. **Canvas 2D 常用接口 + 每次完整录制。** 保留 beginPath、lineTo、stroke 等作者写法；每次 draw 从透明内容和默认状态开始，成功后替换上一份。相比跨帧累积 bitmap，这与保留式 picture 和响应式重绘相容。
2. **原生上下文录制，渲染继续保留式。** CanvasRenderingContext2D 是 rquickjs class；方法直接修改 Rust Recorder，成功结束时只安装一次不可变结果。TS 负责组件与类型，不保存另一份绘图状态或命令数组。每个方法有 JS→Rust 调用成本，但不调用 GPU、不创建 View。
3. **绘图在最终布局之后、record 之前执行。** 不把 draw 塞入 engine::submit_layers 或普通 onLayout。纯绘图变化不重新布局，布局变化也不会用旧尺寸提交新图。
4. **在现有 Graph 增加内部 paint 阶段。** 不修改作者的 Solid effect 调度语义，不建立第二张依赖图。图形数据失效只标记 paint computation，统一在绘制阶段运行。
5. **用一个通用 Canvas 命令封装局部显示列表。** 每份内容独立管理状态栈、资源引用与合成边界，普通 DrawList 只保留它的放置位置。
6. **几何、排版和 GPU 资源各有所有者。** 采用现成曲线库做路径与描边几何，字体仍走 Impeller Typography，光栅仍是 Impeller。不引入 Skia、平台 CGContext/Android Canvas 或字体引擎。
7. **明确兼容范围。** 支持的 Canvas 2D 方法写清默认值和边界，未支持项不声明、不静默降级。组件生命周期、图片 source 和错误类型使用 Impasto 的约定。

接口语义对照 [HTML Canvas 2D 标准](https://html.spec.whatwg.org/multipage/canvas.html#the-canvas-state)；绘制失效与布局分离、目标混合需要隔离的经验也可见 [CustomPainter 文档](https://api.flutter.dev/flutter/rendering/CustomPainter-class.html)。以下尺寸、事务、调度和版本边界均是 Impasto 的设计约定。

### 2. 公共组件与使用方式

**文件：`packages/solid-ui/src/props.ts`，目标接口：**

```typescript
export interface CanvasSize {
  readonly width: number;
  readonly height: number;
  readonly pixelRatio: number;
  readonly textScale: number;
  readonly reduceMotion: boolean;
}
export interface CanvasController {
  invalidate(): void;
  toLocal(point: { x: number; y: number }): { x: number; y: number } | null;
}
export type CanvasProps = Omit<ViewProps, "children"> & {
  children?: never;
  draw: (context: CanvasRenderingContext2D, size: CanvasSize) => void;
  controller?: (controller: CanvasController | null) => void;
  onDrawError?: (error: CanvasError) => void;
};
```

- 从 `@impasto/solid-ui` 导出 Canvas、CanvasProps、CanvasController 和 Canvas 2D 类型；底层 scene 声明与录制类型在 solid-renderer，不作为应用主入口。
- Path2D 是 scene 导出的 Rust 原生构造器，经 solid-renderer/solid-ui 重导出；CanvasRenderingContext2D 由回调提供，CanvasGradient 由原生 create 方法提供。公开 TS 文件只有声明，运行时对象由 rquickjs class 提供，不向全局安装 DOM 类。
- ref 仍是普通 NodeRef；controller 单独交出失效和坐标转换能力，不让同一个 ref 有时是节点、有时是上下文。
- Canvas 无隐式 300×150；未声明尺寸时遵守普通 View 的布局规则，空内容没有图形固有尺寸。百分比和剩余空间由消费者显式声明。
- 绘图区是 View 的 content box，原点在 border/padding 内侧；width/height 不含 border/padding。绘图裁到该区域，背景、边框在它之外按 View 规则绘制。
- children 不进入 Canvas；文字覆盖层、按钮和可访问内容使用外部 Stack/View 组合。

**文件：`apps/gallery-solid/src/widgets/sparkline.tsx`，目标调用：**

```tsx
<Canvas
  style={{ width: 104, height: 40 }}
  accessibilityLabel="最近行情走势"
  draw={(ctx, size) => {
    const points = series(); // 读取信号，自动登记依赖
    if (points.length === 0) return;
    ctx.strokeStyle = rising() ? "#3CD1A0" : "#FF5D73";
    ctx.lineWidth = 1.5;
    ctx.lineJoin = "round";
    ctx.beginPath();
    ctx.moveTo(0, points[0]);
    for (let i = 1; i < points.length; i++) {
      ctx.lineTo(i * size.width / (points.length - 1), points[i]);
    }
    ctx.stroke();
  }}
/>
```

draw 返回 void 且必须同步；不得在其中 await、创建 effect、登记持久监听、写信号、修改树或再次触发布局。它读取输入并生成这一版内容。对可变外部数组，作者改变后调用 controller.invalidate；修改数组本身不假装具有响应性。

### 3. API 分层与交付边界

| 能力 | 首个屏幕版本 A | 屏幕版本 B | 后续独立交付 |
|---|---|---|---|
| 路径 | begin/close、move/line、二次/三次、rect/roundRect、arc/arcTo/ellipse | Path2D 复用与复制、SVG path 输入 | 路径布尔运算 |
| 绘制 | fill/stroke、fillRect/strokeRect/clearRect、两种 fill rule | 不扩增同义接口 | 像素修改 |
| 画笔 | 实色、线宽、cap/join/miter、alpha、dash/offset | 线性、双圆径向、锥形渐变 | pattern、阴影与 filter |
| 状态 | save/restore、六参数仿射矩阵、clip | getTransform 返回普通矩阵值 | DOMMatrix/DOM 适配 |
| 混合 | source-over、destination-over/out、multiply、screen、lighter、xor | 有完整 clip 范围语义后再加 copy/source-in 等 | 全部模式逐项验收 |
| 文字 | 用外部 Text 组合完成首个图表 | fillText、measureText、字体/对齐/有限基线 | strokeText、完整 ink metrics |
| 图片 | 保留资源设计，不提供假实现 | drawImage 三种参数形态、裁切、采样 | pattern、Canvas/ImageBitmap 源 |
| 输入 | 沿用 ViewEvents，controller.toLocal | isPointInPath/isPointInStroke 的同步几何查询 | 自绘区域的语义构建 API |
| 离屏 | 不导出 | 不导出 | OffscreenCanvas、导出、读回，先补 SDK |

A 是绘图闭环；B 才是常用屏幕 Canvas 的完整交付。证券应用在 A 后迁移路径，在 B 后迁移图内文字、图片和复用路径。每个版本只导出实际实现并验收过的方法。

### 4. 上下文的具体接口

**文件：`packages/solid-renderer/src/canvas/types.ts`，A/B 合并后的目标接口：**

```typescript
export type Matrix2D = Readonly<{ a: number; b: number; c: number;
  d: number; e: number; f: number }>;
export type FillRule = "nonzero" | "evenodd";
export type CanvasPaint = string | CanvasGradient;
export type CanvasComposite = "source-over" | "destination-over"
  | "destination-out" | "multiply" | "screen" | "lighter" | "xor";
export interface CanvasPath {
  closePath(): void;
  moveTo(x: number, y: number): void;
  lineTo(x: number, y: number): void;
  quadraticCurveTo(cx: number, cy: number, x: number, y: number): void;
  bezierCurveTo(c1x: number, c1y: number, c2x: number, c2y: number, x: number, y: number): void;
  rect(x: number, y: number, width: number, height: number): void;
  roundRect(x: number, y: number, width: number, height: number,
    radii?: number | readonly number[]): void;
  arc(x: number, y: number, radius: number, start: number, end: number,
    counterclockwise?: boolean): void;
  arcTo(x1: number, y1: number, x2: number, y2: number, radius: number): void;
  ellipse(x: number, y: number, rx: number, ry: number, rotation: number,
    start: number, end: number, counterclockwise?: boolean): void;
}
export interface CanvasRenderingContext2D extends CanvasPath {
  readonly canvas: Readonly<{ width: number; height: number }>;
  fillStyle: CanvasPaint;
  strokeStyle: CanvasPaint;
  lineWidth: number;
  lineCap: "butt" | "round" | "square";
  lineJoin: "miter" | "round" | "bevel";
  miterLimit: number;
  globalAlpha: number;
  globalCompositeOperation: CanvasComposite;
  lineDashOffset: number;
  font: string;
  textAlign: "left" | "right" | "center" | "start" | "end";
  textBaseline: "alphabetic" | "ideographic" | "top" | "middle" | "bottom";
  direction: "ltr" | "rtl" | "inherit";
  imageSmoothingEnabled: boolean;
  save(): void;
  restore(): void;
  beginPath(): void;
  translate(x: number, y: number): void;
  rotate(radians: number): void;
  scale(x: number, y: number): void;
  transform(a: number, b: number, c: number, d: number, e: number, f: number): void;
  setTransform(a: number, b: number, c: number, d: number, e: number, f: number): void;
  resetTransform(): void;
  getTransform(): Matrix2D;
  setLineDash(segments: readonly number[]): void;
  getLineDash(): number[];
  fill(rule?: FillRule): void;
  fill(path: Path2D, rule?: FillRule): void;
  stroke(path?: Path2D): void;
  clip(rule?: FillRule): void;
  clip(path: Path2D, rule?: FillRule): void;
  fillRect(x: number, y: number, width: number, height: number): void;
  strokeRect(x: number, y: number, width: number, height: number): void;
  clearRect(x: number, y: number, width: number, height: number): void;
  createLinearGradient(x0: number, y0: number, x1: number, y1: number): CanvasGradient;
  createRadialGradient(x0: number, y0: number, r0: number,
    x1: number, y1: number, r1: number): CanvasGradient;
  createConicGradient(angle: number, x: number, y: number): CanvasGradient;
  fillText(text: string, x: number, y: number, maxWidth?: number): void;
  measureText(text: string): TextMetrics;
  drawImage(source: string, dx: number, dy: number): void;
  drawImage(source: string, dx: number, dy: number, dw: number, dh: number): void;
  drawImage(source: string, sx: number, sy: number, sw: number, sh: number,
    dx: number, dy: number, dw: number, dh: number): void;
  isPointInPath(x: number, y: number, rule?: FillRule): boolean;
  isPointInPath(path: Path2D, x: number, y: number, rule?: FillRule): boolean;
  isPointInStroke(x: number, y: number): boolean;
  isPointInStroke(path: Path2D, x: number, y: number): boolean;
}
export interface CanvasGradient { addColorStop(offset: number, color: string): void }
export interface TextMetrics {
  readonly width: number;
  readonly fontBoundingBoxAscent: number;
  readonly fontBoundingBoxDescent: number;
}
// Path2D 为可构造类，并实现 CanvasPath。
// constructor(path?: Path2D | string); addPath(path: Path2D, transform?: Matrix2D): void;
```

所有坐标是场景点，角度为弧度。font 的 px 也表示场景点。pixelRatio 只用于像素对齐等作者计算；引擎只在原有根缩放处乘一次，作者不再按 DPR 手动放大 Canvas。

### 5. 状态、路径与坐标合同

以下规则必须有独立测试，不能只凭图表截图验收：

- 每次 draw 新建上下文：黑色填充/描边、线宽 1、butt、miter/10、alpha 1、source-over、空 dash、单位矩阵；字体 `10px Sans`，start、alphabetic，方向继承节点，图片平滑开启。
- save 保存画笔、变换、裁剪和文字状态；当前路径独立于该栈。空栈 restore 无操作；回调返回后自动平衡尚存的 save，不允许影响外部 picture。
- beginPath 清当前路径；fill/stroke/clip 不清路径。fill 隐式闭合开放子路径，但不修改随后 stroke 所见的路径。rect 方法不替换此前路径。
- 当前路径各段在创建时应用当时的 CTM；Path2D 保留自身局部坐标，使用时再应用 CTM。不能在 stroke 时把当前路径再整体变换一次。
- 对当前路径做描边时仍考虑当前线宽/CTM。Rust 准备层使用逆矩阵与描边轮廓处理非等比变换；路径几何和描边尺度分别验证。奇异矩阵不产生 NaN：既有路径填充仍按已保存几何处理，需要逆变换的退化描边不输出面积。
- clip 只收紧当前裁剪，恢复靠 restore。空路径与当前裁剪求交后区域为空，协议保留这条空裁剪。resetTransform 只重置 Canvas 内部 CTM，不能移除节点、祖先、picture 原点或 point_scale。
- 负宽高按形状与图像各自规则归一化；零面积图形无绘制。负半径抛出带 API 名的参数错误。
- 颜色与数值属性的无效赋值保留旧值；非有限绘图参数不生成命令。结构错误、非法半径、超出限额属于有类型的错误。Native 边界仍完整验证，不能仅相信 TS 检查。
- `setLineDash` 在读取数组元素前检查长度。奇数段重复一次，重复后最多 256 段。负数和非有限输入保持旧值，全零等同实线。有效数组计入录制预算，由保存的状态和描边操作共享。绘制和 `isPointInStroke` 都用曲线控制多边形的长度估算虚线切换次数，并在 kurbo 虚线迭代和描边时检查截止时间与段数限额。
- Path2D 与 CanvasGradient 可跨 draw 复用，但每次绘制操作截取当时版本。后续修改不会追溯改变已提交图像；非信号修改需要 invalidate 才重新录制。
- 渐变使用显式 Canvas 坐标，按绘制操作的 CTM 解析。绘图状态持有渐变引用，每条填充、描边或文字命令录制时取得独立的色标快照。双圆径向对应 SDK ConicalGradient，锥形对应 SweepGradient。色标稳定排序，空色标列表透明。

### 6. 录制、响应式与一帧

```text
输入 / onFrame 写信号
  → 原有 reactive 冲刷与 mutation
  → layout → driver → 必要时 layout → onLayout → 冲刷 → 最终 layout
  → Canvas paint 阶段：运行失效 draw、准备与原子提交
  → Tree::record → Engine::submit_layers → present
```

**Graph 的最小扩展：** 给 effect 节点增加内部执行阶段属性，普通 render/tracked effect 保持原队列；增加供嵌入层使用的 `create_paint_effect`、`run_paints`、`has_paints`。普通 `run_updates` 不执行 paint effect，has_updates 也不把等待 paint 的节点误判为普通工作。无需新增作者侧 Solid API 或改变公开 Kind 变体。

- 绑定 Canvas 时，在当前 owner 下建立一个 paint computation。该 computation 实际调用 draw，因此它读取的 signal/memo 都进入原有依赖图；不是靠普通 effect 提前执行 draw 来猜依赖。
- 首次尺寸确定、依赖失效、draw 函数变化、controller.invalidate、字体环境变化、尺寸变化时标脏。pixelRatio/textScale/reduceMotion/继承方向改变也使 draw 重新得到环境快照。
- 最终 layout 后每个有效、可绘制的 Canvas 最多录制一次。只有 paint 脏也必须进入这一阶段，不能以 ran_layout 为条件。
- 最终布局后先释放 Tree 借用，再进入拟新增的 rt::vm_paint turn。该入口使用 Host::with 的活动 Ctx 和既有 Graph/JsEval，不建立第二个 Context 或 JS 调度队列。
- Graph 中保存一个只捕获绑定身份的原生调用 thunk；真正的 draw/onDrawError 作为 Persistent 保存在 Host 的 Canvas 回调表中。调用前先复制必要引用并结束 References/Tree/arena/Recorder 的可变借用。
- measureText 与几何查询通过原生 context 在当前 turn 内读取 Rust 状态；查询不能重新获取同一 VM 锁，也不能触发同步盒子布局。
- paint 期间禁止写信号/树、创建 computation 或强制盒子测量/布局；在入口用明确 guard 报错，不靠无限重试。允许只读 Typography 查询及 recorder 内部操作。
- draw 成功、同步返回且准备成功后才安装新版本；空成功结果清空旧内容。异常、异步返回或校验失败均保留旧版本，版本号不推进。
- 一次 `draw` 失败时保留上一次提交，并在下次失效时重试。`onDrawError` 在下一帧的脚本阶段投递，本轮回调共用 128 ms 预算。超时记入对应 Canvas 节点的诊断，尚未投递的错误留到后续帧。未设置回调的错误直接进入诊断，owner 清理后丢弃其待投递错误。
- Context 仅本次回调有效，之后调用其方法抛 `ContextClosed`。不提供可在事件处理器任意写入旧上下文的入口。Path2D/Gradient 本身不持有已关闭 context。
- 无尺寸、display:none 或脱树的节点保留失效标记但不执行；重新可绘制时补一次。首版不承诺屏幕外祖先裁剪的精确可见性剔除；应用继续通过虚拟列表卸载或暂停外部数据驱动控制负载。
- Hidden/Paused 时不消耗 pending paint，也不靠它持续要帧；Resumed 后对真正失效的输入补绘。只有 GPU 缓存丢失时直接重放 CPU CanvasList。
- owner 清理和 vm_forget_ids 的节点回收路径都移除 paint computation、Persistent 回调与绑定身份，关闭 DrawLease，向 controller prop 交 null；迟到 controller/提交由 generation 拒绝。类 finalizer 只释放 Rust 数据与预算，不调用 JS 或 GPU，卸载不依赖 GC 及时发生。
- invalidate 请求恰好下一帧，并在帧内去重；静态 Canvas 不建立常驻 onFrame。连续动画用已有 onFrame 更新输入并再要下一帧。

### 7. Rust 原生类、录制与原子提交

CanvasRenderingContext2D、Path2D、CanvasGradient 与 CanvasController 通过 `src/ui/player/vm/canvas.rs` 的 rquickjs class 暴露。类方法调用 `uicore::canvas::Recorder`、路径和画笔的具体 Rust 方法；TS 仅保留 Props、类型导出、组件样式/owner 绑定。

**数据流：** `JS draw → rquickjs class 方法 → Rust Recorder → finish/prepare → OwnedRec（MutationOp::SetCanvas）→ Tree → protocol::CanvasList → picture → engine`。

- 每次绘制创建有 epoch 的 DrawLease，状态为 Open/Closed。Context 指向该 lease，方法入口验证绑定身份、NodeId generation、epoch 与 active paint。成功、异常、中断、清理都通过 Rust RAII 关闭 lease；回调返回后留在 JS 全局的旧 context 也不能再写入。
- Recorder 拥有当前路径、CTM、状态栈和调用形成的 Rust 记录；Path2D/Gradient 拥有可版本化 Rust 数据。绘制操作保存当时的不可变快照，后改 builder 不影响已安装内容。跨方法共享 Recorder 只为同次录制，不使用 Arc<Mutex> 或异步运行时。
- 原生方法先完整转换参数，再借用 Recorder；可能执行 JS getter/coercion 的步骤不能跨越 RefMut。返回 JS 对象前也先释放可变借用，防止分配/GC/finalizer 重入。
- 画笔属性 getter 若需要返回原来的 Gradient JS 对象，用带 Ctx 生命周期的 Class/Value 字段并实现 Trace；纯 Rust 几何/颜色字段不追踪。不要在原生类之间用 Persistent 互相固定成 GC 无法回收的根。
- 跨 turn 的 draw/onDrawError 使用与现有回调一致的 Persistent；构造器与 native function 的声明/导出进入 vm/modules.rs。类方法优先从当前 Ctx 的 userdata 取得存活 Host，不为每个图元复制裸 Host 指针。
- 类构造器有独立的原生导出表，并合并到 scene_names 的声明集合；不把 Path2D 混进只按普通函数包装的 CORE_NAMES 分派。Canvas 组件传入稳定的 draw 包装闭包，实际调用 props.draw 时仍处于 paint reader 下，因此 getter/信号依赖继续由 Graph 跟踪。
- finish 只取得 Rust 拥有数据；prepare 校验、规范化路径/描边/裁剪和资源引用，形成不可变 CanvasList。随后构造带类型化 canvas payload 的 OwnedRec，一次走已有 mutation::mutate/apply 路径。getter、draw、prepare 失败时均不安装部分结果。
- abi.yml 只需登记 SetCanvas 等真正的场景操作常量；现有 build.rs 可以生成对应 Rust/JS/TS 编号。每个 lineTo 不进入场景 mutation 表，也不新增 Canvas 操作的 JS 数组协议、编码器、解码器或 C 布局。
- 同步 measureText/isPointInPath/isPointInStroke 直接读取 Rust 当前状态和共享几何实现，无需把当前路径快照从 JS 再发一遍；它们不 flush 尚未完成的 Canvas，不触发盒子布局。
- 原生类的具体解析、预算和颜色规则与无 VM 的 Rust Recorder 共用。Node mock 只记录组件调用与生命周期，不能变成第二套路径/画笔实现。

**文件：`crates/protocol/src/canvas.rs`，核心类型形状：**

```rust
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasIsolation {
    Shared,
    Isolated,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanvasList {
    pub ops: Arc<[CanvasOp]>,
    pub images: Arc<[CanvasImageRef]>,
    pub isolation: CanvasIsolation,
}
```

CanvasOp 为封闭 enum：Save、Restore、ClipPath、DrawPath、DrawText、DrawImage、ClearRect。路径数据只含 Move/Line/Quad/Cubic/Close 和 fill rule；操作分别持有局部几何、解析后的画笔与必要矩阵。协议不含 rquickjs Class/Persistent/Ctx、NodeId、JS 回调、Impeller 句柄或股票类型，CanvasList 不允许再嵌套 CanvasList。

外层 `Command::Canvas { x, y, width, height, list }` 放在现有 DrawList；x/y 是当前 picture 或扁平列表的原点。translate_command 只变更 x/y。引擎播放时以当前原点和缩放为基底，每个操作的矩阵相对这个基底解释。

### 8. 几何与颜色依赖

Canvas 几何与颜色解析各使用一项纯计算依赖。

| 依赖 | 作用 | 边界 |
|---|---|---|
| kurbo 0.13.1 | 曲线/椭圆、SVG path 解析、仿射、dash、stroke 轮廓与 winding | MIT OR Apache-2.0；不包含光栅器、字体或窗口后端 |
| csscolorparser 0.9.0 | Canvas 颜色字符串解析与 sRGB RGBA 规范化 | MIT OR Apache-2.0；只启用必要功能，不扩展样式系统语义 |

版本与许可依据为 [kurbo 发布记录](https://github.com/linebender/kurbo/releases/tag/v0.13.1)、[kurbo 项目](https://github.com/linebender/kurbo)、[csscolorparser 包元数据](https://docs.rs/crate/csscolorparser/0.9.0/source/Cargo.toml)。kurbo 已有 [dash](https://docs.rs/kurbo/0.13.1/kurbo/fn.dash.html)、[stroke](https://docs.rs/kurbo/0.13.1/kurbo/fn.stroke.html) 和 [winding](https://docs.rs/kurbo/0.13.1/kurbo/trait.Shape.html#tymethod.winding)，不自行实现另一套曲线描边算法。

几何使用 f64 计算，在检查有限性/范围后转到协议 f32。曲线近似误差按最终物理尺度控制，建议阈值 0.25 物理像素；缓存键包含会改变精度的有效尺度。极端放大重新准备几何，但仅 GPU 缓存丢失不要求重新执行 JS。颜色字符串按有界缓存解析，支持常见 hex/rgb/rgba/hsl/命名颜色；无 DOM 的 currentColor、var 与主题 token 不在颜色语法里，作者显式读取主题后传入。

### 9. 清除、混合与合成隔离

- 普通 source-over 的 Canvas 用独立 picture 缓存与 save/clip，无需为每个 Canvas 分配长期纹理；picture 与离屏纹理不是同一概念。
- 出现 clearRect 或任何受目标影响的混合时，准备层把 isolation 置 Isolated。播放该 Canvas 时将内容绘入裁到 content box 的透明 SaveLayer，结束后 source-over 合成回父级。
- View 背景/边框在 Canvas 内容之外绘制；clearRect 和 destination-out 不能清掉它们、兄弟节点、父 picture 或另一个 Canvas。
- clearRect 记录当前位置、范围和裁剪，忽略填充色/globalAlpha/普通混合状态，输出 Clear；不能用透明矩形或“删掉前面的命令”替代。后续绘制仍按顺序叠加。
- SaveLayer 的触发从实际命令推导；不向作者提供容易误用的 opaque/disableIsolation 开关。空 Canvas 不创建层。
- 首批复合模式逐项映射 SDK 并做像素验收。copy、source-in、destination-in 等涉及透明源范围的模式不能只换一个 blend enum；需证明整个 clip 区域的语义后再开放。
- 内部 save/clip 与外部效果栈严格隔离；无论错误还是正常结束，都恢复进入 Canvas 前的 builder save count。

### 10. 文字与图片

**文字：**

- fillText 使用同一份 Typography，先按无换行、无省略的单行文本排版，再按 textAlign 和 textBaseline 定锚点。maxWidth 为正时只压缩横向，不让段落换行或截断；measureText 返回未压缩宽度。
- 空串 fillText 不输出命令；空串 measureText 的 width 为 0，字体度量从同字体的基准度量取得，不把空串交给会返回 BadScene 的现有段落创建路径。文本中的换行按单行 Canvas 规则规范为空白。
- font 首批语法限定为可选 normal/italic、可选权重、正数 px 字号、已注册 family 列表；不提供 CSS 字体加载、rem/em/%、行高和 oblique 角度。缺省注册名 Sans/SansCJK 继续有效。
- 斜体必须进入 TextStyle、测量键和绘制键；不得只改 paint 导致宽度与实际文字不一致。
- 用 SDK 的 alphabetic/ideographic baseline、首行 ascent/descent 支撑有限基线与返回的 fontBoundingBox 字段，和字形墨迹边界分开。top/middle/bottom 的锚点按受支持字体的对应度量验收；未能核对的 hanging/ink metrics 不声明，更不能返回假 0。
- 字号是显式场景点，不自动乘系统 textScale；作者可用 size.textScale 调整，测量与绘制遵守相同选择。专用 canvas_metrics 查询避免调用会自动乘 text_scale 的 Tree::measure_text。
- 字体注册/替换时，相关 Canvas 文字缓存失效；复用 picture 时继续保留段落引用。排版 begin/end 范围覆盖布局与 Canvas 测量，纯绘图帧也要正确 touch。

**图片：**

- source 沿用 Image 的资源标识与加载管线，不接受 DOM 图片元素。三参形式按资源自然像素尺寸映射为场景点；显式目标宽高优先。九参 source rect 使用源像素，destination rect 使用场景点。
- 负尺寸归一化、源裁剪越界的对应目标裁切、零面积与非有限参数分别测试。翻转通过 CTM；负尺寸不隐式镜像。
- 图片未就绪时保留资源引用并跳过该图元，准备失败与解码失败分别进入诊断。就绪后只使依赖它的 Canvas/picture 失效，不触发盒子重新测量。
- Tree 保存 source 到 Canvas 节点的使用索引，并在替换/销毁时更新；不能只增加 ImageCensus 总数而无法定向处理 ImageReady。自然尺寸在播放时解析，外盒仍由 style 决定。
- CanvasList 图片必须进入 image_census、加载请求、PictureCache 复用时的 touch 和释放流程；不能只在首次播放扫描。
- imageSmoothingEnabled 在 SDK 的 Linear/NearestNeighbor 间选择；不声明 SDK 未提供的质量档位。文字和图像画笔继续服从 Canvas alpha/clip/混合。

### 11. 输入、语义、错误与预算

- Canvas 复用 ViewEvents 和 uicore 的手势认领，默认没有监听，不拦截父滚动。图元不自动成为命中节点，也不提供第二棵事件树。
- controller.toLocal 使用上一帧的可交互几何，逆去祖先滚动/变换及节点 padding；不可逆或无几何返回 null。它不逆去 context 的 CTM；图形查询再使用相应 CTM，避免转换两次。
- isPointInPath/isPointInStroke 在有效 draw 内读取当前路径/状态，走准备层同一几何。事件处理器用 controller.toLocal 与应用数据做选点，不能保留已经关闭的 context 查询。
- Canvas 自身通过 role、accessibilityLabel/hint 提供整体语义；图内独立操作以真实 View/Pressable 覆盖。首版不假装图形或 fillText 自动具有语义，也不接管 Field 的编辑与焦点。

**文件：`packages/solid-renderer/src/canvas/types.ts`，错误接口：**

```typescript
export type CanvasErrorCode = "InvalidArgument" | "LimitExceeded"
  | "ContextClosed" | "DrawSideEffect" | "AsyncDraw"
  | "TextUnavailable" | "Unsupported" | "Interrupted";
export interface CanvasError extends Error {
  readonly code: CanvasErrorCode;
  readonly api?: string;
}
```

原生 Recorder 的建议初值为每版 65,536 条记录、262,144 个输入路径段、256 层 save、256 个 stops/gradient、4 MiB 录制数据；准备后的结果也独立限额。新增会话级 Canvas 原生内存预算，初值 32 MiB，覆盖活跃 Recorder、可变 Path2D/Gradient、准备中与保留的 CPU 列表；共享不可变数据按实际分配计一次。VM 已有 16 MiB memory limit 不能自动约束这些 Rust 分配，必须单独计数、预留、退款并进入诊断。旧版仍被显示列表引用时不能提前退款。

Canvas 回调复用现有非 eval turn 的 128 ms 上限，并标注 canvas/NodeId；嵌套 measureText 和原生方法继承当前 deadline，不在每个方法入口重新起表。VM interrupt handler 能中断 JS 循环；长时间运行的 Rust dash/stroke/解析/准备循环还需主动检查单调时钟 deadline 和计数上限，不能假定 VM 会在 native 方法中抢占。输入长度在分配前检查、乘法用 checked 运算，失败通过既有异常机制退出并由 DrawLease guard 清理。

绘制事务只覆盖命令内容和绘图资源，不回滚作者偷偷执行的外部 I/O；draw 的纯函数约束不能被描述成通用 JS 事务。提交失败保留旧图与其资源引用，新图临时引用立即释放。

### 12. GPU 生命周期与缓存

- draw/error 的 Persistent、paint binding 与原生类由 JavaScript UI 会话持有；Recorder 和 CPU 几何使用 Rust 所有权，安装后的 CanvasList 归 Tree；Impeller Path/Paint/Paragraph/DisplayList 归 engine。所有操作仍在渲染线程，Engine 与 Typography 的 !Send 约束保持。
- Canvas 默认成为独立 picture owner；与 ListDriver 的 owner 来源独立计数/标记，关闭或清空 Canvas 不得撤掉列表行需要的隔离。普通 View 的 children 能力不因有无 Canvas 动态改变，公共 Canvas 组件自行禁止 children。
- 同尺寸、相同绘图版本重复 present/滚动不运行 JS、不重新准备几何、不重建 Canvas picture。节点变换与 opacity 尽量继续走 LayerTree。
- Hidden/Paused/MemoryWarning 的 Engine::trim 可以释放 GPU picture/纹理/段落；CPU CanvasList 仍可用于恢复。Resumed 重建 GPU 显示列表，不以每个 Canvas 重跑 draw 作为恢复前提。
- 纹理和段落在旧 root/display list 释放后再回收。新图提交或 GPU 安装失败继续沿用现有回滚机制；Canvas 的局部命令不得漏计资源。
- CPU 已准备版本与 GPU 已安装版本分开记录。GPU 安装失败保留已安装的旧图与资源；CPU 可以保留待安装的新版本，后续重试不要求把 Tree 回滚或重跑 JS。
- Engine::shutdown 保持现有顺序：在 Host::with 内执行 reactive::release_all 后，关闭全部 Canvas lease/绑定并释放 Canvas Persistent，再清空其余 References/定时器、移除 userdata，最后释放 Context/Runtime。关闭过程中拒绝新建 Canvas 绑定；正常退出和启动失败共用清理路径，Host/Inner 活过最终 class finalizer。
- 首版不做命令 diff、局部损伤重绘或跨线程 Canvas。高频十字线和静态网格可用少量独立 Canvas 分层，保持同一套 API。

### 13. 离屏与像素 API 的后续设计入口

目标形状可为固定尺寸 OffscreenCanvas + getContext("2d") + 异步导出，但它与屏幕回调的跨帧替换语义不同：离屏上下文需要明确累计、清除与快照时刻，不能直接借用当前 draw Context。

开放前必须具备：Metal/Vulkan 对等的离屏目标、渲染与读回同步、像素格式/预乘 alpha/色彩转换、后台与设备丢失、取消与超时、输出尺寸/并发/内存上限，以及 PNG 编码的可复用实现。不得用窗口截图、临时隐藏窗口、平台 Canvas 或第二套光栅器兜底。getImageData/putImageData、toBlob、Canvas 作为 drawImage 源、worker 转移均留在该独立交付，不在首版类型里挂空壳。

## Phase 0: 固定接口与最小消费

**文件：`docs/canvas-design.md`、`docs/components-design.md`、`docs/refactor-plan.md`、`docs/stock-app-design.md`**
- 统一 Canvas 公共接口、实施阶段与验收范围，在组件清单和规划中建立入口。

**文件：`apps/gallery-solid/src/pages/canvas.tsx`（新增）、`src/app.tsx`**
- 新增独立 `/canvas` 能力页，先使用可编译的现有 View/Text 搭结构；随后逐阶段加入绘图，不把所有测试都埋进股票业务。
- 固定直线/曲线、变换、裁剪、清除与兄弟背景、空图和错误保留旧图场景。

## Phase 1: 通用录制、准备与播放

**文件：`Cargo.toml`、`Cargo.lock`、`crates/uicore/Cargo.toml`**
- 按已核对版本增加 kurbo 与 csscolorparser，检查依赖树与许可。

**文件：`crates/protocol/src/canvas.rs`（新增）、`src/draw_list.rs`、`src/lib.rs`**
- 定义 CanvasList、局部操作、路径与资源引用，并增加 Command::Canvas；数据不带 SDK 类型。

**文件：`crates/uicore/src/canvas/mod.rs`、`record.rs`、`prepare.rs`、`path.rs`、`paint.rs`（均新增）、`src/lib.rs`、`src/error.rs`**
- 实现有上限的录制输入、状态解释、路径/描边/裁剪、颜色和事务校验；以结构化 CanvasError 接入原有 Error。

**文件：`crates/uicore/src/tree/node.rs`、`mod.rs`、`layout.rs`、`store.rs`、`record/mod.rs`**
- 增加可选绘图 payload、owner 来源、局部录制、命令平移、资源计数和回收；纯 set_canvas 不标布局脏。

**文件：`crates/engine/src/player/canvas.rs`（新增）、`player/mod.rs`、`player/ops.rs`、`player/paint.rs`、`pictures.rs`、`textures.rs`**
- 实现 A 范围的 Path/clip/clear/合成隔离和资源遍历；每条失败路径恢复 builder save count。
- 此时先由 Rust 测试直接提交 CanvasList，证明不依赖 TS 也能正确绘制。

## Phase 2: 原生上下文、绘制阶段与公共 Canvas

**文件：`crates/reactive/src/lib.rs`、`crates/javascript/src/ui/player/reactive.rs`、`src/ui/player/mod.rs`**
- 加入同图的 paint 阶段、owner 清理、只读 guard；最终布局后先释放 Tree 借用，再运行绘制 turn，计入下一帧需求。

**文件：`crates/javascript/src/ui/player/vm/canvas.rs`（新增）、`src/ui/player/vm/mod.rs`、`src/ui/player/vm/bindings.rs`、`src/ui/player/vm/modules.rs`、`src/ui/player/vm/turns.rs`**
- 使用现有 class/methods/Trace/JsLifetime 能力实现 Context 和 Controller；接入 Host 活动 Ctx、Persistent、NodeId/绑定 generation、DrawLease 与预算。
- 在 scene 中声明并导出原生类/构造器和绑定入口；vm_paint 截取局部异常，复用现有运行时中断与错误归因。
- 在 shutdown 与 vm_forget_ids 中释放绑定；finalizer 只回收纯 Rust 状态。

**文件：`crates/javascript/src/ui/abi.yml`、`src/ui/player/mutation.rs`**
- 登记一次最终 SetCanvas，OwnedRec 保存类型化 CanvasList payload；直接复用现有 mutation 路径，不把绘图逐方法写入 ABI。
- build.rs 保持现有常量生成职责，不增加 JS 录制格式或通用对象解码器。

**文件：`packages/solid-renderer/src/canvas/types.ts`（新增）、`src/scene.ts`、`src/mutation.gen.ts`（生成）、`runtime/index.ts`、`packages/reactive/src/modules.d.ts`、`packages/solid-renderer/testing/scene.ts`、`testing/native.ts`**
- 只声明 A 的原生接口并接好 scene 的运行时导出。Node 替身记录调用/绑定，不实现生产 recorder。

**文件：`packages/solid-ui/src/Canvas.tsx`（新增）、`src/props.ts`、`src/index.tsx`、`apps/gallery-solid/src/pages/canvas.tsx`**
- 用 bindBox/eventsOf 包装 View，向 native 绑定 draw/error，交付 controller。
- 阶段出口：真实 QuickJS 中信号只重绘对应 Canvas，方法直接写 Rust Recorder，颜色不触发布局，尺寸变化只安装最终尺寸，静态页面停止要帧。

## Phase 3: 原生复用路径、渐变与图形查询

**文件：`crates/javascript/src/ui/player/vm/canvas.rs`、`src/ui/player/vm/modules.rs`、`crates/uicore/src/canvas/path.rs`、`paint.rs`、`prepare.rs`、`packages/solid-renderer/src/canvas/types.ts`、`runtime/index.ts`、`packages/reactive/src/modules.d.ts`**
- 实现 Path2D/Gradient 的原生 class、SVG path 与不可变版本快照；补齐属性对象身份、Trace 和无环的所有权。
- 精确填充/描边查询直接读 native path，与实际绘制共用几何；原生类构造器按真实 scene 导出验收。

**文件：`crates/uicore/src/tree/hit.rs`、`crates/javascript/src/ui/player/vm/canvas.rs`、`packages/solid-renderer/src/scene.ts`、`packages/solid-ui/src/Canvas.tsx`**
- 基于现有反变换提供 controller.toLocal，覆盖滚动、旋转、非等比缩放和 padding；该基础方法随阶段 2 一起交付，此阶段完成组合验收。

## Phase 4: 同源文字与图片

**文件：`crates/protocol/src/draw_list.rs`、`canvas.rs`、`crates/uicore/src/shaper.rs`、`src/canvas/text.rs`（新增）、`prepare.rs`、`crates/embedder/common/src/shaper.rs`**
- 接入 FontSlant、Canvas 专用度量和基线数据。Shaper 可增加有默认“不支持”结果的方法，避免强迫外部实现返回伪度量；MonoShaper 与 ImpellerShaper 各自给出真实合同内结果。

**文件：`crates/engine/src/typography/key.rs`、`shape.rs`、`paint_store.rs`、`mod.rs`、`src/player/canvas.rs`、`pictures.rs`、`textures.rs`**
- 补齐缓存身份、画笔/基线、文字 retain；支持图片 natural size/source crop/sampling，图片加载完成使相关 picture 失效。

**文件：`crates/uicore/src/tree/store.rs`、`crates/javascript/src/ui/player/vm/canvas.rs`、`packages/solid-renderer/src/canvas/types.ts`、`packages/solid-ui/src/index.tsx`**
- 贯通同步 measureText、drawImage 资源引用与 B 版本完整类型。
- 阶段出口：测量与绘制字号相同、字体更新失效、异步图片到达后显示、复用 picture 不丢纹理，Metal/Vulkan 一致。

## Phase 5: 原生验收、诊断与应用迁移

**文件：`crates/javascript/src/ui/component_tests.rs`、`crates/reactive/src/lib.rs` 的测试、`crates/uicore/src/canvas/tests.rs`（新增）、`crates/engine/src/player/canvas.rs` 的测试**
- 覆盖下面的正确性矩阵，使用真实会话验证 class/getter、paint lane、错误局部化、无副作用和资源生命周期；补 GC/Trace、泄漏、被 JS 保留的关闭 context、启动失败/重复退出、JS 超时与 native 准备超时。测试始终运行当前解释器，几何用 MonoShaper 可在无 SDK 环境验收，不引入 QuickJS 缺失分支。

**文件：`packages/solid-ui/src/canvas.test.ts`、`packages/solid-ui/type-tests/canvas.tsx`、`crates/javascript/tests/fixtures/ui/canvas.tsx`（新增）**
- Node/类型测试覆盖 Props、绑定调用、controller 清理与导出；真实 QuickJS fixture 验证原生 class、上下文失效和不可变快照，不在 TS 测试中复制 Recorder 算法。

**文件：`crates/embedder/common/src/diagnostics/hub.rs`、`diagnostics/module.rs`、`diagnostics/inspect.rs`、`src/timing.rs`、`src/host.rs`、`crates/javascript/src/session/mod.rs`、`crates/javascript/src/ui/player/mod.rs`、`packages/devtools/src/store.ts`、`src/pages/performance.tsx`**
- 增加录制/准备时间、命令与路径段数量、重绘/复用计数；沿用环形缓冲。接入时先定位字段消费点，生成端与消费端同步更新，不只加孤立指标。

**文件：`apps/gallery-solid/src/widgets/sparkline.tsx`、`src/pages/canvas.tsx`、`apps/gallery-solid/quotes.stocks.test.ts`**
- `/stocks` 及个股详情使用 Canvas；`/canvas` 保留独立的能力页。

**文件：`README.md`、`docs/architecture.md`、`docs/components-design.md`、`docs/refactor-plan.md`、`docs/canvas-design.md`**
- 落地后更新现状，收掉已完成目标。构建生成文件后运行一致性检查、受影响 Cargo/TS 检查与仓库完整检查。

### 正确性与性能出口

| 类别 | 必测场景 |
|---|---|
| 路径 | current path 与 Path2D 的不同时机变换、开放子路径、零长度、负半径、evenodd、非等比描边、dash 与 miter |
| 状态 | save 不回滚路径、restore 下溢、嵌套 clip、resetTransform 不重置外层、奇异矩阵后 restore |
| 合成 | clearRect 中间挖洞、半透明叠加、目标混合、兄弟/背景保持、Canvas 嵌在 opacity/clip/scroll 内 |
| 调度 | 同帧多次失效合并、纯 paint 帧、resize+onLayout、反应依赖切换、失败后恢复、非法写状态 |
| 资源 | 图元替换/销毁、Path2D/gradient 后改、迟到图片、字体换代、trim/resume、SDK 提交失败回滚 |
| 文字图片 | CJK/RTL、空串/空格、斜体、基线/maxWidth、DPR/textScale、裁切越界、近邻采样 |
| 输入语义 | 祖先滚动与旋转后的 toLocal、零尺度、图表横拖与纵滚、键盘操作的真实覆盖节点 |

像素验收使用 Impeller 原生画面，按既有每通道 2 的容差规则建立小尺寸确定金样；曲线近似参数固定后才钉图。浏览器只能作为支持子集的语义对照，不能替代 Metal/Vulkan 验收；不执行网页截图或故事视频脚本。

性能分三组：一个静态 Canvas；可见列表中多个 24 点小图；一个 1000 点分时/200 根 K 线加独立十字线。记录同设备同模式暖态 p50/p95：draw JS、原生方法调用数/时间、prepare、record、submit、present 分开，附操作/段数、VM 内存与独立 Canvas 原生内存。另以相同数据测直接 Rust Recorder，明确逐方法 JS→Rust 成本；Path2D 复用先消除重复工作，批量点输入仅在实测需要时另定接口，不先假定原生绑定没有开销。静态重复 present 的 draw/prepare/picture rebuild 必须为 0；修改十字线不重录价格图；纯绘图不增加布局轮次。用设备刷新周期确定整帧预算，不能先承诺一个脱离硬件的固定 Canvas 耗时。

## 架构说明

- 只新增通用绘图数据与组件；命名使用 Canvas、Path2D 和 CanvasList，不增加证券专用原生命令。
- protocol 的新变体与 TextStyle 新字段需要迁移；Graph 的新方法是内部能力扩展，普通 Solid API 行为保持。
- 已知封闭操作用 enum；Shaper 是已有替换边界，沿用它。无需 CanvasBackend trait、对象安全的异步绘图接口或跨线程共享上下文。
- 新增依赖只负责纯几何与解析；rquickjs class 沿用既有依赖能力。第三方类、Ctx/Persistent 不进入产品协议；TS 声明与 native 导出保持一份合同。
- `crates/impeller-sys` 提供 Impeller 绑定。Canvas 绘制由共用实现处理，macOS 和 Android 宿主提供窗口、输入与资源。
- 屏幕 Canvas 不需要长期 CPU bitmap；几何准备、命令复制与偶发 SaveLayer 有成本，首版即计量并设上限。

## 文件变更汇总

以下为实施目标；类型和方法只在对应阶段验收后导出。

- `Cargo.toml` -- 登记发布版本依赖。
- `Cargo.lock` -- 由 Cargo 更新锁定依赖树。
- `apps/gallery-solid/src/app.tsx` -- 增加 Canvas 能力页路由。
- `apps/gallery-solid/src/pages/canvas.tsx` -- 独立原生绘图场景。
- `apps/gallery-solid/src/widgets/sparkline.tsx` -- 首批 Canvas 消费。
- `apps/gallery-solid/quotes.stocks.test.ts` -- 验证真实应用消费。
- `crates/engine/src/pictures.rs` -- 复用/回滚与嵌套资源。
- `crates/engine/src/player/canvas.rs` -- 局部命令与隔离合成播放。
- `crates/engine/src/player/mod.rs` -- 登记模块与测试。
- `crates/engine/src/player/ops.rs` -- 分派 Canvas 命令。
- `crates/engine/src/player/paint.rs` -- 复用路径与画笔构建。
- `crates/engine/src/textures.rs` -- 扫描 Canvas 图片引用。
- `crates/engine/src/typography/key.rs` -- 完整字体/画笔身份。
- `crates/engine/src/typography/mod.rs` -- Canvas 文字缓存生命周期。
- `crates/engine/src/typography/paint_store.rs` -- Canvas 文字 paint 槽。
- `crates/engine/src/typography/shape.rs` -- 同源基线与度量。
- `crates/protocol/src/canvas.rs` -- 通用局部命令与资源类型。
- `crates/protocol/src/draw_list.rs` -- Canvas 命令与 FontSlant。
- `crates/protocol/src/lib.rs` -- 导出协议类型。
- `crates/javascript/src/ui/abi.yml` -- 登记最终原子 SetCanvas 操作。
- `crates/javascript/src/ui/component_tests.rs` -- 原生会话与 class/GC/预算回归。
- `crates/javascript/tests/fixtures/ui/canvas.tsx` -- 真实原生组件 fixture。
- `crates/javascript/src/ui/player/vm/bindings.rs` -- 注册原生 Canvas 绑定入口。
- `crates/javascript/src/ui/player/vm/mod.rs` -- 持久回调、预算和 shutdown 清理。
- `crates/javascript/src/ui/player/vm/modules.rs` -- 声明与导出 Canvas 原生类。
- `crates/javascript/src/ui/player/vm/turns.rs` -- 原生 paint turn 与节点回收清理。
- `crates/javascript/src/ui/player/vm/canvas.rs` -- 原生 Context/Path2D/Gradient/Controller、DrawLease 与提交。
- `crates/javascript/src/ui/player/mod.rs` -- 最终布局后的绘制阶段。
- `crates/javascript/src/ui/player/mutation.rs` -- Typed CanvasList payload 与原子变更。
- `crates/javascript/src/ui/player/reactive.rs` -- 同图 paint computation。
- `crates/reactive/src/lib.rs` -- 分阶段执行与只读 guard。
- `crates/javascript/src/session/mod.rs` -- 传递 Canvas 帧成本。
- `crates/embedder/common/src/diagnostics/hub.rs` -- Canvas 指标。
- `crates/embedder/common/src/diagnostics/inspect.rs` -- 导出 Canvas 帧成本。
- `crates/embedder/common/src/diagnostics/module.rs` -- 序列化 devtools 帧字段。
- `crates/embedder/common/src/shaper.rs` -- Canvas 专用度量。
- `crates/embedder/common/src/host.rs` -- 接收会话的 Canvas 成本。
- `crates/embedder/common/src/timing.rs` -- 构造对应帧样本。
- `crates/uicore/Cargo.toml` -- 引用几何和颜色依赖。
- `crates/uicore/src/canvas/mod.rs` -- 录制准备入口与错误类型。
- `crates/uicore/src/canvas/paint.rs` -- 颜色与渐变规范化。
- `crates/uicore/src/canvas/path.rs` -- 路径、描边与几何查询。
- `crates/uicore/src/canvas/prepare.rs` -- 状态解释和不可变结果。
- `crates/uicore/src/canvas/record.rs` -- Rust Recorder、状态栈与原生内存预算。
- `crates/uicore/src/canvas/tests.rs` -- 几何与事务验证。
- `crates/uicore/src/canvas/text.rs` -- 字体语法和文本准备。
- `crates/uicore/src/error.rs` -- 保留结构化 Canvas 错误。
- `crates/uicore/src/lib.rs` -- 导出通用接口。
- `crates/uicore/src/shaper.rs` -- 可选 Canvas 度量合同。
- `crates/uicore/src/tree/hit.rs` -- 共享局部坐标转换。
- `crates/uicore/src/tree/layout.rs` -- owner 来源与尺寸失效。
- `crates/uicore/src/tree/mod.rs` -- Canvas 节点操作登记。
- `crates/uicore/src/tree/node.rs` -- 可选绘图 payload。
- `crates/uicore/src/tree/record/mod.rs` -- 局部录制、平移与资源 touch。
- `crates/uicore/src/tree/store.rs` -- 绘图生命周期与图片计数。
- `packages/devtools/src/pages/performance.tsx` -- 展示 Canvas 成本与复用指标。
- `packages/devtools/src/store.ts` -- 接收对应帧字段。
- `packages/reactive/src/modules.d.ts` -- 声明 scene 原生类导出。
- `packages/solid-renderer/runtime/index.ts` -- 重导出原生值与类型。
- `packages/solid-renderer/src/canvas/types.ts` -- 公共类型与错误。
- `packages/solid-renderer/src/mutation.gen.ts` -- 按唯一描述生成。
- `packages/solid-renderer/src/scene.ts` -- 绑定与查询声明。
- `packages/solid-renderer/testing/native.ts` -- scene 原生类的 Node 导出替身。
- `packages/solid-renderer/testing/scene.ts` -- 组件绑定的 mock paint 调用记录。
- `packages/solid-ui/src/Canvas.tsx` -- Canvas 组件。
- `packages/solid-ui/src/canvas.test.ts` -- Props 与生命周期。
- `packages/solid-ui/src/index.tsx` -- 作者 API 导出。
- `packages/solid-ui/src/props.ts` -- CanvasProps 与 controller。
- `packages/solid-ui/type-tests/canvas.tsx` -- 原生 API 正反类型用例。
- `README.md` -- 落地后更新能力说明。
- `docs/architecture.md` -- 落地后更新绘制阶段与命令。
- `docs/canvas-design.md` -- Canvas 设计与验收。
- `docs/components-design.md` -- 组件清单与能力边界。
- `docs/refactor-plan.md` -- Canvas 实施入口。
- `docs/stock-app-design.md` -- 统一消费 Canvas。
