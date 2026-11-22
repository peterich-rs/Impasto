# 统一对外组件与属性接口

> 文中的实施阶段与代码引用反映设计或审计时的仓库结构。当前 crate 职责、源码入口和验证命令见 [架构](architecture.md)。

本文是 `@impasto/solid-ui` 的对外组件合同。布局、内容、交互、输入、滚动和浮层都从该包导入；`style` / `class` / `classList` 是展示配置，`disabled`、`variant`、`controller` 等领域属性保留。底层仍是 View、Stack、Scroll、Text、Image 五种节点。样式值与来源沿用 `style-design.md`。主题作用域的原生索引仍按该文推进，不在组件里另写一套求值。

## 破坏性变更与迁移

1. 应用的组件统一从 `@impasto/solid-ui` 导入；`@impasto/solid-renderer` 保留编译、挂载、节点与领域命令接口。Text/Image 沿用 renderer 中的唯一实现，由 solid-ui 重导出，已有 renderer 导入不需要立即删除。
2. Row 的 `gap/justify/fill` 改为 `style.gap/justifyContent` 与明确的尺寸声明。主轴分配用 `flex`，填满视口用百分比尺寸，逐处核对原有几何。
3. Button 的 `wide` 改为 `style={{ width: "100%", flexShrink: 1 }}`；`variant/size/disabled/loading` 保留为控件领域属性。Row 中均分剩余空间也可显式声明 `flex: 1`。
4. Image 的 `w/h/mode` 改为 `style.width/height/objectFit`；已知资源尺寸使用独立 `intrinsicSize`，换图不能改写作者尺寸。
5. 普通组件的 `ref` 指向 `style` 对应的根节点，卸载时回调 null。Dialog 等浮层指向可见面板，使用挂载占位的返回值不作为面板 ref。
6. 测试重置函数移到 `@impasto/solid-ui/testing`；组件与公开 Props 类型全部显式导出。测试入口不进入应用 bundle。

## 可行性评估

现有五种节点、原生 children 插入、StyleProps、bindStyle、交互状态、编辑器与列表驱动足以承载这些组件。Button 已证明单 View 可以组合布局、背景、手势与语义，Text 已使用独立样式来源。新增布局组件只需要声明预设，不增加布局算法。需要配套完成 Image 资源更新与尺寸拆分、复合组件的样式目标、主题方向同步和 ref 生命周期。有条件可行：按阶段完成真实 QuickJS 验收，不能仅凭 Node 替身宣布接口可用。

## 当前接口与消费点

- `packages/solid-ui/src/index.tsx` 统一导出布局、内容、交互、输入、滚动和浮层组件，以及公共 Props、NodeRef、UIChildren 和样式类型；测试重置从 `testing/index.ts` 导出。
- `packages/solid-renderer/runtime/index.ts` 提供挂载、scene、bindStyle、Text/Image 和控制器绑定。根 Stack 的填充轨道使用 component 来源，作者来源仍受 StyleSurface 约束。
- `packages/solid-renderer/src/jsx.ts` 定义四种原生标签、可空 NodeRef、ViewEvents 和 UIChildren。`runtime/slot.ts` 的每行 View 由原生 insert 归一化任意行内容。
- `packages/solid-ui/src/{View,Row,Column,Stack,Grid,Icon}.tsx` 复用 `element.tsx` 的来源、ref 与语义绑定。Row 继承环境方向；Stack 保持一格重叠策略。
- `packages/solid-renderer/runtime/{text,image}.ts` 导出 TextProps/ImageProps。Image 的 source 与 intrinsicSize 更新不改作者 width/height。
- `packages/solid-ui/src/{Button,Pressable}.tsx` 共用 `pressable.ts`，组合内部交互和作者事件，disabled 同步原生语义与 focusable。
- `packages/solid-ui/src/field/shell.tsx` 区分外框、控制区、编辑文字、标签与错误目标。`styled-text.ts` 将组件字体默认值放在 component 来源，完整传递作者 StyleInput。
- `packages/solid-ui/src/{ScrollArea,VirtualList,Recycler,RefreshControl}.tsx` 保留视口、内容、行 owner 与驱动控制参数的归属。ListContent 的作者样式限制由原生完整校验。
- `packages/solid-ui/src/{Dialog,Sheet,Menu,Toast,Tooltip}.tsx` 使用 `target.ts` 保存响应式样式快照。浮层关闭、重开和 owner 销毁时配对撤销 panel ref；Dialog/Sheet 在退出动画完成后清空。
- `packages/solid-ui/src/{theme,theme-bind,use-theme}.ts` 将默认主题色板、文字角色、尺寸 token 与 locale 方向同步到原生 Environment。主题作用域与按依赖建立的节点索引仍在 `docs/refactor-plan.md`。
- `packages/solid-router/src/{index.ts,view.tsx}` 保留 RouterView 与 StackTransition；它们的填充轨道使用 component 来源。
- `packages/solid-ui/type-tests/components.tsx` 由 tsc 检查正反例。`crates/javascript/src/ui/component_tests.rs` 编译并执行 `tests/fixtures/ui/component*.tsx`，检查真实 DrawList、输入、语义、主题更新和浮层生命周期。

