# 原生编译和无头测试工具链

应用使用 `app.toml` 声明依赖。框架包以源码随 SDK 分发，现有的纯 JavaScript 依赖可由 Rolldown 解析。链接器会报告 QuickJS 不支持的 Node.js 内置模块，以及链接后仍使用的浏览器全局变量，并标明原文件位置。

## 编译

Oxc 将 TSX 降级为 Impasto 渲染指令，再由 Rolldown 在内存中链接。正式构建生成 IQJS `app.bin` 和 `app.sym`。用 `impasto symbolicate <app.sym> <行> <列>` 查询 TSX 位置，或使用 `--stack <日志>` 批量还原 QuickJS 异常栈。原生 TypeScript 7 负责类型检查，工具链支持显式路径、SDK 内置编译器和经过 SHA-512 校验的下载缓存。

直接从 `@impasto/solid-ui` 导入的 View、Row、Column、Stack、Grid、RepaintBoundary 和 Text，以及从 `@impasto/solid-renderer` 导入的 Text，可以与宿主原语一起批量挂载。`Row`、`Column` 和 `Grid` 的预设处于组件样式层，类名和内联样式可以覆盖它们。静态语义属性在创建时提交，动态纯文本绑定更新原有文本节点。带 ref、属性展开或富文本 runs 的组件沿用运行时绑定。

静态样式支持编译器可证明只读的本地 `const`，以及通过显式相对路径导入的基本类型常量和冻结的扁平对象。含动态值的字面量样式将固定声明放入池中，独立属性单独绑定，共用长属性的简写与长写按原有层叠规则一起更新。静态类名、分支中的类名和字面量 `classList` 的规则可以预编码，主题、断点和交互状态仍由宿主求值。Rolldown 链接时共享相同的样式、类名和 postcard 模板，首次挂载时才注册对应条目。

组件同步读取响应式属性或 accessor，以及异步函数在 `await` 后注册 owner 相关的 effect 或 cleanup 时，编译器会给出警告。虚拟列表的静态 `contentProps.style` 复用引擎的表面约束。编译器与设备运行时的依赖关系由 `just boundaries` 校验。

单文件编译器的 Wasm 接口 `compile_tsx` 返回 JavaScript、诊断和 SourceMap。

```sh
cargo rustc -p bundler --no-default-features --features wasm \
  --target wasm32-unknown-unknown --lib --crate-type cdylib
wasm-bindgen --target web --out-dir target/impasto-playground \
  target/wasm32-unknown-unknown/debug/bundler.wasm
impasto devtools --compiler target/impasto-playground
```

`wasm-bindgen` 工具版本需与 Cargo.lock 中的依赖版本一致。生成的 DevTools 页面内嵌编译器，可以离线编辑 TSX 和查看降级结果。

## Perfetto 和源码位置

使用构建产物对应的 `app.sym`。Detailed 录制从 QuickJS 的检查点采集函数、生成位置和调用栈。检查点提供的行列可以还原到 TSX，缺少生成位置的帧保留 QuickJS 提供的名字。发布侧车去除了源码正文，保留了命名函数的原始范围。

```sh
impasto symbolicate build/app.sym --trace raw.pftrace --output mapped.pftrace
impasto devtools --symbols build/app.sym
```

Perfetto 中的 `js_stack_sample` 同时保留生成栈和 `js_source_stack`。进程外 DevTools 在 Capture 的帧详情中显示同一份映射。

## 热更新与原生契约

macOS、iOS 和 Android 的 `impasto run` 共用 Session 更新协议。监听期间，Rolldown 保留模块图并对已知源文件使用增量扫描。组件函数和默认导出的箭头组件发生兼容变更时，只传输变化的模块工厂和源码映射。只在同模块组件中使用的辅助函数参与对应组件签名，修改辅助函数会更新依赖它的组件。兼容更新复用模块级初始化值，未改动的子树继续使用原有的 Signal、Context、路由和原生节点。热更新直接在内存中生成补丁。纯模块变更会沿依赖关系传到组件边界，入口初始化或不可重放的顶层副作用变更会触发候选 Session 校验。开发工厂保留对应版本的行列映射，`console.trace`、错误页和 DevTools 日志可以显示原始 TSX 位置。

正式平台构建按链接后存活的原生引用省略未用插件的注册和平台工程，同时保留依赖闭包及无 JavaScript 导出的平台插件。对只使用具名导出的宏模块，Rust 编译时移除未使用的方法绑定。对象、事件和命名空间导入保留完整导出。Android 按 `plugin.toml` 中的 `[android.permissions]` 规则生成权限清单。方法范围不可静态确定的对象或命名空间，以及由 Rust 直接调用的依赖，保留关联权限。应用直接声明的权限由应用管理。

平台包内的 `plugins-manifest.json` 记录脚本 ABI 和已链接原生接口的 ABI 指纹。使用 `impasto build bundle --platform android --installed-manifest <清单>` 可在生成 OTA 产物前验证宿主契约。命名空间和动态导入按可能使用的导出校验。新增 Android 权限需求会触发同样的重装提示。

## 静态资源与测试

