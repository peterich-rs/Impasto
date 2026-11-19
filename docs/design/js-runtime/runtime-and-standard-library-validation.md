# JS 运行时与标准库验收

## Rust 标准库复验

`just standard-check` 校验 `legacy` 和 `portable` 的四种传输组合。检查从 Rust 绑定模块逐组生成声明，核对 QuickJS 中的全局值、构造器、静态成员、原型成员及继承，并用同一份用例验证 TypeScript 和 QuickJS。`AbortSignal` 直接构造属于 `legacy`，由两组声明分别表达。

声明契约检查覆盖五组配置。Rust workspace 串行通过 680 项，忽略 8 项。`js`、`js-host`、`js-headless` 和 `quickjs` 的全目标 Clippy 通过。Android debug APK 构建通过，Pixel 9 用独立应用 ID 冷启动并提交 Vulkan 首帧。

原生化验收代码树为 `cdf0853e83f1d6231975b2db8323502cd574c18b`。对照代码树 `06a5aeec47cea700d84da33f24110abfa4d7df1b` 使用 JS prelude。原生化验收覆盖两个 profile 的 Rust Web API。Headers 和 URL 状态计入标准库的 Rust 存储额度。Body 缓冲、reader、Fetch 请求和事件句柄由 Rust 管理，持有的 QuickJS 对象参与 GC tracing。`js::quickjs::buffer` 在 VM 创建时捕获原生访问器，标准库与宿主解码共用实际视图长度。

workspace 串行通过 675 项，沿用 8 项忽略。相关 crate 的全目标 Clippy、依赖边界、portable TypeScript 正反例及独立消费程序通过。新增用例覆盖原生构造器继承、监听再进入、Abort 清理、reader 成环后的额度释放、可调整大小的字节视图和 console 手动时钟。Pixel 9 使用独立 application ID 启动 debug Gallery 并提交 Vulkan 首帧，验收包随后清理。 macOS 宿主单测通过 20 项。首轮原生标准库构建提交了 Metal 首帧，最终代码树的两次窗口复验停在 `CVDisplayLink` 初始化，返回 `-6661`。

Release 在同一 macOS 上用同一 Gallery bundle 和夹具交错测量，每侧预热 2 轮并采样 20 轮。表中为中位数。

| 场景 | JS prelude | Rust 标准库 |
| --- | ---: | ---: |
| 空 Runtime 启动，ms | 1.844 | 0.254 |
| 空 UI 启动，ms | 3.223 | 1.725 |
| Gallery 启动，点击负载，ms | 101.10 | 98.88 |
| Gallery 市场点击脚本及布局回调，ms | 5.512 | 5.506 |
| Gallery 滚动脚本及布局回调，µs | 243.90 | 243.94 |
| 16 个 timer 和 16 个 Promise job，µs | 19.10 | 23.44 |

点击和滚动的 reactive 更新及 setter 计数一致。原始样本、p95、最大值、二进制哈希和平台日志在 `target/js-native-standard-acceptance/`。

## 运行时迁移初次验收

基线提交为 `c0f409bb4acc817f927560881f8edecbbd48281b`。验收代码树为 `06a5aeec47cea700d84da33f24110abfa4d7df1b`，由候选的 Cargo、crate、包、工具和 CI 文件生成，文档沿用基线索引。运行环境为 Apple Silicon macOS 27.2.1、Rust 1.100.0、Impeller SDK `77f043bc63196b9979cc09c5b97bb6d9b00be34d`。Android 构建使用 NDK 28.2.13676358 和 JDK 21。

## 接入和兼容

- `js` 管理 VM、宿主模块、值转换、任务和资源。独立运行入口是 `cargo run --locked -p js-headless`，该程序只依赖 `js`，验收了严格编译的 IQJS 模块、同步和 Promise 方法、owned 模块事件、标准库与外部传输。
- `quickjs` 在显式扩展内安装 UI 绑定。源码与受信任字节码入口保留原引导检查点，入口后接入 Shell 外部传输。外部完成仍在命令点交付，启动和本地积压在帧内按 HTTP、模块、socket 的顺序交付。UI 保留严格 job checkpoint 和原 reactive、布局、Canvas 顺序。
- 本地 HTTP 使用 Inline 或 External 中的一种。启动流错误在 JS 登记之后交付，空 EOF 关闭 reader，abort 立即释放请求及 Promise。带数据的末块按数据计费，空终结事件可用预留位置。后台 credit 等待与 Shell 命令接收分离，Runtime 关闭时唤醒等待者。Shell 的 HTTP 重定向逐跳限制为 HTTP 或 HTTPS，并保留 HTTPS 降级拒绝。
- `legacy` 保留整响应 `fetch`、`fetchStream().getReader()` 和 HTTP 错误名称。共享的 Body 实现修正空流、读中取消、终结后保留未读数据和读取失败。`portable` 的 fetch 在响应头到达时结算，要求 External。当前声明覆盖的 Web 子集见 `crates/js/src/standard`。URL 使用锁中的 `url 2.5.8`，编码覆盖 UTF-8 和流式解码。Web 子集用 QuickJS 执行等价边界夹具。
- 后端的 loader、`NativeFactory` 和受信任字节码入口位于 `js::quickjs`。一般编译和票据入口使用目录和自有结果。字节码使用原 IQJS 封套。CLI 独立 bundle 使用首方模块目录，macOS 和 Android 打包使用应用与目标声明的模块目录。
- 兼容入口的消费者包括现有 `open_program_with_io` 和 `open_trusted_bytecode_with_io` 的调用方，以及基于旧 `IoEvent` 的测试和嵌入者。平台主路径走 `Delivery`。兼容 token 入口在这些消费者迁到显式路由和完成工厂后再删除。
- 保持提前装配。空 VM 与 UI 测得约 1 ms 增量，Gallery 启动的中位数接近基线，当前样本未显示拆出独立 lazy 安装的收益。