## 设计

### 1. 关键决策

1. **保留 Button、Text、Row、Column 等作者组件。** 底层 Kind 继续只有 View/Stack/Scroll/Text/Image；组件名称不要求对应新的 Rust 节点类型，避免组件数量扩张布局引擎。
2. **solid-ui 是应用的组件入口。** 布局组件在 solid-ui 中实现薄模板，Text/Image 重导出 renderer 的实现；不在两个包各写一份组件。小写原生标签继续用于控件实现和底层接入，不作为应用文档的主要写法。
3. **展示配置统一进入 StyleProps。** 移除 Row.gap/fill/justify、Button.wide 等平行入口；保留 variant、size、disabled、axis、items、controller 等领域属性。工具类与对象样式提交同一套原生声明，不在 TS 中解析 CSS 或计算坐标。
4. **复合组件按可见部分声明样式目标。** 每个目标接受完整 StyleProps，使用 labelProps、contentProps、scrimProps 等明确名称；不增加一套 contentStyle/contentClass/contentClassList 平行词汇。
5. **默认值使用 component 来源。** class、inline、imperative、animation 按现有优先级覆盖；不能把默认样式放在 inline 数组前面。内部结构与控制器状态由组件拥有，不用作者样式声明占位高度、滚动 offset 或退出动画进度。
6. **事件和语义由组件组合。** Button 与 Pressable 共享无节点的交互逻辑，Button 不再包一层 Pressable 节点。内部禁用、焦点与键盘处理必须执行，再派发允许的用户回调；用户回调不能覆盖掉内置行为。
7. **先规范已有组件，再按消费补齐新控件。** 不通过空导出或兼容壳预占 Checkbox、Radio、Select、Slider 等名字。没有领域状态、键盘与无障碍验收的组件不进入公共入口。

### 2. 对外组件清单

以下是目标清单。“已有”仅表示存在实现，不代表已符合本方案全部属性合同。

| 分组 | 名称 | 当前状态 | 目标职责与根节点 |
|---|---|---|---|
| 基础布局 | View | 新增组件封装 | 通用 View，默认纵向、min 0、shrink 0、无隐式 max |
| 基础布局 | Row | 已有，统一属性 | View 的横向预设；方向继承父级 |
| 基础布局 | Column | 新增 | View 的纵向预设；保留明确的布局语义 |
| 基础布局 | Stack | 新增组件封装 | 原生 Stack，一格 grid 重叠 |
| 基础布局 | Grid | 新增组件封装 | View 的 grid 预设，轨道与 placement 进入 style |
| 内容 | Text | 已有，补 Props/ref | 原生 Text 叶节点，支持继承、runs 与文字样式 |
| 内容 | Image | renderer 已有，改造后重导出 | 原生 Image 叶节点，分离资源与作者尺寸 |
| 内容 | Icon | 已有，统一属性 | 带镜像能力的图形容器；当前不是 SVG 或图标库加载器 |
| 内容 | Canvas | 已落地，见 [Canvas 2D 设计](canvas-design.md) | 普通 View 的绘图内容；尺寸走 style，draw 在最终布局后录制 |
| 交互 | Pressable | 新增 | 单 View 的无皮肤交互入口，提供键盘、禁用与焦点合同 |
| 控件 | Button | 已有 | 单个交互 View 加文字/图标，variant/size 产生默认样式 |
| 控件 | Switch | 已有 | 受控或本地布尔状态，样式目标为交互根、track 与 label |
| 输入 | Field、TextArea | 已有 | TextEditor 的语言模板；保留控制器、选区、IME 与校验 |
| 导航 | NavigationBar、Tabs | 已有 | 标题和操作区、页签组合；业务路由仍由 router 提供 |
| 滚动 | ScrollArea | 已有 | viewport、content 和 ScrollController 的组合 |
| 列表 | VirtualList | 已有 | ListDriver 驱动窗口、测量与行 owner；固定纵向 |
| 列表 | Recycler | 已有 | 条目复用，不独立提供滚动视口 |
| 刷新 | RefreshControl | 已有 | overscroll 阈值与刷新状态，不再实现滚动物理 |
| 浮层 | Dialog、Sheet | 已有 | 面板、遮罩、焦点作用域与退出生命周期 |
| 浮层 | Menu、Popover | 已有 | 基于 anchor 的浮层；菜单条目语义需要单独验证 |
| 浮层 | Tooltip、Toast | 已有 | 非模态提示，不窃取焦点 |
| 页面设施 | KeyboardAvoidingView、OverlayRoot、ErrorPage | 已有 | 键盘避让、浮层宿主、可恢复错误页面 |
| 路由包 | RouterView、StackTransition | 已有，留在 solid-router | 路由条目与转场 owner 生命周期 |

