# 还没做的

已落地的行为以代码和 `docs/architecture.md` 为准。这里只留还没做的。

## 平台通道

macOS 已实现权限五法。还差：

- Android `Bridge` 的 `permission_status`、`request_permissions`、`open_app_settings`
- 有宿主实现后再加 `@impasto/camera`

`FileSystem` 与 `AsyncStorage` 的方法就是各包 `@impasto/<pkg>/host` 上的导出。路径拒绝 `..`、绝对路径和反斜杠。带 schema 的存储库以后只依赖这两个包，不进 `solid-ui`。

## Gallery

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

- Kotlin 全部。触发条件是 Solid 被第二个真实应用消费。重启时走 `crates/quickjs/abi.yml` 的变更缓冲，信号用已发布的 `com.lightningkite:reactive`。
- `crates/ios`。预构建矩阵出现 iOS 列之前不写。
- 外接 devtools 协议服务。触发条件是第二个真实应用。应用内面板已经在 `packages/devtools`。
- Picture 字节。只在 Rust 里编解码一张 picture 的局部 `DrawList`，不包含 `LayerTree`。不定 opcode，不提交金样。TypeScript 和 Kotlin 不写对照实现。
- 跨 picture 选区。触发条件是阅读序，以及两端点之间的节点集合。
- 导航往返之后，`picture_writes` 只计仍在树上的页面 picture。
- 把 `submit_layers` 挪到单独的 gpu 线程。`Engine` 仍和会话一起在渲染线程上。

## 必须守住

- `reduceMotion` 把运动写成终值，并且仍然发出运动结束。只跳过 `tick_motions` 会把 overscroll 和页面弹簧留在半程。
- 路由的退出条目活到运动结束。
- 包使用 `onBack(priority, fn)`。直接调用 `core.onBack` 会排在渲染器那一次登记前面。
