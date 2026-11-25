# Rust 业务下沉与原生插件体系规划

应用可将业务逻辑放在自己的 Rust crate（本地插件），也可链接独立的 Rust 插件 crate（平台插件）。二者共用 `impasto` SDK、ESM 绑定和显式注册入口。CLI 按目标平台选择插件，生成类型声明并把插件静态链接进应用入口。本文记录插件契约、构建流程和平台验收条件。

## 1. 宿主边界

| 环节 | 现有实现与约束 |
| --- | --- |
| 宿主契约 | [`javascript::host`](../crates/javascript/src/host/mod.rs) 在关闭默认 feature 时只依赖 `std`。`Module` 和 `Method` 描述同步方法、Promise、资源工厂和流。`PluginValue` 通过自有值传数据，`StreamSink` 提供 credit 背压和取消。 |
| 运行时 | [QuickJS binder](../crates/javascript/src/runtime/quickjs/binder.rs) 把 `ModuleRegistry` 注册为原生 ESM。`NativeResource` 留在 VM 所在线程。一方资源沿用 `call(method, ...args)` / `close()`，SDK 对象使用生成的类型化代理。异步工厂通过 [`ResourceFactory`](../crates/javascript/src/host/value.rs) 把资源构造交回 VM 线程。 |
| 类型与打包 | [`module-stubs`](../crates/embedder/common/src/module_stubs.rs) 生成一方包的 `src/native.d.ts`。目标构建清单记录插件声明路径以及 [`pack.mjs`](../tools/pack.mjs) 使用的插件导入映射。Release 使用 [`compile_with_native_exports`](../crates/engine/src/js/compiler.rs) 校验静态导入。 |
| 装配与权限 | 公共宿主通过 [`Boot`](../crates/embedder/common/src/lib.rs) 接收模块并补齐一方能力。[`ModuleConfig::restrict`](../crates/engine/src/session/modules.rs) 按 `@impasto/` 后缀匹配 `app.modules`。现有的 [`@impasto/native-plugins`](../crates/embedder/common/src/modules/native_plugins.rs) 是 iOS 的显式启用 JSON 桥接入口，继续沿用其现有规则。 |

插件通过 Rust 源码与宿主一同构建，沿用 SDK 与框架工具链的版本约束。应用和插件可使用自己的业务依赖。插件及其 Cargo 构建脚本属于应用信任的本地代码，拥有应用进程权限。`app.toml` 的 `[permissions]` 控制框架能力的装配和暴露，OS 权限仍由系统和应用授权决定。这套集成采用静态注册，超出范围的扩展包括 Node-API、运行时动态库加载和原生代码热替换。

## 2. SDK、注册与调度

### 2.1 依赖方向

箭头表示 Cargo 依赖方向。

```text
应用 native/ 与平台插件 ──> impasto ──> javascript（default-features = false）
                               └──────> impasto-macros（过程宏）
应用平台入口 ──> 目标平台 embedder ──> embedder/common ──> impasto
      └────────> 生成的插件汇总 crate ──> 应用 native/ 与平台插件
```