## 验证

| 检查 | 结果 |
| --- | --- |
| Rust workspace | `cargo test --locked --workspace --no-fail-fast -- --test-threads=1` 通过 667 项，忽略 8 项。包括 QuickJS 120 项和 macOS 宿主 20 项 |
| 调度和资源 | 子进程验证 eval、timer、TLA、job、listener 的执行期限和恢复。资源回归覆盖连续 64 次完成与取消、到期、坏交付后的下一条事件、票据消费及 Web body credit |
| Shell I/O | 流耗尽单流额度时，短请求与本地 WebSocket 握手仍交付。关闭后流任务在测试期限内退出等待。OpenWire 本地 HTTP 与增量读取通过 |
| 依赖和声明 | 两个边界脚本、style-types `--check`、module-stubs `--check`、首方 TS 正反例和 portable TS 正反例均通过 |
| Rust 检查 | `RUSTFLAGS="-D warnings" cargo check --locked --workspace --exclude macos`、运行时相关 crate 的 Clippy、无 SDK 快照的全 workspace Clippy、rustfmt 和 diff 检查通过 |
| TypeScript | `pnpm install --frozen-lockfile`、`just gallery-dist` 和 `pnpm -r test` 通过 |
| macOS arm64 | 使用已链接 Impeller SDK 启动 Gallery，提交首帧并输出 720 × 1280 PNG |
| Android arm64 | `just android` 和 `just android-release` 构建成功。独立 ID 的 debug APK 在 Pixel 9（API 37）与 ARM64 模拟器（API 34）冷启动并提交 Vulkan 首帧 |
| Android x64 | 用 NDK 的 API 24 x86_64 编译器构建 `x86_64-linux-android` debug 原生库并核对 ELF ABI |
| Kotlin | `kotlin/gradlew -p kotlin :library:jvmTest` 成功，任务结果为 `NO-SOURCE` |

## 性能样本

Release 基线和候选使用同一 Gallery bundle 和同一测试夹具。各预热 2 轮，交错测量 20 轮。每格依次为中位数、p95、最大值。RSS 取各测试进程的峰值。

| 场景 | 基线 | 候选 |
| --- | ---: | ---: |
| 空 Runtime 启动，ms | 0.846 / 0.892 / 0.902 | 1.992 / 2.085 / 2.127 |
| 空 UI 启动，ms | 2.340 / 2.560 / 2.578 | 3.398 / 3.587 / 3.651 |
| 16 个 timer 和 16 个 Promise job，µs | 19.12 / 28.58 / 30.50 | 19.42 / 29.04 / 52.17 |
| Gallery 启动，点击负载，ms | 104.85 / 110.66 / 113.58 | 104.39 / 108.32 / 116.55 |
| Gallery 市场点击脚本及布局回调，ms | 6.174 / 6.566 / 6.637 | 6.038 / 6.300 / 6.429 |
| Gallery 滚动脚本及布局回调，µs | 265.75 / 314.08 / 385.63 | 262.37 / 309.00 / 311.88 |
| 进程峰值 RSS，MB | 23.99 / 25.23 / 26.33 | 24.46 / 25.99 / 26.07 |

点击维持 2 次 reactive 更新、53 次 setter。滚动维持 2 次更新、24 次 setter。屏外 Canvas、批量 Canvas 和固定布局的不变量由 workspace 回归校验。原始样本、二进制与锁文件哈希、测试日志、APK 验收日志和首帧 PNG 保存在仓库根的 `target/js-runtime-acceptance/`。

另用相同的 legacy `fetchStream` 夹具交错测量有请求在途且尚无事件的 512 次 `drive`，再投递 16 个 16 KiB chunk 并读取。各侧预热 2 轮和测量 20 轮。空转驱动中位数为 5.75 → 68.73 µs，两侧均无 Rust 分配。256 KiB 入队并交付的中位数为 21.81 → 108.42 µs，读取中位数为 23.85 → 37.77 µs。入队和交付的 Rust 累积分配为 265,440 → 266,032 字节。排队后的 QuickJS 分配快照为 176,248 → 651,344 字节。候选把 chunk 复制进 QuickJS heap，由 VM 上限约束应用持有的数据，基线的外部 buffer 不列入该 heap 快照。原始数据见 `target/js-runtime-acceptance/transport-bench/`。

## 待验证

- 原生 Linux CI 和 Android x64 模拟器运行。本机模拟器的 ABI 为 arm64-v8a。
- 已链接 Impeller SDK 的全 workspace Clippy 仍报已有 engine FFI 标注与 lint 问题。验收使用仓库 CI 同样的无 SDK 路径，以及 SDK 链接下的宿主实际启动。
- Gallery 的逐阶段分配与复制字节、固定布局单独时延 A/B，以及原生宿主的 idle 和恢复时延。当前流样本来自独立 Runtime 的手动时钟。