不增加 Box/Flex/HStack/VStack 等同义出口。Padding、Sized、Align、Clip、Opacity、Transform 通过 View/Stack 的 style 表达；Spacer 使用 View 的 flex，不新增一类渲染节点。需要不同测量、裁剪或生命周期的内部包装继续保留，不能为了“单节点”破坏结构。

### 3. 公共属性与类型

基础类型定义在 `packages/solid-renderer/src/jsx.ts` 并从 renderer 重导出，组件 Props 在 `packages/solid-ui/src/props.ts` 引用它们。下例描述目标接口，完整事件沿用现有原生事件集合。

```typescript
import type { Node as NodeHandle } from "./tree.ts";

export type NodeRef = (node: NodeHandle | null) => void;
export type UIChildren = NodeHandle | string | number | boolean |
  null | undefined | readonly UIChildren[] | (() => UIChildren);
```

`packages/solid-ui/src/props.ts` 的组件合同：

```typescript
import type { NodeRef, UIChildren, ViewEvents, StyleProps } from "@impasto/solid-renderer";
import type { Role } from "./semantics.ts";
import type { ButtonSize, ButtonVariant } from "./theme.ts";

export interface ElementProps extends StyleProps {
  ref?: NodeRef;
}
export interface ViewProps extends ElementProps, ViewEvents {
  children?: UIChildren;
  focusable?: boolean;
  role?: Role;
  accessibilityLabel?: string;
  accessibilityHint?: string;
}
export type RowProps = ViewProps;
export type ColumnProps = ViewProps;
export type GridProps = ViewProps;
export type StackProps = ViewProps;
export interface PressableProps extends ViewProps {
  disabled?: boolean;
}

export type ButtonProps = Omit<ViewProps,
  "children" | "role" | "focusable" | "onClick"> & {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  loading?: boolean;
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: UIChildren;
  labelProps?: StyleProps;
};

export interface ImageProps extends ElementProps {
  source?: string;
  intrinsicSize?: { width: number; height: number };
  alt?: string;
  required?: boolean;
}
```

- 每个公共组件导出独立的 `XxxProps`，泛型列表导出 `VirtualListProps<T>` 与 `RecyclerProps<T>`。叶组件不机械继承 View 的 children/输入法等无关属性。
- `ViewEvents` 从现有 JSX 事件类型提取；公共 View 的 role/label 由组件调用已有语义命令，不要求原生小写标签接受相同语义对象。Role 沿用现有身份，不自行补造未接入的原生角色。
- ref 指向实际样式节点，不增加为了 ref 存在的盒子。条件显示与浮层重新挂载时回调新节点，销毁时回调 null；普通样式变化不替换节点。ref 回调变更先清理旧回调。
- children 归一化与插入继续走原生 scene。Text 的 children 是文字内容，runs 为独立富文本内容；两者的优先级保留现有 runs 优先合同。
- Row/Column/Grid 的方向与 display 是可覆盖的默认预设。Stack 的轨道策略、Scroll 的裁剪与驱动占位属于结构约束，不能被普通样式覆盖；限制同时覆盖 style 与 class，未知或禁止字段必须在原生完整校验时明确报错，display:none 允许。
- 原生文字方向、布局方向从根或主题作用域继承。移除每个 Row 强制写 locale 方向的做法；主题桥接未就绪前不能让 RTL 回归悄悄消失，先用显式根方向验收。