- `impasto` 提供公共 SDK 和宏重导出。宏 crate 只生成代码，用 SDK 路径调用转换与注册 API，避免循环依赖。`impasto::abi` 重导出必要的底层契约，供宏展开与手写适配使用。
- `javascript`、`engine` 和 `raster` 无需引用插件 SDK。SDK 同样不依赖引擎、图形栈、QuickJS、平台 embedder 或 Tokio。新增依赖断言纳入 [`just boundaries`](../justfile)。
- `impasto::os::{ios, android, macos}` 预留为平台扩展位置，按实际平台使用 `#[cfg(target_os = "...")]`。平台依赖在插件自己的 `Cargo.toml` 中按目标选择。
- 开发期生成的应用工作区必须把平台入口、插件汇总 crate 与各插件解析到同一份 SDK 和同源的 `javascript::host`。CLI 在装配前报告版本或来源冲突。Cargo 可以同时解析同名包的不同版本或来源，feature 也会在同一依赖图内合并，单个 `Cargo.lock` 无法替代这项校验。[Cargo 依赖解析](https://doc.rust-lang.org/cargo/reference/resolver.html) · [Cargo feature 规则](https://doc.rust-lang.org/cargo/reference/features.html#feature-unification)

### 2.2 一个注册入口，两种消费方式

每个插件 crate 采用 `pub fn register(registrar: &mut PluginRegistrar)`。它可以添加多个模块。CLI 为选中的 crate 生成依次调用 `register` 的 Rust 代码。元数据导出器和宿主都调用这份入口，注册顺序确定，无需依赖链接段扫描。

```rust
pub trait ImpastoPlugin: Send + Sync + 'static {
    fn describe(&self) -> ModuleDescriptor;
    fn build_module(&self, ctx: &PluginContext) -> Result<abi::Module, PluginInitError>;
}
```

- `PluginRegistrar` 是可借用的具体收集器，`add(plugin: impl ImpastoPlugin)` 在内部持有不同类型的模块工厂。工厂与 `describe()` 的创建路径均只收集描述，允许在没有宿主、VM 或平台窗口的进程中执行。
- `ModuleDescriptor` 记录完整 ESM specifier、注册名、方法签名和调用类型、关联类型定义、事件名和事件负载类型。沿用 `ModuleExports` 的命名和导出规则，类型声明、ESM 导出名与运行时绑定共享描述。ESM 导出表只收录运行时值。应用构建清单从描述派生，手写模块也要经过同样的校验。
- `build_module` 由 embedder 在服务就绪后、脚本执行前调用。模块实例归当前窗口的宿主所有。初始化失败要带模块名和原因返回，清理本次已装配模块，再终止该宿主启动。网络请求和系统交互由已装配模块的方法按需发起。
- 装配按来源区分模块。一方能力从现有白名单按 `[permissions]` 和目标平台过滤，插件按已解析的构建计划加入。执行一方过滤后再合并扩展，三个平台和 headless 测试入口都遵循同一规则。为一方保留的 specifier 和注册名，即使本次关闭，也不能由插件占用。合并后使用 [`ModuleRegistry::try_new`](../crates/javascript/src/host/registry.rs) 校验重复名称、导出和事件，不修改一方 `package_folder` 的文件布局规则。

### 2.3 执行、服务与生命周期

`PluginContext` 提供绑定到当前宿主和模块的可克隆句柄。跨队列只传自有值、可发送句柄或任务结果。现有 [`RtExec`](../crates/embedder/common/src/exec.rs) 继续承载 worker 与平台主线程调度，async Future 由公共宿主复用 [`rt.rs`](../crates/embedder/common/src/rt.rs) 的进程级运行时。

| 能力 | 规划契约 |
| --- | --- |
| `Runtime` | `spawn` 接受 `Send + 'static` Future，`spawn_blocking` 承载同步 I/O 和 CPU 重任务，`spawn_main` 复用平台 TaskRunner 并在调度不可用时返回错误。公开签名使用 `std::future::Future` 与 SDK 自己的任务错误。宏通过宿主调度器提交任务，让请求预算与关闭行为沿用同一条宿主路径。 |
| 取消 | SDK 提交的任务归宿主所有。宏派发 JS 调用时关联发起它的 VM 和请求，实例方法额外关联资源。关闭、超时或分离停止交付结果，并向任务发出取消信号。已开始的阻塞任务需要协作退出，外部副作用由发起它的业务负责。`Task<T>` 返回执行结果，任务跟踪由宿主管理。 |
| `AppPaths` | 路径服务提供 `document_dir`、`cache_dir` 和 `temp_dir` 的读取接口，目录不可用时返回错误。使用 JS 传入的文件名时由业务层校验，并保留宿主沙盒路径约束。 |
| HTTP / WebSocket | SDK 通过 embedder 适配器复用宿主提供的传输。按目标检查 `[permissions]` 后再注入可用服务，缺失由显式值或结构化错误表示。先接入异步 HTTP 与取消，WebSocket 在需要时复用同样的边界。HTTP 适配必须处理现有 [`HttpClient`](../crates/embedder/common/src/http.rs) 的阻塞契约；流式响应沿用有界通路，DevTools 观测复用现有包装层。 |
| 事件 | 模块声明 `events = E` 后，宏绑定该模块的 `EventEmitter<E>`。发射器只能发送本模块声明的事件，复用 `HostEvent` 交给 UI 队列，JS 回调由当前 VM 执行。持续高频数据使用具备背压的 `ReadableStream`。 |
| 生命周期 | SDK 生命周期枚举由 embedder 从 [`engine::session::Lifecycle`](../crates/engine/src/session/input.rs) 映射。订阅归宿主，`Subscription` 释放时退订。通知在 UI 队列串行执行，系统 UI 操作转交平台主线程。订阅回调以弱句柄引用其所有者，避免与持有订阅的对象形成循环。 |
| 日志 | 插件复用宿主安装的 [`tracing` 订阅器](../crates/embedder/common/src/log_sub.rs)。复用范围取决于最终依赖解析与宿主已开启的日志出口。 |

脚本更新会新建候选 VM，通过首帧校验后才替换当前 VM。现有 [`update_source`](../crates/engine/src/js/player/mod.rs) 会复用宿主模块配置。VM 级对象、Promise、流和监听以 VM 代次隔离，候选被拒绝时清理候选自己的工作，切换后旧结果不能送到新 VM。宿主级模块可能被两代 VM 短暂共享，脚本更新无法回滚已发生的网络、写盘或平台操作。

## 3. 宏与跨语言契约

### 3.1 导出与方法映射

| 标注 | 职责 |
| --- | --- |
| `#[impasto::module(specifier = "...")]` | 声明 ESM 模块与注册入口。默认注册名取 Rust 类型名，冲突时显式改名。无状态模块可用单元结构体，有状态模块通过 `init = from_context` 指定 `fn(&PluginContext) -> Result<Self, PluginInitError>` 初始化函数。 |
| `#[impasto::export]` | 标注对应的固有 `impl`，导出其中的 `pub` 方法。默认用 `camelCase`，可显式改名，重名报编译错误。 |
| `#[derive(ImpastoType)]` | 为 DTO 生成值转换和 TypeScript 类型描述。派生结果描述引用的类型，模块渲染声明时遍历引用并去重。 |
| `#[impasto::object]` | 声明由 Rust 持有的对象类型。JS 得到带类型化方法与 `close()` 的代理。 |
| `#[derive(ImpastoEvents)]` | 为模块事件枚举生成负载转换与监听签名，沿用 `addEventListener` / `removeEventListener`。 |

| Rust 形式 | JS 形式与执行位置 |
| --- | --- |
| `fn f(...)` 返回 `T` 或 `Result<T, E>` | `T`，在 VM 所在线程执行，适合短小的内存操作。 |
| `async fn f(...)` 返回 `T` 或 `Result<T, E>` | `Promise<T>`，解码为自有参数后派发到宿主异步池。 |
| `#[impasto::method(blocking)] fn f(...)` | `Promise<T>`，在阻塞工作池执行。 |
| `#[impasto::method(main_thread)] fn f(...)` | `Promise<T>`，通过平台 TaskRunner 执行。该属性只支持同步 Rust 函数，主线程借用限于回调执行期间。 |
| `#[impasto::method(awaits_user)]` | 用于需要等待系统用户交互的 Promise 方法。取消或宿主分离时释放槽位与交互关联，沿用现有 `Method::awaits_user` 的免超时语义。 |
| `async fn f(..., sink: TypedSink<T>, cancel: CancelToken) -> Result<(), E>` | `ReadableStream<T>`，JS 可在末尾传 `signal?: AbortSignal`。`sink.send(item).await` 受 credit 约束，返回错误使流结束为错误。JS 主动取消、对象关闭或 VM 销毁会通知生产者，未传信号时同样受宿主生命周期约束。 |
| `#[impasto::method(resource)] fn open(...)` 或 `async fn open(...)`，返回 `Obj` 或 `Result<Obj, E>` | `Obj` 或 `Promise<Obj>`。显式标注对象工厂，由 Rust 类型检查验证 `Obj` 的对象契约。 |

同步 `&str` 参数在调用期间借用。异步、blocking、main-thread 方法仅接受可跨队列的自有参数。首版导出限于固有方法、关联函数和受支持的具体类型。宏在签名处拒绝不兼容的属性组合、泛型导出和借用返回值。Promise 的首版 JS 签名沿用现有行为，取消信号来自宿主与资源生命周期。流继续接受 JS `AbortSignal`。

### 3.2 结构化值与错误

| Rust 类型 | TypeScript 契约 |
| --- | --- |
| `()`、`bool`、`String` | 无返回值在 TypeScript 声明为 `void`，底层单元值使用 `PluginValue::Null`。`bool` 和字符串映射为 `boolean` 和 `string`。 |
| `i8` 到 `i32`、`u8` 到 `u32`、`f32` / `f64` | `number`。整数校验有限、无小数和目标类型范围，浮点按位宽转换。 |
| `i64` / `u64` | 默认 `number`，编码和解码都校验 JS 安全整数范围。对需要完整范围的 ID 可使用 `#[impasto(as_string)]`，以十进制 `string` 往返并校验符号与范围。 |
| `Bytes` 或 `#[impasto(bytes)] Vec<u8>` | `Uint8Array \| ArrayBuffer`，底层为 `PluginValue::Bytes`。当前 [codec](../crates/javascript/src/runtime/quickjs/codec.rs) 对小 payload 输出 `Uint8Array`，大 payload 输出 `ArrayBuffer`，边界存在复制。 |
| `Option<T>` | 结构体字段和尾部参数为可选项，输入签名允许 `null`。`None` 字段在输出对象中省略。`#[impasto(nullable)]` 字段总是输出，`None` 写成 `null`。独立返回值或非尾部参数使用 `T \| null`。 |
| `Vec<T>`、字符串键 map | `T[]` 和 `Record<string, T>`。二进制走专用的 `Bytes` 表示。 |
| 派生结构体、枚举 | 结构体生成 `interface`，纯单元枚举生成字面量联合，带数据的枚举生成带 `type` 判别字段的联合。允许声明改名和判别字段，宏检查字段与标签冲突。 |
| `Result<T, E>` | 成功只暴露 `T`。Rust 层保留结构化错误，绑定层按 `ImpastoError` 生成 `[scope:code] message`，同步抛错、Promise 拒绝或流失败。参数转换错误带参数或字段路径。 |

绑定沿用 codec 的遍历深度、项数和字节预算。宏基于解码后的值校验，当前 codec 会把 `undefined` 与若干非值类型映成 `Null`。可空、缺失与不支持的 JS 类型若要有严格区分，需在插件入口补齐解码策略，并保留一方模块兼容语义。

### 3.3 异步对象需要扩展底层契约

现有 [`NativeResource`](../crates/javascript/src/host/extensions.rs) 的同步 `call(&mut self, ...)` 与 [`ResourceTable`](../crates/javascript/src/runtime/quickjs/native_resource.rs) 支持 owner 线程独占与回收。新增类型化对象分发保留这条路径及现有 WebSocket 的调用形态。

1. **同步对象**用 `#[impasto::object(local)]` 与 `#[impasto::export(local)]` 在 VM 线程持有，可用 `&mut self`。支持 async、blocking、main-thread 方法或流的对象用自有共享状态，要求 `Send + Sync + 'static`，异步或跨线程实例方法使用 `&self`。生成的代理先校验实例和方法并取得自有调用数据，再释放资源表借用后派发。插件负责需要串行化的业务操作。
2. **异步工厂**需要新增可发送的资源完成结果。现有 `Box<dyn NativeResource>` 可以是线程亲和的，保持由 VM 线程创建和访问。异步完成只交付可发送的自有状态或构造器，并走现有的一次性请求路由。VM 线程确认请求和代次有效后才构造并登记资源、解析 Promise。待处理工厂和存活对象都受限额约束，超限或迟到的完成释放携带的状态。
3. **关闭与回收**使用同一个幂等入口。`close()` 立即使句柄失效，取消该对象的任务和流，GC、VM 退出或更新同样触发清理。已取得对象引用的任务可能仍在退出，平台资源按其所属队列释放。流应唤醒等待 credit 的生产者，迟到结果不得调用 JS 或复活句柄。

### 3.4 应用侧示例

以下用法展示从 Rust 异步工厂和实例方法生成的声明，以及应用如何使用这些声明。

```rust
use impasto::{ImpastoType, PluginRegistrar};

#[derive(Clone, ImpastoType)]
pub struct OrderSummary {
    pub id: String,
    pub total_cents: u32,
}

#[impasto::object]
pub struct OrderStore {
    orders: Vec<OrderSummary>,
}

#[impasto::export]
impl OrderStore {
    pub async fn query(&self) -> Vec<OrderSummary> {
        self.orders.clone()
    }
}

#[impasto::module(specifier = "@app/order")]
pub struct OrderModule;

#[impasto::export]
impl OrderModule {
    #[impasto::method(resource)]
    pub async fn open_store() -> OrderStore {
        OrderStore {
            orders: vec![OrderSummary { id: "order-1".into(), total_cents: 2599 }],
        }
    }
}

pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(OrderModule::plugin());
}
```

```ts
declare module "@app/order" {
  export interface OrderSummary {
    id: string;
    totalCents: number;
  }

  export interface OrderStore {
    query(): Promise<OrderSummary[]>;
    close(): void;
  }

  export function openStore(): Promise<OrderStore>;
}
```

```tsx
import { For, createSignal, onCleanup } from "solid-js";
import { Text, View } from "@impasto/solid-ui";
import { openStore, type OrderStore, type OrderSummary } from "@app/order";

export function OrderPage() {
  const [orders, setOrders] = createSignal<OrderSummary[]>([]);
  const [error, setError] = createSignal("");
  let disposed = false;
  let store: OrderStore | undefined;

  onCleanup(() => {
    disposed = true;
    store?.close();
  });

  void (async () => {
    const opened = await openStore();
    if (disposed) {
      opened.close();
      return;
    }

    store = opened;
    try {
      const next = await opened.query();
      if (!disposed) setOrders(next);
    } finally {
      opened.close();
      store = undefined;
    }
  })().catch((reason: unknown) => {
    if (!disposed) setError(String(reason));
  });

  return (
    <View>
      <For each={orders()}>{(order) => <Text>{order.id}: {order.totalCents}</Text>}</For>
      <Text>{error()}</Text>
    </View>
  );
}
```

## 4. 插件发现与工程配置

### 4.1 应用清单

```toml
name = "my-shop"
version = "0.1.0"
targets = ["macos", "ios", "android"]

[permissions]
http = true

[native]
path = "native"

[plugins]
camera = { path = "../plugins/impasto-camera", targets = ["ios", "android"] }

[ios]
bundle_id = "com.example.myshop"

[ios.usage_descriptions]
NSCameraUsageDescription = "用于拍摄商品照片与扫描订单二维码"

[android]
application_id = "com.example.myshop"
```

- `[native]` 指向含 `Cargo.toml` 的应用 crate，启用于所有应用目标。模块标识来自 Rust 导出，应用私有模块约定使用 `@app/`。
- `[plugins]` 的键是应用内别名，`path` 相对应用根。首版支持本地路径。Git 与 registry 来源待清单定位、版本锁定及分发规则确定后扩展。
- 插件启用平台默认取应用与插件声明的交集。显式 `targets` 必须属于两者，误配时报出插件与目标。现有 CLI 使用共享的 `lib/main.tsx`。共享代码静态导入某插件时，该插件必须在所有构建该入口的目标上启用。需要按平台切换应用入口时，先扩展 CLI 的入口配置。
- `[permissions]` 继续使用一方能力键，值可为布尔或目标列表。插件的 `requires` 必须在当前目标启用且由宿主实现。`optional` 允许服务缺失，插件需处理该状态。OS 权限与签名设置单独处理。

### 4.2 可复用插件清单

`plugin.toml` 放在插件 crate 根目录。`min_impasto_version` 约束负责清单解析和打包的框架工具链版本。Cargo 包名、版本、SDK 依赖和 crate 自身 feature 由 `Cargo.toml` 定义，注册函数名固定，原生 ESM specifier 由 Rust 导出描述给出。

```toml
[plugin]
targets = ["ios", "android"]
min_impasto_version = "0.1.0"

[js]
specifier = "@impasto/camera"
entry = "ts/index.ts"

[capabilities]
requires = []
optional = ["http"]

[ios]
sources = ["ios/CameraBridge.swift"]
frameworks = ["AVFoundation", "CoreMedia", "CoreVideo"]
min_version = "15.0"
usage_description_keys = ["NSCameraUsageDescription"]

[android]
sources = ["android/src/main/kotlin"]
permissions = ["android.permission.CAMERA"]
min_sdk = 24

[[android.uses_features]]
name = "android.hardware.camera"
required = false
```

`[js]` 可省略。存在包装层时，`js.specifier` 仅表示公开 TS 入口。`ts/index.ts` 通过完整 specifier 导入 Rust 声明的原生模块，例如 `@impasto/camera/native`。这条导入和应用直接导入本地模块共用打包清单。相对路径都相对于插件根，生成器校验规范化后的输入路径，拒绝越界和缺失资源。

### 4.3 合并规则

- 按目标合并插件源码、强弱链接 Frameworks、Android 权限与硬件声明、最低系统版本。同名同值去重，Android `uses-feature` 的 `required` 取并集。Framework 强弱链接或同一 Gradle 坐标的固定版本冲突时，列出来源并终止构建。插件可以声明固定版本的 Gradle 坐标，生成器将其写入依赖表。
- 最低系统版本取宿主基线与插件要求的最大值，并同步给原生编译、链接和工程设置。产物绑定该最低版本，CLI 在构建摘要列出导致提高版本的插件。
- 插件声明必要的隐私说明键，应用填写非空用途文案。缺失时在打包前报错。Android manifest 声明与系统运行时授权都需具备；签名、entitlements 和应用标识由应用配置控制，插件可声明所需能力，由生成器验证。
- `[macos]` 支持 Swift、Objective-C 和 C 源码，以及强、弱链接 Framework。插件在 `required_entitlements` 中列出需要的权限，应用在 `[macos.entitlements]` 中启用，并在 `[macos.usage_descriptions]` 中填写系统用途文案。

## 5. CLI 构建与开发流程

### 5.1 按目标解析并生成

```text
app.toml + plugin.toml + Cargo.toml
  → 解析目标、能力与来源
  → 生成目标插件汇总 crate，在开发机编译并运行描述导出器
  → 产出该目标的 manifest.json 和 .d.ts
  → pack.mjs 打包，Release 用同一描述目录校验 IQJS 导入
  → 用同一组插件编译目标入口，合并平台工程与应用产物
```

1. **输出与依赖解析。** CLI 生成 `build/Cargo.toml` 应用构建工作区，包含 `build/plugins/<platform>/` 下的汇总 crate 与对应的 `build/<platform>/native/` 平台入口。汇总 crate 以明确别名引用应用和选中的插件 crate。生成工作区有独立 `Cargo.lock` 和显式 resolver，由该工作区应用宿主必需的 profile 设置。开发机导出与目标入口共用已解析的锁文件，第二步以后按锁构建。兼容性错误要指向 SDK 版本、来源或插件依赖。构建并发时锁住共享工作区的生成和使用，分别暂存和发布各目标与 profile 的产物，清除已移除模块的声明。[Cargo 工作区规则](https://doc.rust-lang.org/cargo/reference/workspaces.html)
2. **主机导出，目标校验。** 主机编译只选择目标对应的汇总 crate 与导出器。`#[cfg(target_os)]` 可以切换实现与依赖，公开 specifier、导出、类型和事件在该目标选中的 crate 中必须能由主机编译导出，并在各构建 profile 与目标 ABI 上保持一致。需要专用 SDK 的代码隔离在目标实现下，开发机实现可以返回能力不可用。macOS 插件若链接清单内的 Swift 或 Objective-C 源码，可用 `cfg(impasto_metadata)` 为开发机导出器提供无需链接的实现。插件应在 Cargo 的 `unexpected_cfgs` lint 中声明此 cfg。`register` 和 `describe` 只读取静态描述。描述导出失败时提示出错插件和目标。目标产物携带构建时的插件描述摘要，执行脚本前校验目标端登记的描述和实际模块接口。一方模块目录沿用该目标的权限与平台能力规则，再与插件目录合并。
3. **生成清单与 TypeScript。** `build/plugins/<platform>/manifest.json` 记录 schema 版本、平台、模块注册名、完整 specifier、导出列表、声明路径、TS 入口映射和合并后的平台工程输入。声明位于同目录 `types/`。打包和 [`impasto analyze`](../crates/cli/src/analyze.rs) 消费该目标清单，分析器把声明加入根文件并为包装入口添加路径映射。IDE 配置只引用一个目标的声明集。
4. **打包与链接校验。** `pack.mjs` 接收 `--plugins-manifest`，按完整名解析包装入口和原生 ESM。现有一方包规则、`scene` / `reactive` 外部映射以及 `assertNoScheduler` 继续生效。预检冲突包含一方包、启用的包装入口、原生模块和内建模块。对已配置但未在当前目标启用的包装入口直接报错，避免回退到同名 npm 包。Release 将清单的字符串描述加入编译器目录，沿用 [现有静态链接校验](../crates/javascript/src/compiler/mod.rs)。扩展编译器消费自有 specifier 与导出名，运行时 `Module` 保有实际回调。Debug 在打包时检查外部 specifier，运行时检查导出并拒绝不兼容的更新。
5. **未选平台的命令。** 插件应用的 `build bundle` 与 `analyze` 使用 `--platform` 选择清单；应用只声明一个目标时可推断。有多个目标时要求明确选择，输出到该目标的构建目录。现有 `impasto test` 的 headless 宿主只装配一方模块；首版遇到插件应用给出明确诊断，插件 Rust 逻辑使用 `cargo test`，桥接测试由专门的轻量宿主或平台验收运行。

无本地 crate 且当前平台无启用插件时，沿用现有宿主构建、打包与脚本更新流程。新增路径不生成插件入口或触发额外的应用 Rust 链接。

### 5.2 平台入口与产物

| 平台 | 需要改造的入口 |
| --- | --- |
| macOS | 给 [`embedder-macos`](../crates/embedder/macos/Cargo.toml) 提供库入口，现有 `macos` bin 和生成的 App bin 共用启动逻辑，后者注入插件注册函数。现有 `.app/Contents/MacOS/impasto` 路径继续使用。 |
| iOS | 由 [`embedder-ios`](../crates/embedder/ios/Cargo.toml) 提供可链接的 Rust 库与宿主构造函数，生成入口作为最终 `staticlib`。延用 [`native-static-libs`](../crates/cli/src/ios.rs) 查询与 staged Xcode 工程，把最终产物和链接参数一同交给 Xcode。真机、模拟器与 profile 分开暂存。 |
| Android | [`embedder-android`](../crates/embedder/android/Cargo.toml) 已支持 `rlib`，生成入口通过 Cargo 依赖别名引用它，并负责最终 `cdylib` 的 JNI 符号和注册函数。先保留现有宿主 `[lib]` 名称，验证 Rust 依赖别名与 JNI 链接。改造 [Gradle Cargo 任务](../crates/embedder/android/apk/app/build.gradle.kts) 的 manifest、包名、输入追踪与各 ABI 输出定位，现有 Kotlin 继续加载 `libimpasto.so`。[Cargo 依赖别名](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#renaming-dependencies-in-cargotoml) |

原生 ABI 入口由最终应用入口确保持有和导出。链接验收须检查 Swift/C、JNI 所需符号和 Release 下的注册引用。第二阶段交付 macOS App、iOS 真机或模拟器 App、Android APK，其他分发命令（包括 AAR、AAB、iOS Archive、IPA、XCFramework）按接入范围逐个验收，未接入时对插件构建明确报错。

### 5.3 增量构建与重载

- 修改应用脚本或插件 TS 包装层且原生构建指纹一致时，Android 调试和 iOS `run --watch` 复用现有脚本更新路径，使用已安装目标的描述清单。macOS 当前通过重启刷新，可复用已链接的宿主产物。
- 原生指纹包含插件选择、Cargo 清单和锁文件、工具链与 feature、Rust 和平台源码、应用平台配置以及链接输入。目录监听覆盖已解析的外部插件和本地 Cargo 路径依赖，忽略生成目录。资源变更沿用现有平台资源重打包规则。
- 原生指纹变化重新导出声明、增量构建并重新部署，构建失败保留现有运行版本。只有在部署确认后更新安装指纹。模块移除后，旧声明、解析映射与平台源文件不得残留在新产物中。

## 6. 平台插件验收场景

相机与扫码用于验证工程合并、系统权限、资源释放和高吞吐帧通路。插件在 Rust 层管理会话状态与扫码，Swift/Kotlin 层负责系统相机采集与交互，Solid 层显示画面并消费扫码流。

- **平台资源**。将 `Activity`、`UIViewController` 或 `NSWindow` 的访问限制到平台主线程回调内。跨线程保留宿主签发的可发送句柄，回调时重新获取当前对象，并在宿主分离后返回不可用。暂停与隐藏时停止采集，恢复时重新检查权限和会话状态，关闭时释放订阅、采集、流与纹理。
- **预览**。使用现有 [`ExternalTexture`](../packages/solid-ui/src/ExternalTexture.tsx) 和 [外部纹理命令](../crates/embedder/common/src/command.rs)。先开放可移交所有权的 RGBA 路径，后续由 embedder 提供平台帧导入适配器，SDK 不公开 `raster::GpuImage` 或 `Rasterizer`。由宿主分配带会话范围的纹理键，用有界的最新帧策略替换过期帧。导入适配器需持有生产缓冲直到 GPU 消费结束，并处理格式、同步 fence、设备更换及不支持设备的回退。Android 可复用 [`HardwareFrame`](../crates/embedder/android/src/app/hardware_frame.rs) 的检测与回退模式，iOS 可复用现有 Metal 导入入口。只有在实测走平台导入且未拷贝像素的设备组合上标记为零拷贝。
- **事件与拍照**。扫码结果通过 `ReadableStream` 传输，JS 取消和对象关闭都停止生产。拍照数据走有界二进制数据通路，大文件可返回应用内路径。TS 包装层在组件 owner 内同步注册 cleanup，异步打开在组件销毁后才完成时立即关闭句柄。
- **原生视图**。地图与 WebView 需要单独定义工厂注册、实例所有权、布局、输入与无障碍。[现有 `Services::platform_views`](../crates/embedder/common/src/lib.rs) 只交付布局结果，工厂注册列为后续扩展。

## 7. 实施顺序与验收

| 阶段 | 交付 | 通过条件 |
| --- | --- | --- |
| 1. 宿主契约与宏 | 轻量 SDK、显式注册、一方权限与扩展装配分开。支持数据派生、同步、Promise 和模块流。以小型一方模块或测试模块验证描述生成。 | `just boundaries`、`just check`、`cargo fmt --all -- --check` 通过。现有一方声明校验通过。测试覆盖合法导出、重复或越权占用、数值越界、可选值、大小二进制 payload、流背压与取消。 |
| 2. 本地业务插件 | `[native]`、按目标生成描述与平台入口、异步对象、应用打包及分级更新。接入 AppPaths 和异步 HTTP 适配。 | 同一个应用在 macOS、iOS 模拟器和 Android 上通过 Debug 与 Release 的 ESM、Promise、对象和流验收。检查二次关闭、取消、迟到完成、脚本切换、静态导出错拼、删除插件、主机与目标描述不一致、一个应用连续构建多个目标。复跑无插件 Gallery 的构建与脚本更新路径。 |
| 3. 平台插件 | 路径型第三方插件、工程清单合并、平台主线程上下文与相机参考插件。CPU 预览先通过，按设备扩展 GPU 导入。 | 应用只通过清单接入插件即可编译与打包。至少用 iOS 和 Android 真机验证授权、前后台、连续打开关闭、扫码流和预览回退，验证已支持设备的帧租约在 GPU 完成后释放。macOS 支持单独按宿主能力验收。 |

### 验收应用

`apps/native-order` 覆盖 `[native]`、异步工厂、对象方法、可取消流与宿主 HTTP 注入。移动端参考应用 `apps/native-camera` 覆盖第三方清单、Swift/Kotlin 编译、权限请求、外部纹理、Rust QR 解码和 JPEG 编码。

从仓库根执行 `cargo build --locked -p cli`。在验收应用目录运行 `../../target/debug/impasto analyze --platform ios` 或 `../../target/debug/impasto build bundle --platform android`。订单应用可用 `build macos`、`build ios --ios-simulator` 或 `build apk` 验证 Release，附加 `--debug` 切换到 Debug。订单日志 `native-order: 1 order · order-1 · 2599¢` 表示异步工厂、对象查询和流首项都已返回。相机授权、前后台切换及 GPU 帧租约按设备验收。

修改涉及生成器或一方模块时补跑 `cargo run --locked -p embedder --bin module-stubs -- --check`、`node tools/check-native-module-types.mjs`。平台接入按 [`justfile`](../justfile) 的 macOS、iOS 和 Android 配方验证宿主与生成产物。
