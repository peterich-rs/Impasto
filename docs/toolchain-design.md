# 把脚手架和场景写入收进 Rust

应用侧命令对齐 Flutter。视图树继续只活在 `uicore::Tree`，这条对齐 nsh 的 core，而不是对齐 Flutter 的 Element。响应式图按 nsh 的 arena 另起一个 crate，调度对齐仓库已经依赖的 Solid `2.0.0`，不移植它的源码。

## Breaking Change Notice

工作区内部的包名和全局对象会变。`publish = false`（`Cargo.toml` 第 17 行），没有 crates.io 下游。会破的是本仓库的 gallery、`packages/*` 和还在调用 `just host` 的脚本。

1. `import … from "solid-js"` 保持不动。解析改到垫片 `@impasto/reactive`。应用代码不改 import。
2. 删掉 `flush()`。帧内由 shell 调用 `reactive::run_updates`。应用代码里如果有人直接 `flush()`，改成什么都不调用；测试里要同步时改调 native `batch`。
3. `@impasto/<pkg>/host` 改成包入口：`@impasto/clipboard`、`@impasto/linking`、`@impasto/file-system`、`@impasto/async-storage`、`@impasto/app-state`、`@impasto/permissions`、`@impasto/devtools`。`host.gen.ts` 按能力改名，例如 `clipboard.ts`。
4. 全局 `core` 取消。QuickJS 注册具名模块 `scene`、`reactive`。`core._mutate` 不再是脚本能看见的对象。
5. crate `hostshell` 改名 `shell`。Rust 调用方把 `hostshell::` 换成 `shell::`。
6. `HostNode` 改名 `Node`。JSX 的 `Element` 仍是这一类型，只是名字变了。

## Feasibility Assessment

做得到，有三处边界。场景树已经在 Rust：`uicore::Tree`（`crates/uicore/src/tree/mod.rs:52`）持有槽位、代数、Taffy、命中和弹簧，`NodeId` 已是槽位加 generation（`crates/uicore/src/tree/node.rs:15`）。QuickJS 使用 Cargo 钉住的 quickjs-jit 0.14.2；`linked/rt` 的 Rust 绑定把参数交给现有 `mutate` / `apply_now`，帧末由 `flush_mutations` 排空队列。`Runtime` 与 `Context` 由 `Engine` 持有，启动失败和 Drop 共用显式 shutdown，先释放图、定时器与 Persistent，再释放 Context 与 Runtime。nsh 的 `Graph`（`nsh/crates/nsh-core/src/reactive.rs:1`）证明 arena、双向边、值留在运行时里这条路能撑住 Solid 的调度；Impasto 要对齐的是 `2.0.0` 的上述 API，不是 nsh 写下的 Solid 1.9，也不把 nsh 的 `Host` 名字带过来。iOS 宿主不存在（`docs/refactor-plan.md:33`），`impasto build ios` 在 `crates/ios` 落地前只能生成工程、不能打出安装包。打包器换成 Rust 是后续的事，这条链路先把 Babel 和 esbuild 编进 CLI 发布包。

Feasible with caveats: iOS 包要等宿主；应用机器不装 Node 要等 CLI 把打包器打进发布物；阶段 2 完成前不能删 `solid-js`。

## Current Surface Inventory

作者实际从 `"solid-js"` 引入、并且这条链路要保住签名的：

- `createSignal` — 返回 `[get, set]`。gallery 状态和 `packages/solid-ui`、`packages/form`、`packages/solid-router` 都在用。
- `createMemo` — `apps/gallery-solid/src/pages/market.tsx`、`watchlist.tsx`。
- `createRenderEffect` — 行情页、`runtime/text.ts`、`runtime/image.ts`。
- `createTrackedEffect` — gallery 各页、`packages/solid-ui` 的控件。
- `onCleanup`、`untrack`、`createRoot`、`getOwner`、`runWithOwner` — `runtime/slot.ts`、`packages/solid-router/src/stack.ts`。
- `createContext`、`useContext` — `packages/solid-router/src/stack.ts`，以及 `packages/solid-ui/src/overlay/OverlayRoot.tsx`。
- `children` — 只有 `runtime/text.ts`。`runtime/slot.ts` 用的是 `createRoot` / `runWithOwner` / `untrack`，不是 `children`。
- `Show`、`For`、`Errored` — devtools、路由、`VirtualList`、`Tabs`。`Switch` / `Match` 没有业务文件直接导入；Babel 的 built-in 从 moduleName 引入，所以 `runtime/index.ts:180` 仍要再导出。同文件的 `Dynamic`（182 行）留在渲染器里，不进 native 清单。
- `flush` — 只有 `boot` 和测试。`batch` 没有人从 `solid-js` 导入。`runtime/motion.ts` 里的 `batch` 是局部数组，不是 Solid 的 `batch`。
- 类型 `Accessor`、`JSX` — `packages/form`、`packages/devtools`、`packages/solid-renderer/src/jsx.ts:58`。

场景桥，今天全在 JS：

- `apps/gallery-solid/build.mjs` 使用 `tools/pack.mjs`。Babel 以 `moduleName: "@impasto/solid-renderer"` 和 `generate: "universal"` 编译 TSX。esbuild 将 `solid-js` 解析到 `@impasto/reactive`，并检查 `dist/quotes.js` 是否混入 Solid 调度器或 `@solidjs/universal`。
- `packages/solid-renderer/runtime/index.ts:17` — `createRenderer<HostNode>`，导出 `insert`、`setProp`、`effect`、`memo`。
- `packages/solid-renderer/src/tree.ts:11` — `HostNode` 是 `{ id }`。裸 number 会被 universal 当成文本。
- `packages/solid-renderer/src/tree.ts:166` — `hostRendererOptions` 按属性名分发，最终进 `CoreHost`。
- `packages/solid-renderer/runtime/host.ts:8` — 读全局 `core`。
- `crates/quickjs/src/scene.js` — `core._mutate(op, …)`。`setGap` 这类具名方法写在这份 JS 里。
- `crates/uicore/src/tree/layout.rs` 的 `Tree::set_gap` — 真正的写入。编号在 `crates/quickjs/abi.yml`。