| 导入 | 构建行为 |
|---|---|
| PNG、JPEG、WebP | 读取固有尺寸并校验相邻的倍率变体。 |
| TTF、OTF | 校验 `assets/` 下的字体并提取名称和字重。宿主从应用包读取字节并注册到字体库，模块仅保留 URI 与字体描述。 |
| SVG | 展开矢量路径和变换，导出 `{width, height, draw(ctx)}`。支持实色填充、描边、虚线和路径填充规则。 |
| WGSL | 校验效果入口及资源，预检 Metal、SPIR-V 和 GLSL ES。可选的 `EffectUniforms` 结构接受 f32 和 f32 向量，生成 `uniformIndex`、`packUniforms` 及 `ImpastoShaderUniforms` 类型映射。`impasto sync` 为 `src/` 和 `assets/` 内的 WGSL 导入生成精确声明，按解析路径区分同名文件。 |

`just solid`、`just solid-app` 和 `impasto test` 通过真实 QuickJS Session 驱动 Taffy 布局、语义查询、输入和离屏测试。`--trace` 提供建树、样式、布局、重绘边界与 FFI 计数。使用 `headless-gpu` 特性构建的 CLI 可比较 PNG。同一轮测试共享框架模块的编译结果。Button、Field、VirtualList 和 Tabs 的用例约束节点创建、FFI 和样式求值的上限。需要应用自己的 Rust 插件时，CLI 会构建链接该插件的测试宿主。

## Gallery 对照数据

### 基础组件与共享池

同一份 Gallery 源码和同一套框架运行时，分别由 `0b07c1a` 的编译器和本次编译器生成 release ESM，再用同一台 Mac 上的 QuickJS Session 交替加载十次。点击场景从自选切换到账户，计时取中位数。

| 指标 | `0b07c1a` 编译器 | 本次编译器 | 变化 |
|---|---:|---:|---:|
| 首帧 FFI 调用 | 2,337 | 1,863 | −20.3% |
| 点击帧 FFI 调用 | 3,132 | 2,463 | −21.4% |
| 点击帧脚本耗时 | 9,347 µs | 6,936 µs | −25.8% |
| release ESM 字节数 | 183,452 | 177,677 | −3.1% |
| IQJS 字节数 | 305,229 | 307,383 | +0.7% |

两边首帧均创建 182 个节点，点击帧原生 setter 均为 145 次。当前自选页在该视口没有可滚动内容，本组不报告滚动性能变化。

### 首次切换原生工具链

在同一台 Mac 上用同一个引擎交替加载旧版和新版 ESM，各运行十轮。FFI 计数覆盖 JavaScript 到原生场景绑定的调用。

| 指标 | 旧版 | 新版 | 变化 |
|---|---:|---:|---:|
| ESM 字节数 | 492,682 | 475,099 | −3.6% |
| IQJS 字节数 | 361,883 | 369,149 | +2.0% |
| 首帧 FFI 调用 | 2,908 | 2,115 | −27.3% |
| 点击帧 FFI 调用 | 1,832 | 1,311 | −28.4% |
| 滚动帧 FFI 调用 | 196 | 147 | −25.0% |
| 点击脚本耗时中位数 | 5,324 µs | 3,682 µs | −30.8% |
| 滚动脚本耗时中位数 | 261 µs | 190 µs | −27.2% |

首帧两边均创建 262 个节点。点击帧的原生 setter 数从 53 变为 83，滚动帧保持 24 次。两个场景均保持 2 次图更新。

用 `just toolchain-baseline --before <旧 ESM> --rounds 10 --min-ffi-reduction 20` 复测，逐轮数据写入 `target/toolchain-baseline/results.json`。

macOS arm64 的 SDK 目录约从 235.7 MB 缩至 59.7 MB，缩减 74.6%。两边按文件逻辑字节数统计。基线使用附带 Node 和依赖的 release 打包配方，在临时副本中补齐了旧脚本缺失的 Darwin 条件块。新版将 TypeScript 归档随 SDK 分发，首次分析时解压到本地缓存。

Gallery 的热更新回归通过真实 QuickJS Session 打开自选 Tab，滚动列表，进入搜索页并输入内容。修改搜索结果的字号和文案后，测试检查相同的输入节点、文本和光标，并返回原 Tab 核对滚动偏移。

```sh
cargo test -p cli --bin impasto gallery_patch_preserves_the_tab_scroll_and_focused_field
```

使用发布版 CLI 在三端分别运行 Gallery `impasto run`，每端连续保存搜索页标题的字号和文案十次，计量从保存到 CLI 收到宿主应用确认的时间。

| 宿主 | 中位数 | 最大值 | 低于 100 ms |
|---|---:|---:|---:|
| macOS | 45.9 ms | 51.2 ms | 10/10 |
| iOS 模拟器 | 47.6 ms | 55.1 ms | 10/10 |
| Android 模拟器（宿主 GPU，GLES） | 41.9 ms | 98.1 ms | 10/10 |

macOS 和 Android 的应用日志显示自选 Tab、搜索路由、滚动偏移和输入内容在补丁间保持一致。并发负载下的 iOS 与 Android 模拟器，以及 Android 软件渲染轮次，均记录到超过 100 ms 的样本。