### 4. style、子目标与 ref 的归属

子目标 props 默认只暴露 StyleProps，确需获取该节点时增加明确的 `inputRef/viewportRef/contentRef`，不开放任意内部事件与 children 透传。

| 组件 | style/ref 目标 | 其他公开样式目标与领域接口 |
|---|---|---|
| View/Row/Column/Grid/Stack/Icon/Pressable | 自身原生根 | children；事件与语义各自绑定 |
| Text/Image | 自身叶节点 | runs 或 source；没有子布局容器 |
| Button | 交互根 | labelProps；图标自行设置样式 |
| Switch | 交互根 | trackProps、labelProps；checked/onChange |
| Field/TextArea | 包含标签与错误的外框 | controlProps、textProps、labelProps、errorProps；FieldController 操作编辑目标，inputRef 指向可聚焦控制区 |
| ScrollArea | 滚动 viewport | contentProps、contentRef；axis/controller/physics/onOffset 保留领域含义，ref 与 viewportRef 指向同一 viewport |
| VirtualList | 列表外框 | viewportProps、contentProps；render 回调拥有每行样式，driver 独占 spacer、extent 和行定位 |
| Recycler | 条目容器 | 每行由 render 回调定义；不接受虚构的 viewportProps |
| NavigationBar/Tabs | 排列根 | titleProps、leading/trailing 内容；Tabs 的 tabProps/panelProps |
| Dialog/Sheet/Menu/Popover | 可见面板 | scrimProps、contentProps、titleProps；关闭后的占位不接收作者样式 |
| Tooltip/Toast | 提示面板 | textProps；anchor、open 与关闭回调 |
| OverlayRoot/KeyboardAvoidingView/ErrorPage | 实际布局根 | 避让/恢复等行为参数保留；ErrorPage 提供 textProps/actionProps |
| RefreshControl | 指示器根 | progress/busy 等刷新状态；不开放驱动内容偏移的样式目标 |

VirtualList.contentProps 的样式域只开放绘制和继承文字字段，布局由 driver 拥有；class 同样按解析后的 Key 校验，不能绕过限制。需要间隔或内边距时由驱动明确支持，并纳入测量/extent 合同，不能简单透传 gap 后继续沿用旧行高估计。

目标原生约束使用封闭的 `StyleSurface`：`General`、`Stack`、`ScrollViewport`、`ListContent`。Stack/Scroll 按节点种类确定；ListContent 由组件创建内部内容节点时登记。新增内部领域命令 `scene.setStyleSurface(node, "list-content")` 由 QuickJS 具名绑定调用 Tree。登记时先校验已接受的声明；失败不得改变约束。约束按来源分类：preset/component 保存组件拥有的结构，class/inline/imperative/animation 均受作者样式域限制。该约束是组件正确性合同，不是脚本安全沙箱。违反约束返回包含节点、来源和 Key 的 StyleError，不能先写入 Taffy 再撤销。

ScrollArea.contentMin 当前还参与控制器估计，保持领域参数，单独审计其最终归属；作者最小尺寸通过 contentProps.style 声明，不能在 TS 中读取并解释 CSS 长度来驱动滚动。

Button 的默认文字颜色、字号和字重放在根的 component 来源，label Text 自然继承；labelProps 只覆盖标签。焦点环的默认外观也进入独立 component 来源，允许作者 focus 工具类覆盖，焦点行为本身仍由组件拥有。多目标的每个 bindStyle 都持有独立来源，卸载不清除其他写入者。

### 5. 事件、内容与生命周期

- Button 固定按钮语义；disabled/loading 阻止激活并更新 focusable/语义，不靠颜色或 opacity 判断能否点击。Pressable 默认 Generic 且可聚焦，显式处理 Enter/Space；disabled 时取消按压并退出焦点。普通 View 的 onClick 不自动变成 Button。
- Pressable 与 Button 共用 `pressable.ts` 的行为绑定；调用现有 uicore 手势、焦点与键盘事件，不在 TS 新造手势识别器。
- 样式隐藏通过已有 conceal 流程取消按压、派发 Blur 并清理交互。先释放 Tree 借用再回调 JS。
- Dialog/Sheet 的公开 ref 活到退出动画完成，随后清空。owner、焦点恢复与可见样式节点同时撤销，不能在设置 open=false 时提前销毁离场内容。
- 列表回收保留 row NodeId 与 createSlot 的 owner；样式更新不重建行。摘下、复用、最终销毁分别沿用现有驱动合同。
- Image 更新 source 只更新资源与可选固有尺寸。intrinsicSize 缺省走资源的固有测量；未知资源保持现有占位约定，待资源就绪重新测量，不能合成作者 width/height。