帧。`flush()` 不在 `shell.rs`，在 QuickJS 会话里，而且有两次：

- `crates/hostshell/src/shell.rs:719` — `on_frame` 把指针交给 session，再 `session.frame`。它不调用 `flush`。
- `crates/quickjs/src/linked/mod.rs:787` — `JsSession::frame` 的顺序是：`tick_motions`（799 行，在任何 JS 冲刷之前）→ 手势 → job → 定时器 → rAF → `vm_frame`（858 行，即 `boot` 的 `flush()` 再 `runFrames`）→ `flush_mutations`（874 行）→ `layout` → driver tick → 可能再 `layout` → `vm_layout`（923 行，即 `runLayouts()` 再 `flush()`）→ `flush_mutations` → 脏了再 `layout` 一次 → `record_into`。
- `crates/uicore/src/tree/mod.rs:68` — 滚动摩擦和弹簧在 `motions`，`set_scroll_offset` 只标绘制（`tree/scroll.rs:52`）。
- `runtime/index.ts:152` 的 `export const core` 是渲染器里的辅助函数（`column` / `row` / `onFrame`），不再读取全局 `core`。两份都要退场，不能只删全局。

要改掉的名字：

- crate `hostshell`（`Cargo.toml:8`）。
- `crates/android/src/host.rs`。macOS 已经是 `crates/macos/src/app.rs`。
- `CoreHost`、`runtime/host.ts`、`HostNode`、`hostRendererOptions`。
- `HostModule`、`HostMethod`、`HostConfig`（`crates/session`、`crates/hostshell/src/modules`）。
- 七个包的 `"./host"` export 和 `src/host.gen.ts`。
- 全局 `core`。

仓库没有面向应用的命令。`just gallery-dist` 打包，`just host` 开窗口（`justfile:51`、`justfile:64`）。Android 走 `crates/android/apk` 的 Gradle。

## Design

两条参照，各取一块，不把另一块带过来。

Flutter 只贡献命令语法和产物目录的纪律：`create`、`run`、`build <target>`、`analyze`、`test`、`clean`、`packages get`；`run` 默认 debug，`build` 默认 release；设备用 `-d`；生成的平台工程放 `build/`，`clean` 删掉。不搬 Widget、Element、Skia 层、kernel 热重载。

nsh 只贡献所有权：视图节点在 Rust（`nsh_core::Core` 收 `Op`，`NodeId = u32`，`nsh/crates/nsh-core/src/ops.rs:16`）；响应式是一张带 generation 的 arena，边双向，值的类型由运行时提供，读的时候加引用，不序列化（`nsh/crates/nsh-core/src/reactive.rs:83` 的 `Host` trait）。不搬它的第二棵影子树，也不搬 `MountBatch`。nsh 把 core 的节点挂成 UIKit / Android View；Impasto 已经从 `Tree::record` 得到 `LayerTree`，再交给 Impeller。再插一层挂载会把像素一致性拆掉。

```text
┌────────────────────────────────────────────────────────┐
│ TSX   lib/main.tsx                                     │
│ createSignal / JSX，签名与现在的调用一致               │
└──────────────────────────┬─────────────────────────────┘
                           │ impasto run / build
                           ▼
┌────────────────────────────────────────────────────────┐
│ crates/cli                                             │
│ 擦类型 → @solidjs/babel-plugin → esbuild              │
│ "solid-js" 指到垫片，调度实现不进 bundle              │
└──────────────────────────┬─────────────────────────────┘
                           │ ESM（debug）或字节码（release）
                           ▼
┌──────────────────── QuickJS ───────────────────────────┐
│ 用户闭包：报价、路由、手势回调                         │
│ 模块 reactive ★              模块 scene ★              │
└─────────┬──────────────────────────┬───────────────────┘
          │ 订阅、标脏、排队         │ insert / setProp
          ▼                          ▼
┌──────────────────┐      ┌──────────────────────────────┐
│ crates/reactive  │      │ uicore::Tree   （现有）      │
│ arena，Value     │      │ Taffy、命中、滚动、弹簧      │
└──────────────────┘      └──────────────┬───────────────┘
                                         ▼
                            LayerTree → engine → Impeller
┌────────────────────────────────────────────────────────┐
│ shell ★  布局前 reactive::run_updates                 │
│ macos / android  窗口、输入、vsync。不做命中           │
└────────────────────────────────────────────────────────┘
```

现在的热路径，以及要剪掉的那一截：

```text
现在
  JSX → universal insert/setProp → HostNode{id}
      → tree.ts 按名字分发 → core.setGap
      → core.js → core._mutate → Tree::set_gap

目标
  JSX → native insert/setProp ★ → Tree::insert / Tree::set_gap
  core.js 的具名分发 ✂ 退出热路径
  boot 里的 flush() ✂
  shell 在 layout 前调用 run_updates ★
```

一帧。调用点是 `JsSession::frame`，不是 `shell.rs` 的 `on_frame`。`on_frame` 只入队再调 `session.frame`。弹簧保持在第一次 `run_updates` 之前，和现在 `tick_motions` 先于 `vm_frame` 一样。两次冲刷都留着：第一次在帧监听之前，第二次在 layout 监听之后、可能的再布局之前。监听器仍然只写 signal。

