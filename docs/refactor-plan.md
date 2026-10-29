# 生产级迭代规划

已落地的布局、文本、手势、焦点、主题、控件、错误处理、包拆分、脚本加载，以及恢复前台后的 present、表面打开重试和销毁等待从本文删除，以代码和 `docs/architecture.md` 为准。这里留还没做的平台通道收尾。录制按 owner 切分 picture 已落地，以 `core` 和 `docs/architecture.md` 为准。渲染引擎已按资源拆成 `Typography`、`Painter`、`PictureCache` 和 `TextureStore`。

Kotlin 路线仍暂缓。`runtimes/quickjs/abi.yml` 维持现状。跨 picture 选区、Picture 字节和 `hosts/ios` 见文末。

## 平台通道

应用和平台之间是直接调用。`core` 的函数以及 `js_http`、`js_socket_*` 已经是这种形状：glue 按参数类型封送，需要离开渲染线程的调用用 promise token 把 `PluginResult` 送回来。

权限、文件、存储、相机走同一条通道。权限库的公开 API 复刻 [permission_handler](https://pub.dev/packages/permission_handler)。引入时钉住 pub.dev 上的一个版本，`Permission`、`PermissionStatus`、`ServiceStatus` 以那一版为准。不把 Dart 插件拷进仓库，也不按权限逐个加框架函数。

### 现状

平台通道已经是直接值封送。`module_prepare` / `module_push_*` / `module_finish` 把参数逐个拷成 `PluginValue`。`Invoke::Sync` 在 JS 线程内联返回，`Invoke::Promise` 经 token 走 `IoEvent::Plugin`。`emit_value` 把结果递归建回 JS 值，≥4KB 的字节零拷贝（会话 slab + quickjs realloc 钩子回收）。`fetch` 整包，`fetchStream` 分块（`HttpHead` / `HttpChunk`），`HttpError.kind` 分类并在 reject 里给 DOMException 名。QuickJS 用 `JS_NewCModule` 物化 `@uifw/<pkg>/host`。一方模块的方法名和 kind 由 `cargo run -p hostshell --bin host-stubs` 写成各包 `host.gen.ts`，hostshell 测试防漂移。`WebSocket` 接受子协议参数。权限库 `@uifw/permissions` 从 `@uifw/permissions/host` 取五法。

还差的只有宿主端的 `PlatformBridge` 实现面：macOS 的权限请求弹窗和 `openAppSettings`、Android 的运行时权限 Intent。`Camera` 仍未注册。

### 权限库

包是 `frameworks/permissions`，包名 `@uifw/permissions`。五法从 `@uifw/permissions/host` 取。不依赖 `solid-renderer`、`solid-ui` 或 `solid-js`。

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

宿主用一个 match 把名字映射到系统 API，映射表跟着钉住的 permission_handler 版本。不认识的名字返回 `denied`，服务状态返回 `notApplicable`。macOS 与 Android 实现这五个方法。iOS 等 `hosts/ios` 存在再接。未注册时静态 `import "@uifw/permissions/host"` 是链接期 `ReferenceError`。

### 文件、存储、相机

`FileSystem`、`AsyncStorage` 的方法就是各包 `@uifw/<pkg>/host` 上的导出，不再另设计一套。

- `FileSystem`：`documentDirectory`、`read`、`write`、`delete`、`pick`。路径拒绝 `..`、绝对路径和反斜杠，并限制在 document 目录内，校验与 `hosts/macos/src/assets.rs` 的 `asset_name` 相同。
- `AsyncStorage`：`multiGet`、`multiSet`、`multiRemove`、`clear`。
- `Camera`：有宿主实现后再加 `@uifw/camera` 包。

带 schema 或迁移的存储库不在这次写。它以后只依赖 `@uifw/file-system` 与 `@uifw/async-storage`，不进 `solid-ui`。

### 阶段

1. 已落地：`session` 的 `HostModule` / `Invoke`、`JS_NewCModule` 物化 `@uifw/<pkg>/host`、`Sync` 直返、`Promise` 走 `IoEvent::Plugin`、`emit_value` 建值、`fetchStream` 分块、零拷贝字节、`host-stubs` 生成各包 `host.gen.ts`。macOS 覆盖权限五法。
2. 待做：Android 的 `Bridge` 覆盖 `permission_status` / `request_permissions` / `open_app_settings`（运行时权限 Intent 与设置页跳转）；`Camera` 注册。

### 文件

- `hosts/android/src/host.rs` -- `Bridge` 实现权限方法，替代 trait 默认值
- `@uifw/camera` -- 有宿主实现后再加包

路由、网络、表单和文案在 `frameworks/solid-router`、`frameworks/net`、`frameworks/form`、`frameworks/i18n`。权限在 `frameworks/permissions`。

## 顺序

平台通道一节里的方法表、包拆分、脚本加载、`submit_bench`、列表滚动的 picture 记账、直接值封送、分块 HTTP、零拷贝字节和宿主表生成已经在代码里。恢复前台的 present、表面重试、销毁等待、`Invalidate` 和 `MemoryWarning` 也已经在代码里。

计时基准只在 `just bench`。质量门在 `just quality`，覆盖率在 `just coverage`，生成物在 `just gen`。

## 暂缓

- 跨 picture 选区。触发条件是出现阅读序和「两端点之间的节点集合」，并且选区 picture 挂在与文本相同的 `Offset` / `Clip` 下面。
- Kotlin 全部。触发条件是 Solid 侧框架被第二个真实应用消费。重启时走 `runtimes/quickjs/abi.yml` 的变更缓冲。
- Picture 字节序列化，见 `docs/protocol-v2.md`。
- `hosts/ios`。预构建矩阵出现 iOS 列之前不写。
- 导航往返之后 `picture_writes` 只计仍在树上的页面 picture。路由表已经是 `/schedule`、`/nav/:depth`、`/list`。包拆分不实现这条断言。
- 带 schema 或迁移的存储库。`AsyncStorage` 与 `FileSystem` 落地之后再写，只依赖 `@uifw/file-system` 与 `@uifw/async-storage`。
- 把排版或 `submit_layers` 挪出渲染线程。现在的模块拆分仍然是 `uifw-render` 上的一个 `!Send` 所有者。按文件拆开不产生线程边界。

## 风险

- `reduceMotion` 必须把运动写成终值，并且仍然发出运动结束。只跳过 `tick_motions` 会把 overscroll 和页面弹簧留在半程，退出中的页面也不会摘掉。
- 路由的退出条目要活到运动结束。`<For>` 在 `phase == "exit"` 期间仍挂着该 key。
- 直接调用 `core.onBack` 会排在 renderer 那一次登记前面。包使用 `onBack(priority, fn)`。
- 文件路径与 `asset_name` 一样拒绝 `..`。绝对路径和反斜杠不进入 document 目录之外。
- 像素金样跟着 gallery 画面走。gallery 把 `theme().color.background` 传给 `RouterView`。画面不变则不重钉。容差仍是每通道 2。