### 6. 使用方式

目标调用示例，新增组件与属性需要按下面阶段落地后才能使用。

```tsx
import { Button, Column, Grid, Image, Row, ScrollArea, Text } from "@impasto/solid-ui";

<Column class="h-full w-full" style={{ minHeight: 0 }}>
  <Row class="items-center justify-between gap-3 p-4">
    <Text style={{ fontSize: 20 }}>自选</Text>
    <Button label="添加" variant="primary" onClick={add}
      class="rounded-lg" labelProps={{ class: "font-semibold" }} />
  </Row>
  <Grid style={{ gridTemplateColumns: [{ fr: 1 }, { fr: 1 }], gap: 12 }}>
    {cards()}
  </Grid>
  <Image source="asset:avatar.png" alt="用户头像"
    intrinsicSize={{ width: 128, height: 128 }}
    style={{ width: 32, height: 32, objectFit: "cover", borderRadius: 16 }} />
</Column>;

<ScrollArea class="flex-1" contentProps={{ class: "gap-2 p-4" }}>
  {rows()}
</ScrollArea>;
```

主题中的颜色、间距和字体通过统一环境作用于类名和对象 token；组件只将 variant/size 等状态转换成 component 默认声明。主题作用域的原生同步按 style-design 推进，不用每个组件自己的 CSS 求值替代。

## 阶段 0：固定公共接口基线

**文件：`packages/solid-ui/src/index.tsx`、`packages/solid-renderer/runtime/index.ts`、两个 package.json。**

- 清点当前组件、类型、工具和测试导出，区分稳定入口与测试入口。暂不修改应用运行行为。
- 为导出与类型声明建立快照清单；记录 Button 单 View、Field focus target、列表 owner 与浮层 panel 的原生关系。

交付：现有应用和测试继续运行，清单中每个现有名称均能定位到唯一实现。

## 阶段 1：统一类型、来源与 ref

**文件：`packages/solid-renderer/src/{jsx,style,tree}.ts`、`runtime/{text,style,index}.ts`；`packages/solid-ui/src/{props,element}.ts`（新增）。**

- 导出事件、ref 与 Props 的可复用定义；Text 导出 TextProps 并补 ref。
- element 只封装已有 bindStyle、ref 与属性绑定，不重写 createElement/insert/children 归一化，不创建第二个渲染器。
- 每个目标的 defaults/class/inline 使用独立来源，ref 挂载/清理与 owner 绑定；动态 ref 切换也有明确撤销。

交付：原生测试证明样式、ref、条件挂载和来源清理不替换无关节点。

## 阶段 2：补齐布局组件与方向继承

**文件：`packages/solid-ui/src/{View,Column,Stack,Grid}.tsx`（新增）、`Row.tsx`、`Icon.tsx`、`index.tsx`。**

- 增加四种布局封装，Row/Column 仅写方向预设，Grid 使用统一轨道样式；全部接收公共 Props。
- Row 移除 gap/fill/justify 快捷属性；Icon 的 mirror 保留为领域行为，用户 style/class 可覆盖外观。
- 小写原生标签保留给实现层，不增加一组 Rust Kind。

**文件：`packages/solid-ui/src/{NavigationBar,Tabs}.tsx`、`apps/gallery-solid/src/{app.tsx,pages/watchlist.tsx}`。**

- 迁移现有 Row 快捷属性，应用页面使用 Column/Row，并显式声明视口与剩余空间填充。
- 根显式声明方向，验证嵌套覆盖及 reparent；完成主题原生同步后移除重复的组件级 locale 反转。

交付：Row 与等价 View、Column 与默认 View 的几何一致，RTL 显式覆盖生效。

## 阶段 3：统一交互组件

**文件：`packages/solid-ui/src/{Pressable.tsx,pressable.ts}`（新增）、`{Button,Switch,Icon}.tsx`、`focus.ts`、`semantics.ts`、`index.tsx`。**