```text
shell.on_frame
    │ session.frame
    ▼
JsSession::frame                         reactive              Tree
    │ tick_motions（现有，仍在最前）                             │
    ├──────────────────────────────────────────────────────────►│
    │ 手势、定时器、rAF（现有）
    │ run_updates ★  代替 onFrame 里的 flush()
    ├───────────────────────────►│
    │                            │ 闭包 setProp
    │                            ├─────────────────────────────►│
    │ onFrame 监听（现有，不再 flush）
    │ layout（现有）
    │ driver tick（现有）
    │ onLayout 监听（现有）
    │ run_updates ★  代替 onLayout 末尾的 flush()
    ├───────────────────────────►│
    │ 脏了再 layout 一次（现有）
    │ record_into（现有）
    ▼
LayerTree → engine → present
```

Legend: `──►` 同步调用，`★` 本设计新增。`tick_motions`、`layout`、`record_into` 是现有顺序，不搬到 `run_updates` 之后或之前。

### 关键决策

1. **命令语法用 Flutter，不用 nsh，也不自造第三套。** Flutter 是 `flutter create`、`flutter run -d`、`flutter build apk`、`flutter analyze`、`flutter test`、`flutter clean`、`flutter pub get`。nsh 是 `nsh init`、`nsh dev`、`nsh run --target`、`nsh build`（直接打字节码）、`nsh check`、`nsh package`。作者规格写的是 Flutter 那一列，所以 bin 的子命令就是那一列。否决 nsh 的 `dev` 和 `run` 分成两条：规格里只有 `run`。否决 nsh 的 `--target` 作为唯一开关：设备选择用 `-d`，平台是 `run -d android` 这种目标，也允许 `-d <设备号>`。

2. **debug 走 ESM，release 走字节码。这是规格，不是 nsh 的现状。** nsh 的 `run` 和 `build` 都打 QuickJS 字节码；ESM 只在字节码版本对不上、或者目标是 web 时才走。Impasto 的 `run` 默认 debug，产物 ESM，QuickJS 直接执行；`build` 默认 release，产物字节码。`--debug` 和 `--release` 覆盖默认值。否决在第一期做 nsh 那种 8790 端口和 `bytecode_version` 回退：`run` 只做改文件再打包再装，和现在的 `build.mjs --watch` 加重启窗口同一级。状态保留的热更新以后再说。nsh 没有 `clean`，生成物放 `.nsh/` 且不自动删。这里用 Flutter 的 `clean`，删应用的 `build/`。

3. **不新建第二棵视图树。** nsh 的 `Core` 是影子树，平台视图才是画出来的那棵，帧末交 `MountBatch`。Impasto 的 `Tree` 已经是画出来的那棵：槽位、代数、Taffy、picture、弹簧都在上面。`insert` / `setProp` 直接调 `Tree` 的方法。否决把 `MountBatch` 或 Flutter 的 Element/RenderObject 再夹一层。`{ id: number }` 这个包装只为了骗过 `@solidjs/universal`（`tree.ts:8`）。universal 删掉之后，JS 拿 `NodeId::to_raw` 的 `u64`。id 只由 Rust 分配，槽位回收时 generation 加一，这是现在的 `NodeId`。否决 nsh 的两套号：JS 发偶数、core 发奇数，以及 `NodeId = u32` 只增不回收、再配一张 `IdTable`。那是因为 nsh 的 id 由 JS 渲染器分配、等 GC 才 `Release`。Impasto 没有第二条分配器。

4. **响应式图复制 nsh 的结构，不复制 `Host` 这个名字，也不复制 Solid 的源码。** 新 crate `reactive`。节点在 arena 里，槽位复用时 generation 加一，依赖是双向整数边，解开用 swap-remove。图不持有跨进用户闭包的借用：闭包可能重入。值类型叫 `Value`。QuickJS 实现把 `Persistent<Value>` 存在节点里，读取时克隆 Persistent，不把对象序列化成 JSON。trait 叫 `Eval`，对应 nsh 的 `Host`（`reactive.rs:83`），方法是 `dup`、`free`、`call`、`same`。否决继续把 `solid-js` 的调度打进 bundle：那是现状，订阅和标脏留在 JS。否决把 nsh 的实现搬过来当依赖：它的队列对齐 Solid 1.9，本仓库测的是 `2.0.0` 里上面那张 API 清单。

5. **滚动偏移和弹簧留在 `uicore` 的 `motions`，并且仍在渲染线程上、仍在 `run_updates` 之前采样。** 现在 `tick_motions` 是 `JsSession::frame` 的第一步（`linked/mod.rs:799`），`set_scroll_offset` 只标绘制。监听器可以读 offset、写别的 signal。禁止每帧 `set()` 一个滚动 signal。否决 nsh 的另一半：它的 offset 由平台视图报进 core，弹簧在 UI 线程的第二张动画图上采样。Impasto 的滚动是 `Tree` 自己算的，没有 UIKit 滚动视图。把弹簧搬去平台线程会拆掉「macos / android 只提供窗口、输入、帧」。`For` 只服务短列表。可见集继续由 `ListDriver` 决定，不把 nsh 的 `Each` 行协调搬进 `reactive`。

6. **控制流组件留成薄 JS。** `Show`、`Switch`、`Match`、`For`、`Errored` 只调用 native 的 `createMemo`、`createRenderEffect`、`createSignal`。它们不进 Rust。`Switch` / `Match` 今天没有业务文件导入，但 Babel 的 built-in 从 moduleName 取这些名字（`runtime/index.ts:179`），垫片必须继续导出。`Dynamic` 留在渲染器，不进这张清单。否决把 JSX 控制流编进 `reactive`：Babel 已经会把它们编成这些调用，再下沉一次没有新的语义。

