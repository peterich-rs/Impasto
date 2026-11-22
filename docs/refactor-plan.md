# 还没做的

已落地的行为以代码和 `docs/architecture.md` 为准。这里只留还没做的。

## 渲染后端与平台宿主

Android 的框架能力、阶段依赖与发布验收见 [Impasto Android 框架能力建设方案](design/android-framework-plan.md)。

iOS 的 UIKit 宿主、Metal、原生混排和应用分发见 [iOS Embedder 实现方案](design/ios-embedder-plan.md)。

宿主目录见 [平台宿主](platform-hosts.md)。Windows 与 Linux 的适配器各自接入窗口、表面、vsync、输入法和无障碍，先验证 Vulkan 交换链、首帧和窗口尺寸变化。后端注入接口见 [渲染后端](rendering-design.md)。

## 样式与布局

设计与剩余验收见 [基于 Taffy 0.15.1 重构样式对象与工具类](style-design.md)。后续工作：

- 主题作用域与按依赖建立节点索引，避免环境更新扫描全部样式节点。
- 真实文字固有尺寸、字体与 runs 的缓存键、CJK 换行及基线的跨宿主验收。
- 图片资源更新与固有尺寸拆分；图片作者尺寸完全迁入 style。
- RTL 横向滚动从 inline-start 起算，物理输入在滚动核心转换一次。
- 清理仍被原生命令测试消费的旧构造与逐字段 setter ABI；产品 TSX 已使用 view、stack、scroll、image 和统一样式。
- 样式解析、来源更新与 Taffy 写入诊断，固定负载的多轮性能记录，以及 Android 原生截图与交互验收。

## 渲染管线

- 在会话阶段记录各阶段脏节点数量、层访问数量及资源缓存命中率，与已有阶段区间一起分析。Android trace 的采集入口见 [Android 帧性能](platform-hosts.md#android-帧性能)。
- 扩展可独立布局的盒子处理，覆盖百分比约束和更多父级布局模式。继续测量文本探测在频繁变宽与字体回退下的成本。
- 复用已录制的合成子树层结构。用虚拟列表滚动和 Canvas 重绘验证裁剪、层顺序、重挂载和 GPU trim 后恢复。
- 根据各负载的提交帧与 SurfaceFlinger 呈现数据，继续降低响应式写入、布局及 Vulkan 绘制的开销。

## 对外组件

对外组件、公共 Props、样式目标和 ref 见 [统一对外组件与属性接口](components-design.md)。主题作用域的原生索引仍在样式一节。

## Canvas

屏幕 Canvas 已落地，见 [为 Impasto 增加 Canvas 2D](canvas-design.md)。还没做的：

- OffscreenCanvas、像素读回与导出
- pattern、阴影、滤镜、strokeText
- 路径布尔运算
- copy、source-in、destination-in

## 平台通道

macOS 已实现权限五法。还差：

- Android `Bridge` 的 `permission_status`、`request_permissions`、`open_app_settings`
- 有宿主实现后再加 `@impasto/camera`

`FileSystem` 与 `AsyncStorage` 的方法就是各包 `@impasto/<pkg>/host` 上的导出。路径拒绝 `..`、绝对路径和反斜杠。带 schema 的存储库以后只依赖这两个包，不进 `solid-ui`。

## JS 运行时与标准库

设计约束见 [Impasto JS 运行时与标准库重构设计](design/js-runtime/runtime-and-standard-library.md)。平台覆盖、性能样本与待验证项见 [JS 运行时验收](design/js-runtime/runtime-and-standard-library-validation.md)。

## Gallery

`/stocks`、`/stock/:symbol`、模拟订单与现有路由的分工见 [证券模拟盘](stock-app-design.md)。`/canvas` 是独立的绘图能力页。

每个框架包在 gallery 里至少有一个真实消费点。Quotes 现有路由作回归。能力页另开，不挤进模拟盘：

| 页面 | 消费 |
|---|---|
| `/controls` | Menu、Switch、Tabs、Icon |
| `/text` | Field 掩码、TextArea、KeyboardAvoidingView |
| `/overlays` | Dialog、Sheet、Toast、Tooltip |
| `/data` | createFetch、VirtualList、Recycler |
| `/persist` | AsyncStorage、FileSystem、createForm |
| `/device` | clipboard、linking、app-state、permissions |
| `/i18n` | `packages/i18n` |

像素金样跟着画面走，容差每通道 2。画面不变则不重钉。

## 暂缓

- 外接 devtools 协议服务。触发条件是第二个真实应用。应用内面板已经在 `packages/devtools`。
- Picture 字节。只在 Rust 里编解码一张 picture 的局部 `DrawList`，不包含 `LayerTree`。不定 opcode，不提交金样。
- 跨 picture 选区。触发条件是阅读序，以及两端点之间的节点集合。
- 导航往返之后，`picture_writes` 只计仍在树上的页面 picture。
- 把 `submit_layers` 挪到单独的 gpu 线程。`Engine` 仍和会话一起在渲染线程上。

## 必须守住

- `reduceMotion` 把运动写成终值，并且仍然发出运动结束。只跳过 `tick_motions` 会把 overscroll 和页面弹簧留在半程。
- `solid-router` 的退出条目保留到退场运动结束。新导航接管转场时销毁尚在退场的条目，完成回调只推进自己启动的阶段。
- 包使用 `onBack(priority, fn)`。直接调用 `core.onBack` 会排在渲染器那一次登记前面。