- 共享激活/禁用/键盘绑定；Pressable 和 Button 各自创建一个根，不互相叠加节点。
- Button 导出 ButtonProps，迁移 wide，将默认文字样式移到根，新增 labelProps。Switch 明确 track/label 目标。
- 焦点环外观使用低优先级组件来源；行为与用户事件正确组合。

交付：原生鼠标、键盘、禁用、loading、焦点隐藏和独立清理回归通过；自定义颜色/字号可以覆盖默认值。

## 阶段 4：规范复合组件

**文件：`packages/solid-ui/src/{Field,TextArea}.tsx`、`field/shell.tsx`、`field/controller.ts`。**

- 导出根/控制区/文字/提示的 Props；FieldController 继续操作实际编辑节点，不把外框 ref 当成编辑目标。

**文件：`packages/solid-ui/src/{ScrollArea,VirtualList,Recycler,RefreshControl}.tsx`、`ScrollController.ts`。**

- 按样式目标表新增 Props，保留 offset、physics、估计与复用的领域接口；校验 VirtualList 内容样式限制。

**文件：`packages/solid-ui/src/{Dialog,Sheet,Menu,Toast,Tooltip}.tsx`、`overlay/OverlayRoot.tsx`、`{NavigationBar,Tabs,KeyboardAvoidingView,errors}.tsx`。**

- 明确面板/遮罩/提示/内容目标与 ref；保留焦点恢复和退出 owner。导出各自 XxxProps。

**文件：`crates/uicore/src/tree/style.rs`、`style/{mod,property}.rs`、`crates/javascript/src/ui/{abi.yml,player/vm/bindings.rs,player/mutation.rs}`、`packages/solid-renderer/src/scene.ts`。**

- 对 Stack/Scroll 与列表内部目标记录允许的样式域，完整候选解析后按 Key 校验；class 和 style 一视同仁。拒绝必须保留来源、几何与驱动状态，不能静默忽略属性。

交付：样式不破坏滚动范围、列表 extent、IME 或浮层生命周期；错误输入原子拒绝。

## 阶段 5：完成资源、主题与出口迁移

**文件：`packages/solid-renderer/runtime/image.ts`、`src/{scene,jsx}.ts`、`runtime/index.ts`、`packages/solid-ui/src/index.tsx`。**

- Image 接入 StyleProps/ref 与 intrinsicSize；补 solid-ui 的 Image/样式类型重导出。

**文件：`crates/uicore/src/tree/{layout,store}.rs`、`crates/javascript/src/ui/{abi.yml,player/vm/bindings.rs,player/mutation.rs}`。**

- 图片资源命令不写入作者尺寸；派生解码尺寸仍通过原有资源就绪与测量链进入 Taffy，不扩展图片缓存项目范围。

**文件：`packages/solid-ui/src/{theme,theme-bind,use-theme}.ts`、`packages/solid-renderer/runtime/index.ts`。**

- 按 style-design 把根方向与主题环境同步给原生；同一套 token 同时作用于类名和对象样式，主题作用域释放与 owner 一致。

**文件：`packages/solid-ui/{package.json,src/index.tsx,testing/index.ts}`（测试入口新增）。**

- 测试重置函数从正式根入口移除；只发布已具备实现与验收的组件及 Props。包根、编译入口和分发复制规则保持一致。

交付：换图不改变显式尺寸，所有正式导入均有唯一实现，测试入口不进入应用 bundle。

## 阶段 6：验证与文档

**文件：`packages/solid-ui/src/{props.test.ts,components.test.ts}`（新增）、`boundary.test.ts`、`focus.test.ts`、`theme.test.ts`、`field/keyboard.test.ts`。**

- 检查全部公共导出及 Props，子目标与 ref 的归属，事件组合与独立来源清理。类型测试拒绝被删除的快捷属性与错误子目标。

**文件：`crates/javascript/tests/fixtures/ui/components.tsx`（新增）、`crates/javascript/src/ui/tests.rs`、`crates/uicore/src/style_contract_test.rs`。**

- 实际编译并运行所有目标组件；检查节点身份、原子拒绝、RTL、换图、IME、滚动、列表和离场生命周期，不能用 mock 的无布局实现代替。

**文件：`packages/solid-router/src/router.test.ts`、`apps/gallery-solid/quotes.esm.test.ts`。**

- 路由退出 owner、回收和应用 ESM 继续通过；另在 macOS/Android 验证截图和输入。原生标签仍可工作不代表新的大写组件已验收。