7. **打包器这一期是 CLI 调用的 Node 工具，不是 oxc。** nsh 的 `nsh-bundler` 用 oxc 自己降 JSX（`nsh/crates/nsh-bundler/src/jsx.rs:1`）。规格写明 Rust 打包器是后续，不挡链路。阶段 1 在框架仓库里继续调现有 `apps/gallery-solid/build.mjs`。面向应用的发布包把 Babel、`@solidjs/babel-plugin` 和 esbuild 打进去，应用机器不装 Node。否决阶段 1 就换成 oxc：Solid universal 的降级规则要以现有插件为准，换编译器会改 gallery 的语义。

8. **`solid-js` 用 esbuild alias 指到垫片，不把垫片的 `package.json` name 写成 `solid-js`。** 垫片包名是 `@impasto/reactive`。CLI 打包时 `alias: { "solid-js": "<垫片路径>" }`。垫片只 re-export native 函数和官方类型声明。否决把 name 字段改成 `solid-js`：pnpm workspace 里会和真包撞名，阶段 2 又还不能删真包。

9. **垫片和旧图不能订阅同一个 signal。** 阶段 2 里 gallery 的信号全部走新图，`solid-js` 的实现不进那份 bundle。测试可以两边各起一张图，不能在同一次 `flush` 里交叉订阅。阶段 2 完成前不删依赖。阶段 3 的 renderer 测试先两边跑，再删 `createRenderer` 和 `@solidjs/universal`。

10. **具名模块，没有全局 `core`，也没有 `host` 模块。** nsh 的渲染器是原生模块（`nsh:rt`）。Flutter 的 `dart:ui` 也是具名库。Impasto 注册 `scene` 和 `reactive`。能力模块是 `clipboard`、`files`、`linking`、`storage`。嵌入外部应用的通道现在不存在，不预留 `host` 这个名字。否决留一个空的 `host` 模块等以后用。

11. **编号留在 `abi.yml`，TS 不再手写逐属性桥。** `setProp` 在 Rust 里把属性名映射到现有 opcode，再调 `Tree::set_gap` 这类方法。`core.js` 里的 `setGap` 包装退出热路径。否决在 TS 再生成一套 `setGap`。否决换一套新 opcode：`abi.yml` 已经是变更缓冲。

12. **平台工程是生成物。** `impasto create` 只写 `app.toml`、`package.json`、`lib/main.tsx`、`assets/`。不生成可提交的 `android/`、`macos/`。`build apk` 在 `build/` 里生成 Gradle 工程并链引擎。`clean` 删 `build/`。框架仓库的 `justfile` 留下，只跑 `cargo test`、取 QuickJS 和 Impeller SDK。否决把 justfile 拷进应用模板：那是框架的工具，不是应用的。nsh 把元数据放 `nsh.json`，这里用规格里的 `app.toml`。

13. **crate、类型、文件、TS 模块里不出现 host。** 对照表见下面的改名。nsh 自己大量使用 host（`nsh-macos-host`、`element/host.rs`、`HostLimits`），这一条明确不跟。`CoreHost` 删掉，场景操作进 `scene.ts`，不留一个改了名的同形类。

### 命令

| 命令 | 行为 | 对齐 |
|---|---|---|
| `impasto create my-app` | 写 `app.toml`、`package.json`、`lib/main.tsx`、`assets/` | Flutter `create` 的目录纪律；不做 nsh 的 rust/mixed 模板 |
| `impasto run` | debug 打包，装到本机桌面窗口，改文件再打包 | Flutter `run` 的 debug 默认；热更新只到「再打包」 |
| `impasto run -d android` | `-d` 选目标或设备 | Flutter `-d` |
| `impasto build bundle` | 只产字节码 | Flutter `build bundle` 的「只出产物」 |
| `impasto build apk` / `ios` / `macos` | 在 `build/` 生成工程，链引擎，打安装包 | Flutter `build apk` |
| `impasto analyze` | 类型检查 | Flutter `analyze`；实现上可以调 `tsc`，名字不用 nsh 的 `check` |
| `impasto test` | 无头跑测试 | Flutter `test` |
| `impasto clean` | 删 `build/` 和打包产物 | Flutter `clean` |
| `impasto packages get` | 调 `pnpm install` | Flutter `pub get`。不自造包管理器 |

`run` 默认 `--debug`。`build` 默认 `--release`。两个旗标互相覆盖默认值。

`create` 出来的目录：

```text
my-app/
  app.toml       标识、版本、目标、权限
  package.json
  lib/main.tsx
  assets/
```

### 类型

`crates/reactive`。trait 对应 nsh 的 `Host`，名字换成 `Eval`。

```rust
pub const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Signal,
    Memo,
    RenderEffect,
    TrackedEffect,
    Root,
    Free,
}

pub struct Node<V, F, C> {
    pub kind: Kind,
    pub generation: u32,
    pub value: Option<V>,
    pub func: Option<F>,
    pub owner: u32,
    /// 订阅谁，以及对方 sources 里的下标，解开时 swap-remove。
    pub sources: Vec<Edge>,
    pub observers: Vec<Edge>,
    pub cleanups: Vec<C>,
}

pub struct Edge {
    pub node: u32,
    pub index: u32,
}

/// 图存的值、跑的闭包、怎么比较。QuickJS 的 `V` 持有 `Persistent<Value>`。
///
/// 实现必须保证：`graph` 交出的借用在下一次进入用户闭包之前结束。
pub trait Eval: Copy + 'static {
    type Value;
    type Func;
    type Cleanup;

    fn dup(self, value: &Self::Value) -> Self::Value;
    fn free(self, value: Self::Value);
    fn call(self, func: &Self::Func, prev: Self::Value) -> Option<Self::Value>;
    fn same(self, a: &Self::Value, b: &Self::Value) -> bool;
    fn run_cleanup(self, cleanup: Self::Cleanup);
}

pub struct Graph<E: Eval> {
    nodes: Vec<Node<E::Value, E::Func, E::Cleanup>>,
    free: Vec<u32>,
}

impl<E: Eval> Graph<E> {
    /// 帧内由 shell 调用。JS 不再 `flush()`。
    pub fn run_updates(&mut self, eval: E);

    pub fn batch(&mut self, eval: E, scope: impl FnOnce());
}
```

