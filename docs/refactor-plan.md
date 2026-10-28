# 生产级迭代规划

已落地的布局、文本、手势、焦点、主题、控件、错误处理、包拆分、脚本加载，以及恢复前台后的 present、表面打开重试和销毁等待从本文删除，以代码和 `docs/architecture.md` 为准。这里留还没做的平台通道收尾。录制按 owner 切分 picture 已落地，以 `core` 和 `docs/architecture.md` 为准。渲染引擎已按资源拆成 `Typography`、`Painter`、`PictureCache` 和 `TextureStore`。

Kotlin 路线仍暂缓。`runtimes/quickjs/abi.yml` 维持现状。跨 picture 选区、Picture 字节和 `hosts/ios` 见文末。

## 平台通道

应用和平台之间是直接调用。`core` 的函数以及 `js_http`、`js_socket_*` 已经是这种形状：glue 按参数类型封送，需要离开渲染线程的调用用 promise token 把 `PluginResult` 送回来。

权限、文件、存储、相机走同一条通道。权限库的公开 API 复刻 [permission_handler](https://pub.dev/packages/permission_handler)。引入时钉住 pub.dev 上的一个版本，`Permission`、`PermissionStatus`、`ServiceStatus` 以那一版为准。不把 Dart 插件拷进仓库，也不按权限逐个加框架函数。

### 现状

- `runtimes/quickjs/src/prelude.js` 的 `getModule` 按名字分支，只构造 `AppState`、`Linking`、`Clipboard`。`FileSystem`、`AsyncStorage`、`Camera` 在 `frameworks/modules` 里有类型，运行时未注册；`hasModule` 即使为真，`get` 仍返回 null。
- 模块方法走 `__uifw.call(name, method, json)`。`js_module_call` 把整段 JSON 交给 `module_call`，`session::PluginValue` 再把消息拆开。事件走 `qjs_emit` 的 JSON 字符串。`Linking` 的 `url` 监听在 prelude 里有扇出，`set_launch_url` 只写入 `launch_url`，没有宿主调用它，也没有发出 `url`。
- `getEnforcing` 的文案是 `module <name> unavailable`。Rust 侧未知模块是 `module is not linked`。
- `fetch` 已经是语言全局。`js_http` 分开接收 method、url、headers 和 body。

这条 JSON 调用是 Flutter `MethodChannel` / `EventChannel` 的形状：方法名加上编码后的消息。它留在三个内建模块上，不再扩展。

### 设计

留着 `make_promise`、`pending.plugin`、下一帧的 `flush_pending`、`IoJob::Plugin`、`submit_or_queue` 和 `PluginValue`。要换的是封送：脚本不组 JSON，glue 也不把结果 `JS_ParseJSON` 回来。

现在读一次启动 URL：

```text
Linking.getInitialURL()
  │
  ▼
prelude createLinking                 ← JSON.stringify
  │
  ▼
__uifw.call(name, method, json)
  │
  ▼
js_module_call                        ← make_promise
  │
  ▼
module_call                           ← parse_args
  │
  ▼
dispatch_module                       ← match (name, method)
  │
  ├─► pending.plugin                  ← AppState、getInitialURL
  └─► IoJob::Plugin
        │
        ▼
      hostshell::plugin               ← 另一处 match
        │
        ▼
qjs_settle_json                       ← 再编成 JSON
  │
  ▼
JS_ParseJSON → resolve(promise)
```

目标是宿主对象上的函数。`permissions.checkPermissionStatus("camera")` 与 `js_http` 一样按参数类型封送：

```text
checkPermissionStatus("camera")
  │
  ▼
对象上的 C 函数                       ★ JS_NewCFunctionData
  │
  ▼
JSValue → PluginValue                 ★ 读 argv
  │
  ▼
方法表
  │
  ├─► 当场 PluginResult               (existing pending.plugin)
  └─► IoJob::Plugin
        │
        ▼
      hostshell 按同一张表实现
        │
        ▼
PluginValue → JSValue                 ★ 按类型建值
  │
  ▼
resolve(promise)                      (existing make_promise)
```

`★` 是新增。`parse_args`、`qjs_settle_json` 的成功路径，以及 prelude 里按名字构造模块的分支删除。

方法表放在 `session`。每一行是模块名、方法名、`Session` 或 `Platform`，以及 `Sync` 或 `Promise`。`module_has` 与 `dispatch_module` 查这张表，不再各写一份名单。

`Sync` 只给不离开渲染线程、也不阻塞的 `Session` 方法。QuickJS 跑在渲染线程上，C 函数就是这次 JS 调用本身，可以直接 `return` JS 值。`AppState` 的当前状态和 `Linking.getInitialURL` 属于这一类：数据已经在会话里，调用方不用 `await`。`frameworks/modules` 里 `getInitialURL` 的类型改成 `string | null`。

`Platform` 一律是 `Promise`。剪贴板要等主线程，`openURL`、文件、权限对话框和网络都不能在渲染线程上等待。渲染线程若堵在平台线程上，而平台线程又要回到渲染线程，会卡住这一帧。这些方法继续 `submit_or_queue(IoJob::Plugin)`，由 `hosts/hostshell/src/net.rs` 的 `plugin` 按同一组名字实现。权限的五个方法都是 `Promise`。

QuickJS 就在 `uifw-render` 上，不在平台主线程上。平台主线程是 AppKit 主线程或 Android 的 Java 主线程，只做窗口、输入入队和 vsync 登记。脚本、命中、布局、录制和 `submit` 在同一条渲染线程上，文字测量才能调用这台 `!Send` 的 `Engine`。否决为了同步调用把 QuickJS 挪到平台主线程：主线程会停在脚本和布局上，树和引擎之间还要再加一次跨线程调用。光栅从这条线程拆出去要先分开排版和 `submit`，不属于这次通道改动。

`__uifw.module(name)` 查表并 `JS_NewObject`。每个方法用 `JS_NewCFunctionData` 挂上，函数数据里是模块名和方法名。`TurboModuleRegistry.get` 只缓存这个对象：

```js
get(name) {
  var cached = modules.get(name);
  if (cached) return cached;
  var object = bridge.module(name);
  if (!object) return null;
  modules.set(name, object);
  return object;
}
```

`get` 在模块未注册时返回 null。`getEnforcing` 抛出 `module "<name>" is not linked`。不认识的方法用同一句，带上方法名。Rust 与 prelude 用这一句。

入参照 `js_http`：在这次 C 调用里读 `argv`，拷进 `PluginValue` 后释放 C 字符串。类型是字符串、布尔、数字、`null`、数组、普通对象，以及 `ArrayBuffer` / `Uint8Array`。`QjsFns.module_call` 去掉 JSON 指针。`runtimes/quickjs/src/linked/host.rs` 的 `parse_args` 删除。

`Sync` 方法在 C 函数里把 `PluginValue` 建成 JS 值并直接返回，不分配 promise token。`Promise` 方法仍用 `make_promise`，完成时照 `qjs_settle_http` 建 JS 值。`settle_plugin` 不再 `encode` 成字符串。失败仍是字符串。上面的 `checkPermissionStatus` 是 `Promise`；`getInitialURL` 是 `Sync`，调用栈在 `JSValue → PluginValue` 之后就返回字符串或 `null`。

事件改成 `glue.c` 里 `BackHandler` 的做法：`addEventListener` 把 JS 函数留在宿主对象上。`set_launch_url` 之后对 `url` 监听 `JS_Call`，参数是 URL 字符串。`AppState` 的 `change` 传状态字符串。`qjs_emit` 与 prelude 的 `_emit` 在三个内建模块迁完后删除。

`getInitialURL` 仍读 `launch_url`。现在没有宿主调用 `set_launch_url`。各端的 URL 入口以后调用它。路由只读 Linking 这个对象。`CoreHost` 上的 `getInitialURL` 和 `onURL` 删除。

三个现有模块迁完，并且没有剩余的 `bridge.call` 之后，再删 `__uifw.call`。`FileSystem`、`AsyncStorage`、`Permissions` 之后只加方法表，不新增 glue 函数。权限包调用 `checkPermissionStatus("camera")`。

否决继续往 `getModule` 里加名字分支。否决按权限各写一个 glue 函数。否决把 permission_handler 的 MethodChannel 搬过来。

### 权限库

包是 `frameworks/permissions`，包名 `@uifw/permissions`。只依赖 `@uifw/modules`。不依赖 `solid-renderer`、`solid-ui` 或 `solid-js`。

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

宿主用一个 match 把名字映射到系统 API，映射表跟着钉住的 permission_handler 版本。不认识的名字返回 `denied`，服务状态返回 `notApplicable`。macOS 与 Android 实现这五个方法。iOS 等 `hosts/ios` 存在再接。未实现的平台上，对象不注册，`getEnforcing` 抛出统一文案。

### 文件、存储、相机

`frameworks/modules` 里已有的 `FileSystem`、`AsyncStorage`、`Camera` 接口就是宿主对象的方法，不再另设计一套。

- `FileSystem`：`documentDirectory`、`read`、`write`、`delete`、`pick`。路径拒绝 `..`、绝对路径和反斜杠，并限制在 document 目录内，校验与 `hosts/macos/src/assets.rs` 的 `asset_name` 相同。
- `AsyncStorage`：`multiGet`、`multiSet`、`multiRemove`、`clear`。
- `Camera`：接口保持 `capture` 与 `preview`。在有宿主实现之前不注册。

带 schema 或迁移的存储库不在这次写。它以后只依赖 `@uifw/modules`，不进 `solid-ui`。

### 阶段

1. `session` 方法表，`__uifw.module(name)` 用 `JS_NewCFunctionData` 生成对象。入参从 `argv` 拷进 `PluginValue`。`Session` 且不阻塞的方法直接返回 JS 值，`Platform` 方法仍走 promise。`AppState`、`Linking`、`Clipboard` 从 prelude 分支和 `__uifw.call` 迁入。`hostshell` 的 `plugin` 改查同一张表。`set_launch_url` 对 `url` 监听 `JS_Call`。删 `parse_args`、`qjs_settle_json` 的成功路径、`qjs_emit` 和 prelude 的 `_emit`。三个模块没有剩余 `bridge.call` 之后删 `__uifw.call`。统一未注册文案。`CoreHost.getInitialURL` 与 `onURL` 删除。
2. 注册 `FileSystem` 与 `AsyncStorage`，接上路径校验和 document 目录。`Camera` 仍不注册。脚本加载已经用 `JS_SetModuleLoaderFunc`。
3. 注册 `Permissions` 的五个方法，macOS 与 Android 按钉住的 permission_handler 版本做名字映射。加入 `frameworks/permissions`。

### 文件

- `session/src/io.rs` -- 方法表（模块名、方法名、`Session` 或 `Platform`、`Sync` 或 `Promise`）；`PluginValue` 仍只在运行时内部封送
- `runtimes/quickjs/src/glue.c`、`runtime_host.inc` -- `__uifw.module`、`JS_NewCFunctionData`；`module_call` 接收 `PluginValue`；返回值按类型建 JS 值；事件用 `JS_Call`
- `runtimes/quickjs/src/linked/host.rs` -- 删除 `parse_args`
- `runtimes/quickjs/src/prelude.js` -- `get` 只缓存 `bridge.module`；删除 `createAppState`、`createLinking`、`createClipboard`、`_emit`
- `runtimes/quickjs/src/linked.rs` -- `dispatch_module` 查方法表；`set_launch_url` 调用 `url` 监听
- `hosts/hostshell/src/net.rs` -- `plugin` 按同一张表实现 `Platform` 方法
- `hosts/macos`、`hosts/android` -- `FileSystem`、`AsyncStorage`、`Permissions`
- `frameworks/modules/src/*.ts` -- 类型保持为宿主对象；`Linking.getInitialURL` 改为同步返回；补 `Permissions`
- `frameworks/permissions/` -- `@uifw/permissions`，公开 API 对齐 permission_handler
- `frameworks/solid-renderer/src/host.ts`、`src/guard.ts`、`runtime/index.ts` -- 删除 `getInitialURL`、`onURL`
- `frameworks/solid-ui/testing/renderTest.ts`、`src/guard.test.ts` -- 深链改走模块对象

路由、网络、表单和文案在 `frameworks/solid-router`、`frameworks/net`、`frameworks/form`、`frameworks/i18n`。权限在 `frameworks/permissions`。

## 顺序

平台通道一节里的方法表、包拆分、脚本加载、`submit_bench` 和列表滚动的 picture 记账已经在代码里。恢复前台的 present、表面重试、销毁等待、`Invalidate` 和 `MemoryWarning` 也已经在代码里。

计时基准只在 `just bench`。

## 暂缓

- 跨 picture 选区。触发条件是出现阅读序和「两端点之间的节点集合」，并且选区 picture 挂在与文本相同的 `Offset` / `Clip` 下面。
- Kotlin 全部。触发条件是 Solid 侧框架被第二个真实应用消费。重启时走 `runtimes/quickjs/abi.yml` 的变更缓冲。
- Picture 字节序列化，见 `docs/protocol-v2.md`。
- `hosts/ios`。预构建矩阵出现 iOS 列之前不写。
- 导航往返之后 `picture_writes` 只计仍在树上的页面 picture。路由表已经是 `/schedule`、`/nav/:depth`、`/list`。包拆分不实现这条断言。
- 带 schema 或迁移的存储库。`AsyncStorage` 与 `FileSystem` 落地之后再写，只依赖 `@uifw/modules`。
- 把排版或 `submit_layers` 挪出渲染线程。现在的模块拆分仍然是 `uifw-render` 上的一个 `!Send` 所有者。按文件拆开不产生线程边界。
- `core/src/tree/record.rs`、`runtimes/quickjs/src/linked.rs`、`hosts/hostshell/src/shell.rs` 和 `core/src/layout_test.rs` 还没按主题拆文件。公有 API 不因此改变。

## 风险

- `reduceMotion` 必须把运动写成终值，并且仍然发出运动结束。只跳过 `tick_motions` 会把 overscroll 和页面弹簧留在半程，退出中的页面也不会摘掉。
- 路由的退出条目要活到运动结束。`<For>` 在 `phase == "exit"` 期间仍挂着该 key。
- 直接调用 `core.onBack` 会排在 renderer 那一次登记前面。包使用 `onBack(priority, fn)`。
- 文件路径与 `asset_name` 一样拒绝 `..`。绝对路径和反斜杠不进入 document 目录之外。
- 像素金样跟着 gallery 画面走。gallery 把 `theme().color.background` 传给 `RouterView`。画面不变则不重钉。容差仍是每通道 2。