**文件：`packages/solid-ui/README.md`（新增）、`packages/solid-renderer/README.md`、`docs/{architecture,refactor-plan,style-design,components-design}.md`、`AGENTS.md`。**

- 用真实可用的入口重写示例；删除被替换的属性表，标明剩余工作，不能把目标清单直接写成当前能力。

检查顺序：类型/导出 → JS 包测试与 gallery 构建 → 真实 QuickJS fixture → Rust 受影响测试与 workspace → 生成检查/rustfmt/Clippy → 原生截图和交互。已有未通过的全仓 lint 与缺失平台验收分别记录。

## 架构说明

- 本方案改变作者属性与出口，需要框架包、QuickJS 和应用 bundle 配套升级；不维持同一属性的多套解析入口。
- 没有新增 Rust 节点 Kind、布局算法、C 产品头文件或异步渲染运行时；Taffy、测量、命中和绘制仍在原有层。
- 不新增组件图标库依赖。Icon 当前只有图形容器与镜像职责，SVG/矢量图标资源需要独立能力设计。
- ref 和子目标不承诺额外节点；节点增删必须有布局、内容作用域、裁剪或生命周期原因。原有 NodeId 代数句柄保持不变。
- 样式主题作用域依赖 style-design 的原生实现；不能引入只在 TS 存在、对原生 class 无效的第二套主题求值。
- OverlayRoot 作为布局宿主接受根样式；useOverlay、控制器和主题/焦点函数不是可视组件，不机械追加 StyleProps。
- RouterView/StackTransition 继续由 solid-router 导出，响应式控制流继续由 reactive 提供；能力模块继续独立发布。
- Checkbox、RadioGroup、Select、Slider 等属于后续产品覆盖，不在本次接口规范中预先导出。

## 文件变更汇总