native 导出，签名与作者现在的调用一致。QuickJS 模块名 `reactive`：

- `createSignal` 返回 `[get, set]`
- `createMemo`、`createRenderEffect`、`createTrackedEffect`
- `createRoot`、`getOwner`、`runWithOwner`、`onCleanup`
- `untrack`、`batch`（对外名字。壳调用的是 `Graph::run_updates`）
- `createContext`、`useContext`
- `children`

`scene` 模块导出 Babel 已经在生成的那些函数，改成 native：

- `insert`、`setProp`、`createElement`、`effect`、`memo`、`createComponent`

`effect` 和 `memo` 是渲染器原语，实现落到 `reactive` 的 render effect 和 memo，再把属性写进 `Tree`。它们不是第二张图。

垫片 `packages/reactive/src/index.ts`：

```ts
export {
  createSignal,
  createMemo,
  createRenderEffect,
  createTrackedEffect,
  createRoot,
  getOwner,
  runWithOwner,
  onCleanup,
  untrack,
  batch,
  createContext,
  useContext,
  children,
} from "#native/reactive";

export { Show, Switch, Match, For, Errored } from "./flow.ts";
export type { Accessor, JSX } from "./types.ts";
```

`flow.ts` 是薄封装。`types.ts` 只放官方声明，不放运行时。

`app.toml`：

```toml
name = "my-app"
version = "0.1.0"
targets = ["macos", "android"]

[permissions]
# 能力模块按目标打开。缺省全关。
```

### 改名

| 现在 | 改成 |
|---|---|
| `hostshell` | `shell` |
| `crates/android/src/host.rs` | `app.rs` |
| `CoreHost`、`runtime/host.ts` | 删除。场景操作进 `scene.ts` |
| `HostNode` | `Node` |
| `hostRendererOptions` | `sceneRenderer` |
| `HostModule`、`HostMethod`、`HostConfig` | `Module`、`Method`、`ModuleConfig` |
| `@impasto/<pkg>/host`、`host.gen.ts` | 包入口。`clipboard.ts` 这种按能力命名 |
| 全局 `core` | 取消。模块 `scene`、`reactive` |

`engine`、`protocol`、`uicore`、`session`、`quickjs`、`macos`、`android` 保留。新文件用 `reactive.rs`、`scene.rs`、`clipboard.rs`。不用 `host_*.rs`。公开 TS 模块：`reactive`、`scene`、`clipboard`、`files`、`linking`、`storage`。

### 应用怎么写

作者侧不换 API：

```tsx
import { createSignal, Show } from "solid-js";

export default function Main() {
  const [symbol, setSymbol] = createSignal("AAPL");
  return (
    <column gap={8}>
      <Show when={symbol()}>{symbol()}</Show>
    </column>
  );
}
```

打包之后 `"solid-js"` 是垫片，`<column>` 仍由 `@impasto/solid-renderer` 接收，`insert` / `setProp` 进 `uicore::Tree`。滚动不写 signal：

```tsx
<scroll axis="vertical" onScroll={(x, y) => setOffsetLabel(`${y}`)}>
  {rows()}
</scroll>
```

`onScroll` 可以写一个给文字用的 signal。视口偏移本身留在 `Scroll` 节点的弹簧和 offset 上。

## Phase 1: 命令骨架

仓库在这一阶段之后仍能 `cargo test`，gallery 仍能打开。`impasto run` 的行为和 `just gallery-dist` 再 `just host` 相同。打包仍走现有 Node 脚本。不改渲染，不改 signal。

**File: `crates/cli/Cargo.toml`**（新）

- package 名 `cli`，bin 名 `impasto`。
- 依赖 `clap`。这一阶段不依赖 `uicore` 或 `quickjs`。

**File: `crates/cli/src/main.rs`**（新）

- 子命令就是设计一节那张表。
- `run` 调 `apps/gallery-solid/build.mjs`，再 `cargo run -p macos -- --js <bundle>`。工作目录是框架仓库时，入口用 gallery，用来保住「还能打开」。工作目录是应用（有 `app.toml`）时，入口用 `lib/main.tsx`，打包脚本参数化，不复制第二份 Babel 配置。
- `run -d android` 调现有 `crates/android/apk` 的 `assembleDebug`。
- `packages get` 执行 `pnpm install`，参数原样转交。
- `create` 只写四个路径。模板里的 `lib/main.tsx` 是一个空 `column`。
- `build`、`analyze`、`test`、`clean` 这一阶段可以打印未实现并返回非零，但 clap 必须认得。不要假装打出了 APK。

**File: `Cargo.toml`**

- `members` 加上 `crates/cli`。

**File: `justfile`**

- 保留 `gallery-dist`、`host`、`test`。JavaScript 解释器由 Cargo 的 quickjs-jit 0.14.2 构建。
- 不把 justfile 写进 `create` 的模板。

