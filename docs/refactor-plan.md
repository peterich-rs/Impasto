# 生产级迭代规划

已落地的布局、文本、手势、焦点、主题、控件、错误处理、包拆分、脚本加载，以及恢复前台后的 present、表面打开重试和销毁等待从本文删除，以代码和 `docs/architecture.md` 为准。这里留还没做的平台通道收尾。录制按 owner 切分 picture 已落地，以 `uicore` 和 `docs/architecture.md` 为准。渲染引擎已按资源拆成 `Typography`、`Painter`、`PictureCache` 和 `TextureStore`。

Kotlin 路线仍暂缓。`crates/quickjs/abi.yml` 维持现状。跨 picture 选区、Picture 字节和 `crates/ios` 见文末。

## 平台通道

应用和平台之间是直接调用。`uicore` 的函数以及 `js_http`、`js_socket_*` 已经是这种形状：glue 按参数类型封送，需要离开渲染线程的调用用 promise token 把 `PluginResult` 送回来。

权限、文件、存储、相机走同一条通道。权限库的公开 API 复刻 [permission_handler](https://pub.dev/packages/permission_handler)。引入时钉住 pub.dev 上的一个版本，`Permission`、`PermissionStatus`、`ServiceStatus` 以那一版为准。不把 Dart 插件拷进仓库，也不按权限逐个加框架函数。

### 现状

平台通道已经是直接值封送。`module_prepare` / `module_push_*` / `module_finish` 把参数逐个拷成 `PluginValue`。`Invoke::Sync` 在 JS 线程内联返回，`Invoke::Promise` 经 token 走 `IoEvent::Plugin`。`emit_value` 把结果递归建回 JS 值，≥4KB 的字节零拷贝（会话 slab + quickjs realloc 钩子回收）。`fetch` 整包，`fetchStream` 分块（`HttpHead` / `HttpChunk`），`HttpError.kind` 分类并在 reject 里给 DOMException 名。QuickJS 用 `JS_NewCModule` 物化 `@impasto/<pkg>/host`。一方模块的方法名和 kind 由 `cargo run -p hostshell --bin host-stubs` 写成各包 `host.gen.ts`，hostshell 测试防漂移。`WebSocket` 接受子协议参数。权限库 `@impasto/permissions` 从 `@impasto/permissions/host` 取五法。

还差的只有宿主端的 `PlatformBridge` 实现面：macOS 的权限请求弹窗和 `openAppSettings`、Android 的运行时权限 Intent。`Camera` 仍未注册。

### 权限库

包是 `packages/permissions`，包名 `@impasto/permissions`。五法从 `@impasto/permissions/host` 取。不依赖 `solid-renderer`、`solid-ui` 或 `solid-js`。

应用看到的是 permission_handler 的调用形状：

```ts
const status = await Permission.camera.status;
const next = await Permission.camera.request();
const batch = await request([Permission.location, Permission.storage]);
const show = await Permission.contacts.shouldShowRequestRationale;
const service = await Permission.locationWhenInUse.serviceStatus;
const opened = await openAppSettings();
```

`request` 对应 permission_handler 里对一组 `Permission` 调用的 `request`。`Permission` 的名字从钉住的那一版抄进权限包，是数据。状态是 `granted`、`denied`、`restricted`、`permanentlyDenied`、`limited`、`provisional`。服务状态是 `enabled`、`disabled`、`notApplicable`。

Android 上 `status` 对永久拒绝仍报 `denied`，只有 `request` 的结果才是 `permanentlyDenied`。`shouldShowRequestRationale` 只在 Android 询问系统，其它平台返回 false。这两条是该包的契约，不是单独的分支设计。

宿主对象名字是 `Permissions`，方法与 permission_handler 的平台接口同名，一共五个：

```ts
export interface Permissions {
  checkPermissionStatus(permission: string): Promise<string>;
  requestPermissions(permissions: string[]): Promise<Record<string, string>>;
  shouldShowRequestPermissionRationale(permission: string): Promise<boolean>;
  checkServiceStatus(permission: string): Promise<string>;
  openAppSettings(): Promise<boolean>;
}
```

权限包把 `Permission.camera.status` 映到 `checkPermissionStatus("camera")`。`onDeniedCallback` 一类链式回调留在权限包内，不进宿主。理由说明由应用自己画。权限包不导入 `solid-ui`。

宿主用一个 match 把名字映射到系统 API，映射表跟着钉住的 permission_handler 版本。不认识的名字返回 `denied`，服务状态返回 `notApplicable`。macOS 与 Android 实现这五个方法。iOS 等 `crates/ios` 存在再接。未注册时静态 `import "@impasto/permissions/host"` 是链接期 `ReferenceError`。

### 文件、存储、相机

`FileSystem`、`AsyncStorage` 的方法就是各包 `@impasto/<pkg>/host` 上的导出，不再另设计一套。

- `FileSystem`：`documentDirectory`、`read`、`write`、`delete`、`pick`。路径拒绝 `..`、绝对路径和反斜杠，并限制在 document 目录内，校验与 `crates/macos/src/assets.rs` 的 `asset_name` 相同。
- `AsyncStorage`：`multiGet`、`multiSet`、`multiRemove`、`clear`。
- `Camera`：有宿主实现后再加 `@impasto/camera` 包。

带 schema 或迁移的存储库不在这次写。它以后只依赖 `@impasto/file-system` 与 `@impasto/async-storage`，不进 `solid-ui`。

### 阶段

1. 已落地：`session` 的 `HostModule` / `Invoke`、`JS_NewCModule` 物化 `@impasto/<pkg>/host`、`Sync` 直返、`Promise` 走 `IoEvent::Plugin`、`emit_value` 建值、`fetchStream` 分块、零拷贝字节、`host-stubs` 生成各包 `host.gen.ts`。macOS 覆盖权限五法。
2. 待做：Android 的 `Bridge` 覆盖 `permission_status` / `request_permissions` / `open_app_settings`（运行时权限 Intent 与设置页跳转）；`Camera` 注册。

### 文件

- `crates/android/src/host.rs` -- `Bridge` 实现权限方法，替代 trait 默认值
- `@impasto/camera` -- 有宿主实现后再加包

路由、网络、表单和文案在 `packages/solid-router`、`packages/net`、`packages/form`、`packages/i18n`。权限在 `packages/permissions`。

## 诊断与日志

日志是调用点上的 `tracing` 事件。过滤是 `hostshell` 里的一份 `Targets`。debug 构建缺省 `trace`，release 构建缺省 `info`。`set_log_targets` 整份替换后重建 callsite 兴趣缓存。同一条事件分发到控制台、`mars-xlog` 文件（有日志目录时）和 devtools 的 `DevtoolSink`。超预算只在进入和恢复各打一条；进入行带分段原因，全量数字仍在 `FrameSample`。

`IMPASTO_INSPECT=<dir>` 已落地。macOS 在 present 成功后写一份 `frame.json`（层树、命令数、picture、耗时和布局问题）并退出；没有 `IMPASTO_DUMP` 时同一帧的 PNG 写到该目录的 `frame.png`。Android 把已记录的 `FrameTiming` 和层树写成 `frame.json`，不退出。应用内面板见 `docs/devtools-plan.md`。

## Gallery 能力面

gallery 是当前唯一的验收载体，像素金样跟着它的画面走。Quotes 模拟盘覆盖自选、市场、个股、交易，见 `docs/quotes-app.md`。`packages/solid-ui` 的 Menu、Switch、Tabs、Toast、Tooltip、TextArea、Icon、NavigationBar、KeyboardAvoidingView、Recycler、ScrollGroup、Field 掩码，`packages/net` 的 `createFetch`，`packages/form` 的 `createForm`，`packages/i18n`，以及 `@impasto/clipboard`、`@impasto/linking`、`@impasto/app-state`、`@impasto/file-system`、`@impasto/permissions` 都没有消费点。

验收标准是页面数加能力映射：每个框架包在 gallery 至少有一个真实消费点，`quotes.node.test.ts` 断言关键交互，像素金样跟着新画面重钉（容差每通道 2 不变）。已确认的页面划分：

| 页面 | 消费 | 验证 |
|---|---|---|
| `/controls` | Menu、Switch、Tabs、Icon | 弹层定位、开层裁剪、受控状态 |
| `/text` | Field 掩码、TextArea、KeyboardAvoidingView | TextEditor、字形簇编辑、IME |
| `/overlays` | Dialog、Sheet、Toast、Tooltip | OverlayRoot、转场、焦点圈 |
| `/data` | createFetch、VirtualList、Recycler | 分块流、窗口化、可见集 diff |
| `/persist` | AsyncStorage、FileSystem、createForm | 平台通道、promise token、表单校验 |
| `/device` | clipboard、linking、app-state、permissions | 宿主桥、权限五法、生命周期 |
| `/i18n` | packages/i18n | directionFor、复数、格式化 |

Quotes 的现有路由作回归基准。能力页另走开路由，不挤进模拟盘。不做 API 地图页——能力映射由测试断言承载，不生成第二份要维护的清单。

## 顺序

平台通道一节里的方法表、包拆分、脚本加载、`submit_bench`、列表滚动的 picture 记账、直接值封送、分块 HTTP、零拷贝字节和宿主表生成已经在代码里。恢复前台的 present、表面重试、销毁等待、`Invalidate` 和 `MemoryWarning` 也已经在代码里。gallery 产物出库（`just gallery-dist` 门面，quickjs 测试读文件并在缺产物时失败）和调用点日志（`tracing`、`Targets`、计时进 `FrameSample`）也已落地。

计时基准只在 `just bench`。质量门在 `just quality`，覆盖率在 `just coverage`，生成物在 `just gen`。

## 暂缓

- 跨 picture 选区。触发条件是出现阅读序和「两端点之间的节点集合」，并且选区 picture 挂在与文本相同的 `Offset` / `Clip` 下面。
- 外接 devtools 面板（协议服务）。触发条件是出现第二个真实应用；应用内面板已落地，见 `docs/devtools-plan.md`。
- Kotlin 全部。触发条件是 Solid 侧框架被第二个真实应用消费。重启时走 `crates/quickjs/abi.yml` 的变更缓冲。
- Picture 字节序列化，见 `docs/protocol-v2.md`。
- `crates/ios`。预构建矩阵出现 iOS 列之前不写。
- 导航往返之后 `picture_writes` 只计仍在树上的页面 picture。路由表已经是 `/schedule`、`/nav/:depth`、`/list`。包拆分不实现这条断言。
- 带 schema 或迁移的存储库。`AsyncStorage` 与 `FileSystem` 落地之后再写，只依赖 `@impasto/file-system` 与 `@impasto/async-storage`。
- 把 `submit_layers` 挪到单独的 gpu 线程。测量上下文和绘制上下文已经分开，层树是 `Send` 的。线程拆分还没做：`Engine` 仍和会话一起待在 `render` 线程上。

## 风险

- `reduceMotion` 必须把运动写成终值，并且仍然发出运动结束。只跳过 `tick_motions` 会把 overscroll 和页面弹簧留在半程，退出中的页面也不会摘掉。
- 路由的退出条目要活到运动结束。`<For>` 在 `phase == "exit"` 期间仍挂着该 key。
- 直接调用 `core.onBack` 会排在 renderer 那一次登记前面。包使用 `onBack(priority, fn)`。
- 文件路径与 `asset_name` 一样拒绝 `..`。绝对路径和反斜杠不进入 document 目录之外。
- 像素金样跟着 gallery 画面走。gallery 把 `theme().color.background` 传给 `RouterView`。画面不变则不重钉。容差仍是每通道 2。