- `AGENTS.md` -- 实施后同步作者入口与属性约定。
- `apps/gallery-solid/quotes.esm.test.ts` -- 新组件入口的应用 ESM 回归。
- `apps/gallery-solid/src/app.tsx` -- 页面根与方向的明确声明。
- `apps/gallery-solid/src/pages/watchlist.tsx` -- Column/Row 与滚动组合消费。
- `crates/javascript/src/ui/abi.yml` -- 图片资源更新所需领域命令。
- `crates/javascript/src/ui/player/vm/bindings.rs` -- 图片资源与固有尺寸输入。
- `crates/javascript/src/ui/tests.rs` -- 注册真实组件 fixture。
- `crates/javascript/src/ui/player/mutation.rs` -- 样式目标校验与资源命令分发。
- `crates/javascript/tests/fixtures/ui/components.tsx` -- 公共组件原生合同。
- `crates/uicore/src/style/mod.rs` -- StyleSurface 与结构化错误。
- `crates/uicore/src/style/property.rs` -- 目标样式域的 Key 元数据。
- `crates/uicore/src/style_contract_test.rs` -- 结构约束与拒绝回滚。
- `crates/uicore/src/tree/layout.rs` -- 图片资源更新不覆盖作者尺寸。
- `crates/uicore/src/tree/store.rs` -- 图片固有尺寸与资源状态。
- `crates/uicore/src/tree/style.rs` -- 来源提交时验证目标约束。
- `docs/architecture.md` -- 实施后描述当前组件与底层节点。
- `docs/components-design.md` -- 本设计与交付合同。
- `docs/refactor-plan.md` -- 保留未完成的组件接口工作。
- `docs/style-design.md` -- 作者示例及组件接口引用。
- `packages/solid-renderer/README.md` -- 编译接口与组件入口的职责。
- `packages/solid-renderer/runtime/image.ts` -- StyleProps、ref 与独立资源更新。
- `packages/solid-renderer/runtime/index.ts` -- 公共类型和根环境接入。
- `packages/solid-renderer/runtime/style.ts` -- 来源与 ref 生命周期配合。
- `packages/solid-renderer/runtime/text.ts` -- TextProps 与叶节点 ref。
- `packages/solid-renderer/src/jsx.ts` -- 可复用事件/ref 类型。
- `packages/solid-renderer/src/scene.ts` -- 图片资源更新接口。
- `packages/solid-renderer/src/style.ts` -- 统一样式类型重导出。
- `packages/solid-renderer/src/tree.ts` -- 公共节点与 children 类型合同。
- `packages/solid-router/src/router.test.ts` -- 复合组件迁移后的退出生命周期。
- `packages/solid-ui/README.md` -- 作者组件手册。
- `packages/solid-ui/package.json` -- 测试入口与类型/导出检查。
- `packages/solid-ui/src/Button.tsx` -- ButtonProps、labelProps 与默认文字继承。
- `packages/solid-ui/src/Column.tsx` -- 新增纵向布局封装。
- `packages/solid-ui/src/Dialog.tsx` -- 面板/遮罩样式及 ref。
- `packages/solid-ui/src/Field.tsx` -- 公开表单 Props。
- `packages/solid-ui/src/Grid.tsx` -- 新增 Grid 样式预设。
- `packages/solid-ui/src/Icon.tsx` -- 公共根属性与 mirror。
- `packages/solid-ui/src/KeyboardAvoidingView.tsx` -- 根样式与避让领域属性。
- `packages/solid-ui/src/Menu.tsx` -- Menu/Popover 面板目标。
- `packages/solid-ui/src/NavigationBar.tsx` -- 标题目标与 Row 快捷属性迁移。
- `packages/solid-ui/src/Pressable.tsx` -- 新增单 View 交互组件。
- `packages/solid-ui/src/Recycler.tsx` -- 容器属性与泛型 Props。
- `packages/solid-ui/src/RefreshControl.tsx` -- 指示器属性与刷新状态。
- `packages/solid-ui/src/Row.tsx` -- 方向预设、样式入口与继承。
- `packages/solid-ui/src/ScrollArea.tsx` -- viewport/content 的 Props。
- `packages/solid-ui/src/ScrollController.ts` -- 作者尺寸与估计控制的边界。
- `packages/solid-ui/src/Sheet.tsx` -- 面板样式与退出 ref。
- `packages/solid-ui/src/Stack.tsx` -- 新增重叠布局封装。
- `packages/solid-ui/src/Switch.tsx` -- 根/track/label 的 Props。
- `packages/solid-ui/src/Tabs.tsx` -- tab/panel 目标与 Row 用法。
- `packages/solid-ui/src/TextArea.tsx` -- 共用 Field 的样式目标。
- `packages/solid-ui/src/Toast.tsx` -- 面板与文字属性。
- `packages/solid-ui/src/Tooltip.tsx` -- 提示面板与 ref。
- `packages/solid-ui/src/View.tsx` -- 新增公共基础容器。
- `packages/solid-ui/src/VirtualList.tsx` -- 列表样式域与驱动所有权。
- `packages/solid-ui/src/boundary.test.ts` -- 根节点与子目标回归。
- `packages/solid-ui/src/components.test.ts` -- 新增导出与组件合同测试。
- `packages/solid-ui/src/element.ts` -- 共用样式、属性和 ref 绑定。
- `packages/solid-ui/src/errors.tsx` -- ErrorPage 的样式目标。
- `packages/solid-ui/src/field/controller.ts` -- 实际编辑目标的控制器合同。
- `packages/solid-ui/src/field/keyboard.test.ts` -- 编辑与键盘行为回归。
- `packages/solid-ui/src/field/shell.tsx` -- 外框、控制区、文字与提示属性。
- `packages/solid-ui/src/focus.test.ts` -- 焦点环覆盖与清理。
- `packages/solid-ui/src/focus.ts` -- 焦点外观默认来源。
- `packages/solid-ui/src/index.tsx` -- 唯一组件实现的公共出口。
- `packages/solid-ui/src/overlay/OverlayRoot.tsx` -- 宿主根属性与面板边界。
- `packages/solid-ui/src/pressable.ts` -- 不创建节点的交互行为绑定。
- `packages/solid-ui/src/props.test.ts` -- 新增类型与移除属性检查。
- `packages/solid-ui/src/props.ts` -- 公共 Props 合同。
- `packages/solid-ui/src/semantics.ts` -- 沿用并公开现有语义角色。
- `packages/solid-ui/src/theme-bind.ts` -- 主题与根方向原生同步。
- `packages/solid-ui/src/theme.test.ts` -- 主题与方向继承。
- `packages/solid-ui/src/theme.ts` -- 主题令牌与正式/测试入口区分。
- `packages/solid-ui/src/use-theme.ts` -- 主题作用域生命周期。
- `packages/solid-ui/testing/index.ts` -- 新增测试专用出口。