**File: `apps/gallery-solid/build.mjs`**

- 这一阶段不改行为。CLI 调用它。

## Phase 2: 响应式图

`solid-js` 依赖还在，只是 gallery 的 bundle 不再把它的调度打进去。画面不变。

**File: `crates/reactive/Cargo.toml`**（新）

- 无 QuickJS 依赖。图是纯 Rust。workspace `members` 加上它。

**File: `crates/reactive/src/lib.rs`**（新）

- 放设计一节的 `Graph`、`Eval`、`Kind`、`Edge`。
- `run_updates` 的队列顺序按 `2.0.0` 测出来的行为写，不按 nsh 的 Solid 1.9 注释写。
- 闭包重入时不能持有 `nodes` 的借用。和 nsh 一样：进 `Eval::call` 之前把值从节点里取出。

**File: `crates/reactive/tests/solid_2_0.rs`**（新）

- 用一个 Rust `Eval`（值是 `i32` / 字符串，不是 `JSValue`）覆盖清单里的语义：`createSignal` 的读写、memo 缓存、render effect 先于 tracked effect、`untrack` 不订阅、`batch` 合并通知、`onCleanup` 在 owner 销毁和 effect 重跑时触发、`createRoot` 销毁子节点、`getOwner` / `runWithOwner`、`createContext` / `useContext`、`children` 的扁平化。
- 对照物是同一版本 Solid 的可观察行为，用小脚本在测试里跑一次 Node，把结果钉成夹具。不把 Solid 源码拷进 crate。

**File: `crates/quickjs/src/linked/reactive.rs`**（新）

- `Eval` 的 QuickJS 实现。`Value` 持有 rquickjs 的 `Persistent`，`dup` 克隆引用，`free` 释放引用；销毁图必须先于 VM。
- 注册模块 `reactive`，导出设计一节的函数。`batch` 调 `Graph::run_updates`。

**File: `crates/quickjs/src/lib.rs`**

- 会话启动时注册 `reactive`。这一阶段 `core` 还在，场景桥还没动。

**File: `packages/reactive/package.json`**（新）

- name 是 `@impasto/reactive`。不要写成 `solid-js`。

**File: `packages/reactive/src/index.ts`**、**`flow.ts`**、**`types.ts`**（新）

- 见设计一节。`Show`、`Switch`、`Match`、`For`、`Errored` 放 `flow.ts`。

**File: `apps/gallery-solid/build.mjs`**

- esbuild `alias` 把 `"solid-js"` 指到 `packages/reactive`。
- 确认产物里没有 `solid-js` 的调度实现。有，则构建失败。
- `@solidjs/universal` 和 `insert` 这一阶段不动。

**File: `packages/solid-renderer/runtime/index.ts`**

- `boot` 仍调 `flush`。阶段 2 的 `flush` 来自垫片，垫片的 `flush` 就是 native `batch`。不能一边留 Solid 的图、一边让垫片的 signal 被 Solid 的 effect 订阅。

## Phase 3: 场景 native

`insert`、`setProp` 直接进 `uicore`。删掉 `createRenderer` 和 JS 的 `flush`。renderer 测试先两边跑，再删旧路径。

**File: `crates/quickjs/src/linked/rt/renderer.rs`**（新）

- 模块 `scene`。`createElement` 调 `Tree` 的创建。`insert` 调插入。`setProp` 用 `abi.yml` 的字段号找到现有的 `Tree::set_*`，不经过 `core.js`。
- 返回给 JS 的是 `NodeId::to_raw()`。JS 不再为了 universal 包一层 `{ id }`。

**File: `packages/solid-renderer/runtime/index.ts`**

- 删 `createRenderer`、`import { flush } from "solid-js"`、`host.onFrame` 里的 `flush()`、`host.onLayout` 里的 `flush()`。
- `insert`、`setProp`、`effect`、`memo`、`createComponent`、`createElement` 改从 `scene` native 导出。
- `boot` 仍注册帧监听和 layout 监听，只是不再冲刷图。两次冲刷改在 `JsSession::frame`。
- 152 行的 `export const core` 删掉，或者改成不叫 `core` 的普通函数。它和全局 `core` 不是同一个对象，但名字会让下一阶段的清扫漏掉。调用方改直接用 `scene`。

**File: `packages/solid-renderer/src/tree.ts`**

- 删属性名分发和 `hostRendererOptions`。保留标签类型需要的常量可以挪到 `scene.ts`。JS 不再为每个属性写一个桥。

**File: `packages/solid-renderer/runtime/host.ts`**

- 删除。`CoreHost` 的场景方法没有了。还没改名的能力调用先临时从模块 `scene` 以外的现有入口走，阶段 4 再收。

**File: `crates/quickjs/src/scene.js`**

- 具名 `setGap` 一类退出热路径。`_mutate` 如果 Rust 侧已经直接调 `Tree`，这份包装不再被 `setProp` 调用。编号表 `abi.yml` 留下。

**File: `crates/quickjs/src/linked/mod.rs`**（`JsSession::frame`，787 行起）

- 不在 `shell.rs` 的 `on_frame` 里刷图。`on_frame` 仍然只是入队和调用 `session.frame`。
- 第一次 `run_updates` 放在现在 `vm_frame` 的位置（858 行）之前，也就是 `tick_motions`、手势、定时器、rAF 之后。这代替 `boot` 在 `onFrame` 开头调的 `flush()`。
- 第二次放在 `vm_layout`（923 行）返回之后、`flush_mutations` 和「脏了再 layout」之前。这代替 `boot` 在 `onLayout` 末尾调的 `flush()`。
- `tick_motions`（799 行）保持第一步。不要把它移到 `run_updates` 之后，也不要把它挪进 `reactive`。
- 监听器仍然只写 signal。`vm_frame` / `vm_layout` 继续跑现有的帧监听和 layout 监听，只是回调里不再进入图。

**File: `packages/solid-renderer/package.json`**

- 去掉 `@solidjs/universal`。

**File: `apps/gallery-solid/package.json`**

- 去掉 `@solidjs/universal`。`solid-js` 依赖要等阶段 2 的测试还需要对照时再留；bundle 已经不包含它。阶段 3 结束时从应用依赖里删掉。

## Phase 4: 改名

只改名字，不改行为。每一处改完，`cargo test` 和 gallery 仍过。

**File: `crates/hostshell/**`**

- 目录改成 `crates/shell`。`Cargo.toml` 的 package 名改成 `shell`。
- 根 `Cargo.toml` 的 member 和 `workspace.dependencies` 跟着改。
- `crates/hostshell/src/bin/host-stubs.rs` 改成按模块生成的名字，不再带 host。`just gen` 的 `-p hostshell --bin host-stubs` 跟着改。

**File: `crates/android/src/host.rs`**

- 改名 `app.rs`。`lib.rs` 的 `mod` 跟着改。与 `crates/macos/src/app.rs` 同级。

**File: `crates/session/src/modules.rs`**、**`crates/hostshell/src/modules/*.rs`**

- `HostModule`、`HostMethod`、`HostConfig` 改成 `Module`、`Method`、`ModuleConfig`。

**File: `packages/solid-renderer/src/tree.ts`**、**`src/jsx.ts`**、**`src/host.ts`**

- `HostNode` 改成 `Node`。`JSX.Element` 仍是它。
- `hostRendererOptions` 若阶段 3 已删，这里只改残留引用。否则改名 `sceneRenderer` 再删。
- `CoreHost` 不留别名。

**File: `packages/clipboard/package.json`** 以及 `linking`、`file-system`、`async-storage`、`app-state`、`permissions`、`devtools`

- 删掉 `"./host"` export。入口就是包自己。
- `src/host.gen.ts` 改成按能力命名的文件：`clipboard.ts`、`linking.ts`、`files.ts`、`storage.ts`。生成器跟着改。

**File: `apps/gallery-solid/build.mjs`**

- 删掉扫描 `exports["./host"]` 的 external 列表（第 9–20 行）。能力模块改由 QuickJS 注册，不再是 esbuild external 的 `@impasto/<pkg>/host`。
- linking 的动态 import 改写（第 49–52 行）改成 `import("@impasto/linking")`，不再拼 `/host`。

**File: `docs/architecture.md`**、**`README.md`**、**`justfile`**

- 把 `hostshell`、`HostActivity` 的描述改成改名后的路径。`HostActivity` 是 Android 框架类名的话留在 Kotlin 侧另说；Rust 文件不再叫 host。这一阶段只改仓库自己的名字。

公开文档和日志 target 里的 `impasto::host`（`shell.rs:745`）改成 `impasto::shell` 或 `impasto::input`。不留 host 前缀。

## Phase 5: 发布构建

**File: `crates/cli/src/build.rs`**（新）

- `build bundle`：release 打包，再把 ESM 交给 `quickjs::compile` 生成 IQJS envelope 字节码。`--debug` 时仍写 ESM。
- `build apk`：在应用的 `build/android` 生成 Gradle 工程，链 `crates/android` 的引擎，跑 `assembleRelease`。逻辑从 `just android-release` 抬过来，路径改到应用目录，不假定框架仓库布局。
- `build macos`：同样在 `build/macos` 生成工程并链 `crates/macos`。
- `build ios`：只生成工程骨架时必须失败并写明 `crates/ios` 不存在。不要打出一个不能安装的包还返回零。`docs/refactor-plan.md` 的暂缓仍然有效。

**File: `crates/cli/src/toolchain.rs`**（新）

- 发布包内置 Babel、`@solidjs/babel-plugin`、esbuild 的可执行文件。应用机器没有 Node 时用这三份。
- 框架仓库里开发 CLI 时，允许退回本机 Node，方便改脚本。退回路径要在 `--verbose` 里打出来。

**File: `crates/cli/src/analyze.rs`**、**`test.rs`**、**`clean.rs`**

- `analyze` 跑 TypeScript 检查。应用没有 `tsc` 时用发布包里的检查器。
- `test` 无头：起 QuickJS，加载字节码或 ESM，不打开窗口。
- `clean` 删应用的 `build/` 和打包产物。不删 `lib/`、`assets/`、`app.toml`。

## Phase 6: 对照

不追时间指标。只要求次数下降。

**File: `crates/quickjs/src/lib.rs`**

- debug 计数：这一帧里 QuickJS 执行微秒、`run_updates` 次数、进入 `Tree::set_*` 的次数。`run` 的 `--trace` 打出来。不要默认打开。

**File: `docs/toolchain-baseline.md`**（新，跑完一次再写数字）

- 同一份 gallery。一次点击，一次列表滚动。
- 三列：阶段 2 之前的 bundle（`solid-js` 打进产物、JS `flush`）、阶段 3 之后。
- `run_updates`（或旧的 `flush`）次数和 `set_*` 次数必须下降。QuickJS 时间只记录。

滚动那一次如果次数没降，先查是不是有监听器在每帧 `set` 滚动 signal。那是违规，不是基线。

### 验证

每一阶段结束都跑：

- `cargo test --workspace`
- `just gallery-dist` 之后 `just host` 能打开 quotes（阶段 1 之后也可以是 `cargo run -p cli -- run`）
- 阶段 2 起，`crates/reactive` 的 Solid 对照测试过
- 阶段 3 起，bundle 里没有 `@solidjs/universal`，也没有 `solid-js` 的调度
- 阶段 4 起，`rg -n host crates packages apps --glob '!target/**' --glob '!node_modules/**'` 不再命中本设计点名的那些标识符。Android 框架类和第三方依赖里的 host 不算。

## Architectural Notes

- Semver：workspace `publish = false`。破的是仓库内的包路径、crate 名 `hostshell`、全局 `core`。迁移步骤在 Breaking Change Notice。
- 图和树都不是 `Send`。`Tree` 已经因为 Impeller 排版上下文留在渲染线程（`tree/mod.rs:49`）。`Graph` 同样只在这条线程上跑。不要为了测试把图放进 `Mutex`。
- `Eval::dup` / `free` 是引用计数，不是序列化。漏 `free` 会卡住 QuickJS 的 GC。节点复用前必须 `free` 掉旧 `Value`。
- `NodeId` 的 generation 继续由 `uicore` 管。`reactive` 的 generation 只管图节点。两套代数不要塞进同一个整数。
- 滚动、fling、越界弹簧继续在 `Tree::motions`。`run_updates` 不采样它们。
- 明确不动：`protocol` 的 `DrawList` / `LayerTree`、`engine` 的 Impeller、`ListDriver` 的可见集、Kotlin 路线（`docs/refactor-plan.md` 仍暂缓）、iOS 宿主、oxc 打包器、nsh 式热更新服务、`MountBatch`。样式对象不在这份方案里。
- 新依赖：`crates/cli` 用 `clap`。`crates/reactive` 不新增第三方依赖。应用运行时不再依赖 `solid-js` 和 `@solidjs/universal` 的实现；类型声明可以由垫片携带。阶段 2 完成前 `package.json` 里的 `solid-js` 版本号还留着，给对照测试用。
- 不把 nsh 当 crate 依赖引进来。结构和测试夹具是参照，代码不链接 `nsh-core`。

## File Change Summary

- `Cargo.toml` — workspace 加入 `cli`、`reactive`，阶段 4 把 `hostshell` 换成 `shell`
- `apps/gallery-solid/build.mjs` — 阶段 2 alias `solid-js`；阶段 4 去掉 `/host` external
- `apps/gallery-solid/package.json` — 阶段 3 去掉 `@solidjs/universal`，阶段 3 结束去掉 `solid-js` 实现依赖
- `crates/android/src/host.rs` — 阶段 4 改名 `app.rs`
- `crates/cli/Cargo.toml` — 新 bin `impasto`
- `crates/cli/src/analyze.rs` — 阶段 5 类型检查
- `crates/cli/src/build.rs` — 阶段 5 `build bundle` / `apk` / `macos`；`ios` 在宿主出现前失败
- `crates/cli/src/clean.rs` — 阶段 5 删 `build/`
- `crates/cli/src/main.rs` — 阶段 1 子命令；`run` 调现有打包和 `macos`
- `crates/cli/src/test.rs` — 阶段 5 无头测试
- `crates/cli/src/toolchain.rs` — 阶段 5 把 Babel 和 esbuild 打进发布包
- `crates/hostshell/**` — 阶段 4 整 crate 改名 `shell`。`on_frame` 不刷图
- `crates/quickjs/src/linked/mod.rs` — 阶段 3 在 `vm_frame` 前和 `vm_layout` 后各调用一次 `run_updates`；`tick_motions` 仍是第一步
- `crates/quickjs/src/scene.js` — 阶段 3 具名属性桥退出热路径
- `crates/quickjs/src/lib.rs` — 注册 `reactive`、`scene`；阶段 6 计数
- `crates/quickjs/src/linked/reactive.rs` — QuickJS 的 `Eval`，`Persistent` 持有 JS 值
- `crates/quickjs/src/linked/rt/renderer.rs` — native `insert` / `setProp` 直达 `Tree`
- `crates/reactive/Cargo.toml` — 新 crate
- `crates/reactive/src/lib.rs` — arena 图、`Eval`、`run_updates`
- `crates/reactive/tests/solid_2_0.rs` — 按 `2.0.0` 的可观察行为钉夹具
- `crates/session/src/modules.rs` — `HostModule` 等改名
- `docs/architecture.md` — 阶段 4 改 hostshell 的路径描述
- `docs/toolchain-baseline.md` — 阶段 6 写入点击和滚动的次数
- `justfile` — 保留框架用的 `gallery-dist` 和 `host`；`gen` 的 bin 名在阶段 4 跟着改
- `packages/clipboard/package.json` — 以及 linking、file-system、async-storage、app-state、permissions、devtools：删 `./host`
- `packages/reactive/package.json` — 垫片，name 是 `@impasto/reactive`
- `packages/reactive/src/flow.ts` — 薄的 `Show` / `Switch` / `Match` / `For` / `Errored`
- `packages/reactive/src/index.ts` — re-export native 和类型
- `packages/reactive/src/types.ts` — `Accessor`、`JSX` 声明
- `packages/solid-renderer/package.json` — 阶段 3 去掉 `@solidjs/universal`
- `packages/solid-renderer/runtime/host.ts` — 阶段 3 删除
- `packages/solid-renderer/runtime/index.ts` — 阶段 3 去掉 `createRenderer` 和 `flush()`
- `packages/solid-renderer/src/host.ts` — `CoreHost` 删除
- `packages/solid-renderer/src/jsx.ts` — `HostNode` 改名 `Node`
- `packages/solid-renderer/src/tree.ts` — 阶段 3 删 JS 属性分发
- `README.md` — 阶段 4 更新目录和 `just host` 的描述
