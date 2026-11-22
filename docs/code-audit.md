# Impasto 源码审查

> 文中的实施阶段与代码引用反映设计或审计时的仓库结构。当前 crate 职责、源码入口和验证命令见 [架构](architecture.md)。

本次引擎整合处理了 A03 的生产帧输出和 A04 的图片依赖汇总。会话直接产出层树，图片依赖由每张 picture 在构建期计算。A04 的语义投影独立跟踪。

## 审查范围与参考仓库

本报告以源码、构建配置和实际测试为依据，不把 README、AGENTS.md 或规划文档中的描述当作实现证据。问题和测试结果对应下列审查基线；后续修改代码后，需要重新验证，不能把本报告的结果直接视为新版本的状态。

| 仓库 | 本地绝对路径 | 审查提交（短 SHA） |
| --- | --- | --- |
| Impasto | `/Users/zhangfan/develop/github.com/Impasto` | `18a509f` |
| Flutter | `/Users/zhangfan/develop/github.com/flutter` | `095babc49d5` |
| React Native | `/Users/zhangfan/develop/github.com/react-native` | `c48341282ec` |

除特别注明外，报告中的源码路径相对于 Impasto 仓库根目录。Flutter、React Native 的关键参考文件在第七节列出完整路径。上述本地路径依赖当前机器的检出位置。

审查时没有修改产品源码。本报告不是对所有源码零遗漏的证明；没有执行 Android 真机、TalkBack 或跨平台像素 golden，相关结论与实际复现分开标记。

## 总体结论

**当前最大的差距不是控件数量，而是一些基础契约没有闭合。**

已经存在的能力中，持久化一致性、图片失效、输入顺序、文本编辑、无障碍以及测试失败传播，都有可以定位到代码、甚至直接复现的问题。继续增加控件，会让更多上层功能依赖这些不稳定行为。

需要明确两件事：

1. **接上 Impeller，不等于接上了 Flutter 的完整 UI 能力。** Flutter 的布局失效、手势竞争、编辑协议、语义树、资源管理和平台适配，仍然需要这里自己实现。
2. **Solid 避免的是组件树层面的 reconciliation，不是整个框架的增量更新问题。** 当前代码里仍有全树扫描、全量数据传递和平方复杂度操作。

本报告不列优点，只列问题及设计建议。

## 一、证据等级与实测结果

证据标记：

- **复现**：审查时执行小程序或测试确认。
- **确定**：可以从源码直接推出。
- **风险**：架构或生命周期存在缺口，但没有做完整故障注入。
- **缺口**：没有找到完整的可交付实现，不等于必须立刻补齐。

优先级：

- **P1**：阻止生产应用依赖。
- **P2**：阻止规模化、复杂应用或稳定跨平台交付。
- **P3**：接口与维护质量问题。

### 实测结果

| 检查 | 结果 |
| --- | --- |
| `cargo test --workspace` | **失败**。QuickJS 部分 58 通过、2 失败、5 忽略。 |
| `cargo test -p uicore -p session` | `uicore` 209 通过、1 忽略；`session` 6 通过。 |
| `pnpm -r test` | 返回成功，但输出 **7 次** `TypeError: host3.setTextRuns is not a function`。 |
| 32 个并发存储写入 | 所有写入返回成功，但该次运行只剩 **2/32 个键**。 |
| 64 个正常结束的 Promise 微任务 | 被判为 `quickjs job round limit (jobs)`，会话打开失败。 |
| 可选图片先缺失、后到达 | 第二次提交仍复用旧 picture，图片解析回调调用次数为 **0**。 |
| 平移后的文字语义坐标 | 视觉坐标 `(100,50)`，语义坐标仍为 `(0,0)`。 |
| 移除 `onClick` 属性 | 旧回调仍然执行。 |
| `<row w={100} h={40}/>` | 编译器把尺寸放进 `staticProps`，渲染器随后把它们跳过，没有写入尺寸。 |
| 带 headers 的 `Request` | 直接抛出 `TypeError`。 |

QuickJS 失败的两个测试：

- `motion_end_survives_unregistering_its_own_callback`
- `motion_end_reports_a_throw_after_unregister`

这证明审查基线上的测试契约不成立；**仅凭这两个失败，不能进一步断言究竟是动画实现还是测试时序错误。**

## 二、已经存在的确定性缺陷

### 2.1 数据、网络、资源与运行时

| 编号 | 级别 / 证据 | 问题、影响与代码位置 | 应该如何设计 |
| --- | --- | --- | --- |
| D01 | **P1 · 复现** | **`AsyncStorage` 会丢并发写入。** `crates/hostshell/src/files.rs:43` 是读取整张表、修改、覆盖文件；`crates/hostshell/src/exec.rs:18` 用 `spawn_blocking` 并发执行，没有串行化或事务。32 个成功写入只留下 2 个键。`save_store` 也不是原子替换，进程中断可能留下截断文件。 | 存储由单一执行者串行持有，或使用有事务的成熟存储。明确原子性、写入顺序、崩溃恢复语义，加入并发与中途终止测试。不能只给单次 `fs::write` 加锁。 |
| D02 | **P1 · 复现** | **常规带请求头的 fetch 路径坏了。** `crates/quickjs/src/prelude.js:235` 的 `Headers.forEach` 忽略 `thisArg`，而 `Request` 在 `:252` 后用它调用 `this.set` / `this.append`。例如 Authorization 请求头会在发出请求之前抛异常。 | 使用经过验证的实现；若继续维护自己的实现，必须按完整调用契约实现，并运行标准兼容测试。不能用 Node 原生 fetch 的测试替代 QuickJS 中的实现。 |
| D03 | **P1 · 复现** | **图片到达和 reload 没有正确贯穿缓存失效。** `crates/hostshell/src/shell/cache_cmds.rs:32` 收到图片只是更新缓存、请求一帧；`crates/hostshell/src/shell.rs:874` 只用层变化和 scale 判断是否重新提交；`crates/engine/src/pictures.rs:65` 只检查 picture generation 和字体 stale；`crates/engine/src/player/upload.rs` 又在 source 已存在时拒绝替换纹理。可选图片可能一直不出现，reload 也可能继续画旧资源。 | 引入资源 ID 与 revision，以及 picture 对资源 revision 的依赖。图片到达、替换、失败恢复必须使依赖它的 picture 和提交状态失效。**请求一帧不等于内容失效。** |
| D04 | **P1 · 确定** | **Android APK 没有网络权限。** `crates/android/apk/app/src/main/AndroidManifest.xml:1` 没有 `android.permission.INTERNET`，但宿主安装了 HTTP 客户端。当前 APK 的网络能力不能按实现存在来验收。 | Manifest 与宿主能力一起生成、校验；打包产物上执行真实 HTTP、TLS、WebSocket 测试。 |
| D05 | **P1 · 复现** | **正常微任务被当成失控脚本。** `crates/quickjs/src/glue.c:3364` 限制每次 drain 最多 32 个 job，超过就返回错误。64 个有限 Promise 回调即可触发，不需要无限循环。 | 区分“执行失败”“本轮预算耗尽”“真正失控”。预算耗尽后保留队列并安排后续执行，而不是将正常异步工作转换成脚本异常。初始化也应有独立策略。 |
| D06 | **P1 · 确定** | **HTTP 和流式请求串行阻塞整个网络命令循环。** `crates/hostshell/src/net.rs:37` 在接收循环内直接调用 `client.perform`、`client.stream`。一个慢请求会挡住后面的请求，长流还会挡住 WebSocket 的 open/send/close 命令。内部使用 Tokio 并没有改变这一事实。 | 使用有并发上限的异步请求调度，每个请求独立任务；控制命令与数据消费分开。连接、请求、流读取都要有取消和背压。 |
| D07 | **P1 · 复现** | **流式网络错误会伪装成正常结束。** `crates/quickjs/src/prelude.js:497` 在 response 已 resolve 后，错误分支让挂起的 `read()` 返回 `{done:true}`，再 reject 已经完成的 opened Promise，错误无法到达消费者。`Reader.cancel` 也没有完整结束挂起读取。 | 流应有明确的 open/closed/errored/canceled 状态，读取 Promise 支持 reject，取消必须完成所有等待者。优先复用符合 Streams 语义的实现。 |
| D08 | **P1 · 确定** | **WebSocket 生命周期和读调度不完整。** `crates/hostshell/src/net.rs:107` 限制 8 个 socket，但远端关闭/连接失败不会从 `serve` 的表中移除；`crates/quickjs/src/prelude.js:468` 关闭后也不删除 JS socket。`socket_loop:328` 只有发送队列 50ms 超时才读 socket，持续发送会饿死接收；普通读错误被 `Err(_) => {}` 吞掉。 | 独立读写任务或一个真正的异步事件循环；连接结束必须回传并释放注册项。终态只发生一次，携带 close code/reason；错误不能无限重试且不通知调用者。 |
| D09 | **P2 · 确定** | **HTTP 取消表永久增长。** `crates/quickjs/src/linked/bridge.rs:1062` 向 `cancels` 插入 token，完成和取消路径都没有移除。JS 的 AbortSignal 也没有完整的监听器清理。长期请求会累积原生状态。 | 每个请求有明确的生命周期对象；成功、失败、取消、会话销毁统一释放 token、监听器和外部缓冲。 |
| D10 | **P1 · 风险** | **图片尺寸限制执行得太晚，挡不住解码内存峰值。** `crates/android/src/decode.rs:172`、`crates/macos/src/pixels.rs:53` 先按原始尺寸分配 RGBA；`crates/hostshell/src/shell.rs:389` 才检查像素上限。小压缩文件仍可能触发巨大分配。 | 在分配前读取 header 并检查尺寸、stride、总字节预算；支持目标尺寸解码。预算要覆盖解码中的临时缓冲，而不只是缓存里的成品。 |
| D11 | **P1 · 确定** | **两端图片 alpha 契约不一致。** macOS 明确输出 premultiplied RGBA；Android 请求 unpremultiplied，失败时又保留默认值。`ImagePixels` 只有字节，没有 alpha 类型，上传也不统一转换。半透明图片没有跨端一致性的基础。见 `crates/macos/src/pixels.rs:55`、`crates/android/src/decode.rs:152`、`crates/engine/src/player/upload.rs`。 | 像素格式必须包含 alpha mode、色彩空间和 stride；在统一边界转换成一种确定格式。拒绝“请求失败就悄悄换一种格式”。 |
| D12 | **P1 · 条件性复现** | **文档目录限制可以被符号链接绕过。** `crates/hostshell/src/files.rs:73` 只做路径字符串检查后 `root.join`。在隔离临时目录中，`docs/link/secret` 成功读到了 root 外的文件。触发前提是目录内存在指向外部的链接。 | 如果这是能力沙箱，就使用目录句柄相对访问及 no-follow 等约束，处理 TOCTOU；否则明确它不是安全边界。单纯禁止 `..` 不构成文件沙箱。 |

### 2.2 Solid 属性更新与文本输入

| 编号 | 级别 / 证据 | 问题、影响与代码位置 | 应该如何设计 |
| --- | --- | --- | --- |
| D13 | **P1 · 复现** | **静态 JSX 属性被无条件跳过。** `packages/solid-renderer/src/tree.ts:519–608` 对所有标签跳过 `w/h/color/...`，但不是所有构造函数都应用这些值。已核对 Babel 输出：`<row w={100} h={40}/>` 正好进入这个错误路径。 | 每种节点声明自己在构造时消费了哪些属性，剩余属性统一走 setter；或者统一创建后应用属性。静态和动态写法必须具有相同语义。 |
| D14 | **P1 · 复现** | **事件不能被声明式移除。** `packages/solid-renderer/src/tree.ts:405` 以及后续事件分支，只在新值是函数时注册；新值为 `undefined/null` 时什么也不做。旧行为和闭包继续存在。 | 监听器 API 必须支持 replace/remove；渲染器要使用 previous value 或稳定 listener slot。所有事件都需要“函数→函数→空”的测试。 |
| D15 | **P2 · 确定** | **属性系统不是闭合的状态更新模型。** `fill=false` 不撤销 fill，`margin=undefined` 不恢复默认值；动态 `source/mode/required/columns/rows` 没有对应分支；JSX 声明的 `image.alt` 会落入 unknown prop。见 `packages/solid-renderer/src/tree.ts:243,370,514` 与 `packages/solid-renderer/src/jsx.ts`。 | 为每个属性定义 set/reset/default/适用节点。类型声明、静态属性、动态属性和原生 setter 必须来自同一个 schema。 |
| D16 | **P1 · 确定** | **输入时间顺序被打乱。** `crates/hostshell/src/input.rs` 分开保存 pointer 和键盘输入；`crates/hostshell/src/shell.rs:743–756` 先处理所有 pointer，再处理所有输入。先给 A 输入、后点击 B，如果进入同一帧，文字可能在焦点切到 B 后才派发。 | 使用统一、有序的输入队列；允许合并相邻移动事件，但不能跨越键盘、焦点或 IME 屏障。 |
| D17 | **P1 · 确定** | **Field 发布的 IME 光标位置是假的。** `packages/solid-ui/src/field/shell.tsx:209` 总是传 `(0,0,1,h)`；焦点放在 pointer 外壳上，而 `crates/quickjs/src/linked/mod.rs:132` 只从“当前聚焦节点自身”读取 editor，无法替这个外壳修正坐标。候选窗和 cursor anchor 无法正确跟随文字。 | 焦点节点应明确关联 `TextInputClient/EditorId`，由编辑器和布局系统计算真实光标矩形；不要递归找第一个 Text，再期望另一层猜到编辑器是谁。 |
| D18 | **P1 · 确定** | **Android 全量编辑回传破坏撤销与反向选区。** `packages/solid-ui/src/field/controller.ts:294` 的 `replace` 对变化文本使用 `EDIT_LOAD`；`crates/uicore/src/text.rs:356` 的 Load 清空 undo。选区又用 `Math.max(anchor,caret)` 规范化，丢失方向，反向选区可能直接折叠。 | 区分外部初始化与 IME 编辑事务；保存 base/extent、composing、revision，不把每次输入当成重新加载文档。 |
| D19 | **P1 · 确定** | **格式化输入没有正确的选区编辑模型。** `packages/solid-ui/src/field/controller.ts:190,250` 直接 `base + value`、`rawBase() + value`。光标在中间或存在选区时，启用 mask/transform 的插入可能变成尾部追加。 | 编辑器先按选区应用 edit，再运行 formatter；formatter 返回新文本及选区映射。不要在 JS 里再拼一套与 Rust 编辑器不同的编辑语义。 |
| D20 | **P2 · 风险** | **IME 消息没有完整的 client 身份。** `session::InputEvent::Replace` 不携带编辑客户端 ID/revision；`crates/android/apk/app/src/main/java/dev/impasto/host/TextInput.java` 的 `nativeSync` 也不携带客户端身份。输入连接切换后，延迟到达的旧消息缺少可靠的归属检查。 | 使用带 generation 的输入连接 ID，编辑状态带 revision；过期连接的消息必须拒绝或重同步。 |
| D21 | **P2 · 确定** | **光标闪烁导致聚焦期间持续申请每一帧。** `packages/solid-ui/src/field/shell.tsx:224` 虽然每 500ms 才改变 blink，却每帧 `requestFrame()`。这会持续触发脚本和帧管线，不只是画光标。 | 使用下一次闪烁截止时间唤醒，仅重绘 caret 所属 picture；静态输入框不应把整个应用变成持续帧循环。 |

### 2.3 无障碍、诊断与公共 API

| 编号 | 级别 / 证据 | 问题、影响与代码位置 | 应该如何设计 |
| --- | --- | --- | --- |
| D22 | **P1 · 复现** | **语义几何与视觉几何不一致。** `crates/uicore/src/tree/semantics.rs:129` 使用 `layout_point`，后者明确不含 scroll/transform；`crates/hostshell/src/semantics.rs` 直接把矩形交出去。平移 `(100,50)` 后仍报告 `(0,0)`。 | 语义树应携带局部变换、clip 与可见性，或统一生成最终场景空间几何。绘制、命中、语义不能各算一套位置。 |
| D23 | **P1 · 确定** | **Android AccessibilityNodeProvider 只是部分节点适配，不是完整无障碍实现。** `crates/android/apk/app/src/main/java/dev/impasto/host/AccessBridge.java:76` 使用数组下标作为 virtualViewId，数组重排就改变身份；`:164` 未处理 accessibility focus/clear focus；没有完整 `findFocus` 和相关事件状态。 | 建立稳定 SemanticsId→virtualViewId 映射；实现 accessibility focus、hover exploration、事件通知、节点删除后的焦点恢复和文本选区动作。必须用 TalkBack 验收。 |
| D24 | **P2 · 确定** | **语义属性存在“能设置但不生效”。** `SemanticsHint.live` 被接收，但 `SemanticsNode` 和平台投影没有这个信息。也缺少完整 merge/exclude/hidden 语义，不能假设“渲染节点都导出”就是正确阅读顺序。见 `crates/uicore/src/semantics.rs`、`crates/uicore/src/tree/semantics.rs`。 | 语义节点应独立于布局包装节点；显式支持合并、排除、隐藏、live region、动作与阅读顺序，并保证端到端传递。 |
| D25 | **P1 · 风险** | **Android release 也默认启用网络诊断采集。** `crates/android/src/host.rs:656` 无条件安装 DiagnosticsHub；`crates/hostshell/src/diagnostics/recorder.rs:67–69` 保留完整 URL 和 body preview。虽然部分 header 已脱敏，但 URL query、请求体和响应体仍可能含敏感数据。 | release 默认关闭内容采集；提供按应用授权的采样与字段脱敏策略。认证头脱敏不等于网络内容安全。 |
| D26 | **P2 · 确定** | **`Boot::with_devtools` 会把 Boot 消耗并丢弃。** `crates/hostshell/src/lib.rs:169` 接收 `self`、设置字段，却只返回 hub，调用者拿不到配置后的 Boot。 | 返回 `Self`、`(Self, Arc<DiagnosticsHub>)`，或借用 `&mut self`。这是行为错误，不只是命名偏好。 |

## 三、架构与能力抽象上的问题

这些不是“把大文件拆小”就能解决的。核心是明确所有权、状态边界、发布时机和复杂度。

| 编号 | 级别 | 架构问题与源码证据 | 建议设计 |
| --- | --- | --- | --- |
| A01 | **P1** | **缺少统一的帧提交契约。** `JsSession::frame` 修改 Tree 后，`crates/hostshell/src/shell.rs:823` 已发布 IME、语义；随后才检查图片是否就绪并提交 GPU。提交失败或资源等待时，画面、命中几何、语义可能不属于同一版本。`crates/quickjs/src/linked/mutation.rs:869` 的 `flush_mutations` 也可能先应用部分操作，再因错误丢掉余下队列。 | 定义 frame revision 和明确的 prepare/commit/fail 状态。失败不一定要回滚全部 JS 状态，但可交互几何、语义和已接受的显示内容必须有一致的版本关系，不能只“保留上一张图”。 |
| A02 | **P2** | **通用 UI 管线被放进 QuickJS 绑定内部。** `crates/quickjs/src/linked/mod.rs:787` 同时编排动画、jobs、timer、RAF、Solid flush、布局、ListDriver、布局回调、录制、语义、IME 和资源发布。换一种脚本实现、做无 JS 的集成测试，都要绕过这个大入口。 | 把通用帧管线从语言适配器中提取出来。语言适配器负责调用与事件分发；布局、录制、资源依赖和帧发布由独立模块负责。无需为此立刻拆成很多 crate。 |
| A03 | **P2** | **增量录制仍有全树前置成本。** `crates/uicore/src/tree/record/mod.rs:72` 每次清空层信息并从 root 做 `sweep_subtree_layers`；还生成扁平 commands 的镜像。缓存 picture 并不代表更新成本只与脏节点有关。 | 维护子树层属性的增量摘要；录制以脏 owner 为单位。扁平显示列表若仅供调试/测试，应按需生成，而不是运行时每次都维护第二份结果。 |
| A04 | **P2** | **资源枚举和语义投影有明显的全量/平方成本。** `crates/protocol/src/layer.rs:42` 为取图片需求克隆所有 picture commands；`DrawList::image_requirements` 线性查重；`hostshell::semantics::project` 对每个输出节点调用扫描全表的 `content_extent`。 | 图片依赖在 picture 构建时汇总并缓存；用索引或集合查重。语义建立 parent→children 索引，单次遍历计算 extent，保留 patch 到平台端，不每次展开成全量数组。 |
| A05 | **P1/P2** | **虚拟列表在数据进入时就是 O(n²)。** `crates/uicore/src/list/driver.rs:190` 每插入一项，都在 staging 中线性查重；`packages/solid-ui/src/VirtualList.tsx` 又全量 map 全部 items。重复 key 被静默丢弃，还可能使 native 索引与 `props.items[index]` 不再对应。 | key→index 建表一次，重复 key 明确报错；支持数据集 revision 和批量增删改。虚拟化不仅要减少挂载数量，也要控制数据更新成本。 |
| A06 | **P2** | **调度策略是“若干硬限制”，不是可解释的帧预算。** JS、布局、录制、提交都在同一线程；`QJS_TURN_MS=128` 每个入口重新计时；Rust 回调不受 JS interrupt 直接约束。`Inbox`、网络 channel 和工作任务没有统一背压。见 `crates/quickjs/src/glue.c`、`crates/hostshell/src/queue.rs`、`crates/hostshell/src/shell.rs`。 | 可以先保持 JS 与 Tree 同线程，但必须设置整帧预算、任务优先级、队列上限和超载策略。不要盲目多线程化；先确定何时让出、哪些工作延后、哪些事件不可丢。 |
| A07 | **P2** | **跨语言 schema 只统一了一部分编号，没有统一契约。** `crates/quickjs/build.rs` 生成枚举；构造、参数、默认值和回调签名仍散落在 `core.js`、C、Rust parser、`CoreHost`、`tree.ts`、`jsx.ts`。host stub 甚至把方法生成为无参数 `Promise<unknown>`，再靠断言改类型。 | schema 应覆盖参数与返回类型、nullable、事件、默认值、错误、单位、线程要求和 capability version，生成双端校验与 TS 声明。否则新增能力必然继续漂移。 |
| A08 | **P2** | **后台任务没有完整的会话所有权。** `crates/hostshell/src/watchdog.rs:57` 启动永久循环，没有停止条件；每次 start 又启动一次。I/O 线程创建失败多处被丢弃；`JsSession::drop` 没有统一取消在途请求。 | 一个 runtime 持有 task group：启动要确认成功，退出取消并 join，完成结果带 session generation。watchdog、网络和模块工作都不能游离在宿主生命周期之外。 |
| A09 | **P2** | **手势实现是内置优先级路由，不是可组合竞争系统。** `crates/uicore/src/tree/gesture.rs` 的 PointerTable 保存一条 route，scroll lock 不接纳第二个手指；tap、long press、selection drag、scroll 的优先级写在 Tree 中。后续 pinch/rotate、嵌套拖拽会持续增加特殊分支。 | 分离原始 pointer 路由和 recognizer；引入接受、拒绝、等待以及组合关系。无需照抄 Flutter，但必须能表达竞争、同时识别、多指和取消。 |
| A10 | **P2** | **文字排版的查询契约不完整。** `crates/hostshell/src/shaper.rs:144` 的 min-content 只按空白分词，而且只接收文本和字号，丢掉 family/weight/spans；`:153` 的 caret 查询也没有传 spans。富文本绘制和选择几何可能使用不同排版输入。大量排版错误还被转换成零尺寸。 | 测量、min-content、caret、word boundary、selection 必须共享完整段落键。换行使用实际断行规则；错误应可诊断，不能默默变成合法的空盒子。 |
| A11 | **P1 · 风险** | **FFI 安全边界依赖注释而不是 API。** `NativePtr(pub *mut c_void)` 可安全构造且 `Send+Sync`；`Engine::draw_metal` 是接收任意裸指针的安全公共函数，却要求它是活着的 CAMetalDrawable。见 `crates/hostshell/src/ptr.rs`、`crates/engine/src/player/mod.rs:283`。 | 裸指针进入系统的构造点应明确为 unsafe，之后使用封装了所有权和生命周期的句柄。不是说现有宿主一定发生 UAF，而是公共安全 API 没有保证它声称的安全性。 |
| A12 | **P2** | **应用实例、root、窗口与会话的身份没有统一。** Android 是全局 `HOST`；TS runtime 有全局 `overlayId`、topLayers；`boot` 没有交回完整卸载句柄。多实例嵌入、反复挂载和开发重载没有清晰的生命周期。见 `crates/android/src/host.rs:454`、`packages/solid-renderer/runtime/index.ts`。 | 定义应用实例和 surface/session/root 的归属；`mount` 返回 disposer；overlay、listener、资源、router 都属于该实例。单实例也要显式约束，而不是靠全局变量隐含。 |

### 虚拟列表实测说明

在审查机器的 **debug 构建**下，仅调用真实 `DriverSet` 填入数据：

| 数据量 | 该次运行耗时 |
| ---: | ---: |
| 5,000 | 约 111ms |
| 10,000 | 约 312ms |
| 20,000 | 约 1.10s |

这不是 release FPS 基准，但 `staging.iter().any(...)` 导致的平方复杂度不依赖构建模式。

**“只有可见行被渲染”无法抵消数据提交阶段的 O(n²)。**

## 四、成熟框架层面的能力缺口

不能简单地写“没有 X 就不成熟”。应该按目标判断：哪些是通用框架底座，哪些是产品范围选择。

| 编号 | 级别 | 当前缺口与证据 | 应该形成的能力 |
| --- | --- | --- | --- |
| G01 | **P1，若要动态下发** | **可以执行 JS，但没有形成可运营的动态包系统。** 当前入口是脚本文本/文件或 APK asset；没有找到完整的包身份、原生能力版本协商、签名验证、原子切换、失败回滚与坏版本隔离。见 `crates/quickjs/src/linked/mod.rs`、`crates/android/src/host.rs`、`crates/macos/src/main.rs`。 | Bundle manifest：包 ID、版本、hash/signature、最低 runtime/ABI、依赖能力、资源清单；安装和激活分离；保留已知可用版本。注意：这不是 React Native 默认提供的能力，但它是动态化目标自己的要求。 |
| G02 | **P2** | **自绘能力仍被封闭在固定原语里。** `crates/protocol/src/draw_list.rs:421` 的 `Command` 主要是矩形、文字、图片和层操作；`Kind` 是封闭枚举。没有完整的 path/canvas/custom paint 扩展面。图表只能不断拼树节点，用户难以实现新的绘制对象。 | 提供受控的自定义绘制节点：约束测量、paint bounds、显示列表、命中与语义扩展。图表绘图坐标属于应用，但不应迫使每条线都成为布局节点。 |
| G03 | **P2** | **图片请求只有 source 和密度，没有目标解码尺寸。** `crates/hostshell/src/images.rs` 的 `ImageRequest` 无尺寸策略，解码器一次返回一张 RGBA；动态图片帧、解码取消、按显示尺寸降采样没有完整抽象。 | Image request 应表达资源身份、目标尺寸、采样、颜色/alpha、动画帧和取消；缓存 key 与这些参数一致。 |
| G04 | **P2** | **跨端像素一致性没有确定的字体与颜色基础。** 两端会从不同系统位置加载 CJK 字体；macOS 图片使用 device RGB；文字的 fallback、字体版本和色彩转换没有统一产品契约。见 `crates/macos/src/app.rs:865`、`crates/android/src/host.rs:383`、`crates/macos/src/pixels.rs`。 | 打包确定字体及 fallback 集合，定义 sRGB 等色彩策略，明确哪些行为允许遵循系统、哪些要求一致。没有这一步，不应把“同一个光栅器”解释成“同一像素”。 |
| G05 | **P2** | **一些平台 API 只有接口外形。** 两端 `pick_document` 都返回 `Ok(None)`；macOS `request_permissions` 只是查询状态；Android Boot 未注册 Permissions。调用者无法区分“不支持”和“用户取消”。见 `crates/macos/src/app.rs:1269`、`crates/android/src/host.rs:330,606`。 | capability discovery 加结构化 Unsupported/Denied/Cancelled 错误；只有完整接通平台生命周期的能力才应宣称支持。 |
| G06 | **P2** | **Android 生命周期恢复和链接处理不完整。** `crates/android/apk/app/src/main/java/dev/impasto/host/HostActivity.java` 接收了 `savedInstanceState`，没有恢复应用状态；没有完整 `onSaveInstanceState/onNewIntent` 路径。`packages/solid-router/src/stack.ts:216` 的 Linking 监听也没有清理。 | 状态恢复协议、路由恢复、编辑状态恢复；热启动 URL 投递；订阅归属 owner 并处理异步注册后已销毁的情况。 |
| G07 | **P2** | **最低 Android 版本与图片实现不匹配。** `crates/android/apk/app/build.gradle.kts` 声明 minSdk 24；唯一解码路径依赖 API 30 引入的 `AImageDecoder`，缺符号时直接失败，没有 fallback。审查时也核对了本机 NDK `android/imagedecoder.h` 中的 `__INTRODUCED_IN(30)`。 | 明确每个 API 等级的能力矩阵：提高最低版本，或提供实际可用的低版本平台解码器。不能只保证 `.so` 能加载。 |
| G08 | **P1，若要发布** | **release 构建仍使用 debug signing。** `crates/android/apk/app/build.gradle.kts:52` 直接使用 debug signingConfig；构建脚本还固定 gallery bundle。它是示例宿主构建，不是通用应用交付系统。 | 分离框架 SDK、示例应用和用户应用；应用配置注入包名、签名、资产、权限、入口与模块；CI 验证最终 release 产物。 |
| G09 | **P2** | **开发工具没有完整覆盖执行环境。** `apps/gallery-solid/build.mjs --watch` 只重建文件；没有找到完整的运行中 reload/HMR、JS 调试协议、断点与 CPU/heap profiling 链路。当前面板不能替代这些能力。 | 区分 build watch、重新启动会话、热替换和调试协议，逐层实现。源码映射应进入实际脚本异常链路，并对原生/JS 内存建立可关联的身份。 |

审查基线上没有看到可交付的 iOS、Windows、Linux 宿主路径。**这属于平台范围，不应被包装成已经支持的跨平台矩阵，也不应挤占上述 P1 缺陷的修复顺序。**

## 五、测试体系本身的问题

**测试会制造比实际能力更乐观的结果。**

| 编号 | 级别 | 问题 | 建议 |
| --- | --- | --- | --- |
| T01 | **P1** | `.github/workflows/check.yml` 不安装 QuickJS 和 Impeller。很多真实运行路径被 cfg 排除。审查机器实际链接后的 2 个 QuickJS 失败，默认 CI 无法提供同等保障。 | 保留无 SDK 检查，但必须增加 linked QuickJS、linked Impeller 和 Android 构建/运行任务。SDK 缺失应显示为能力未测试，不应等同测试通过。 |
| T02 | **P1** | JS 测试输出运行时异常仍然成功；此外 `packages/solid-ui/package.json` 使用 `for ...; do node ...; done`，没有 fail-fast，中间测试失败可以被最后一项成功覆盖。 | 测试 runner 统一收集退出码；未捕获异常、未处理 rejection、框架 fault 必须导致失败。mock host 的接口应由同一 schema 生成，避免再次漏掉 `setTextRuns`。 |
| T03 | **P2** | 测试主要通过类型剥离运行 TS；没有把严格 TS 检查接入当前 CI。`Field` 实际五参数回调与 JSX 中单参数声明等漂移没有被构建入口阻止。 | 固定 TypeScript 版本，CI 执行 `tsc --noEmit`；区分 host runtime、Node 测试和应用编译配置。禁止用 `as unknown as` 长期绕过跨语言契约。 |
| T04 | **P2** | 当前 CI 无法验证真正影响框架交付的闭环：图片晚到、reload、实际输入法、TalkBack、平台恢复、半透明图片一致性、长时资源增长。 | 建立少量真正的端到端框架测试应用，而不是只依赖 gallery 的业务表现。像素 golden、交互事件回放、资源计数与性能阈值分别验收。 |

审查时 LSP 没有提供有效的 TS 诊断，因此没有将其计为类型检查通过。本机相关默认服务器不可用；需要安装所需服务器或调整 `pi-lsp.json` 的命令配置。

## 六、命名和接口细节

命名不是第一优先级，但有些名字正在隐藏错误的契约。

| 当前接口 | 问题 | 建议 |
| --- | --- | --- |
| `Tree::rect` 与 `scene_rect` | 前者返回的累计布局位置排除 scroll/transform，名字却像最终矩形，已经在语义与输入代码中造成误用。 | 明确 `layout_rect` / `visual_rect` / local-to-scene 变换；进一步用坐标空间类型区分。 |
| `core.reload(source)` | 实际只重新加载图片，容易被理解成应用/脚本重载。 | 使用 `reload_image` / `reloadImage`，把资源重载与会话重载分开。 |
| `CoreHost`、导出的 `host`、另一份 `core` | 同样叫 core 的对象，有的返回裸数字，有的返回 renderer node，所有权和适用层不清晰。 | 明确原生绑定与应用 renderer API；应用默认不接触裸 ID 和底层 bridge。 |
| `useFetch` | 只是 `createFetch` 的别名，没有 owner 清理或 hook 生命周期；默认参数还会覆盖调用方的 signal。 | 保留准确的 factory 名；若提供 owner 绑定版本，真正实现清理及 signal 组合。 |
| 数字 opcode、role、轴编号和 `-1/0` 哨兵 | TS、C、Rust 各自理解 magic number，签名无法区分节点、driver、编辑命令和普通整数。 | schema 生成枚举与 branded handle；内部使用领域类型，只有最外层编码时降为整数。 |
| `"bad ..."`、`NotReady`、裸 `String` 错误 | 同一失败丢掉节点、属性、阶段、资源和底层原因，调用方难以判断重试还是修数据。 | 错误包含 category、operation、entity、cause、retryability；绑定层保留结构，最后才映射为 JS Error。 |

**不要把重构变成批量改名。** 先修语义，再让名字表达已经稳定的语义。

## 七、Flutter / React Native 参考源码

不建议照抄它们的整个对象体系，也不建议为了像 React Native 而重新引入一套 view diff。

应该借鉴的是已经被验证过的边界。以下路径均为审查机器上的绝对路径，行号对应开头所列提交。

| 参考内容 | 完整本地源码路径 | 值得借鉴的具体点 |
| --- | --- | --- |
| Flutter `PipelineOwner::flushLayout` | `/Users/zhangfan/develop/github.com/flutter/packages/flutter/lib/src/rendering/object.dart:1138` | 脏节点集合、处理顺序、layout 阶段的约束，而不是依赖全局 dirty 后再跑一次。 |
| Flutter `GestureArenaManager` | `/Users/zhangfan/develop/github.com/flutter/packages/flutter/lib/src/gestures/arena.dart:117` | recognizer 的接受、拒绝、等待，以及明确的竞争生命周期。 |
| Flutter `GestureArenaTeam` | `/Users/zhangfan/develop/github.com/flutter/packages/flutter/lib/src/gestures/team.dart:139` | recognizer 的组合关系。 |
| Flutter `TextInputConnection` | `/Users/zhangfan/develop/github.com/flutter/packages/flutter/lib/src/services/text_input.dart:1695` | 输入连接身份、当前客户端、编辑状态、光标和变换分别建模。 |
| Flutter `SemanticsNode` 的 transform 与 clip | `/Users/zhangfan/develop/github.com/flutter/packages/flutter/lib/src/semantics/semantics.dart:2947` | 语义节点具有自己的 transform、clip、hidden 和 dirty 状态。 |
| Flutter Android accessibility focus | `/Users/zhangfan/develop/github.com/flutter/engine/src/flutter/shell/platform/android/io/flutter/view/AccessibilityBridge.java:1045` | accessibility focus 不等于输入焦点；有状态、有事件、有稳定节点身份。 |
| Flutter `ImageCache` | `/Users/zhangfan/develop/github.com/flutter/packages/flutter/lib/src/painting/image_cache.dart:86` | 图片缓存的条目、字节预算与生命周期。 |
| Flutter `RestorationManager` | `/Users/zhangfan/develop/github.com/flutter/packages/flutter/lib/src/services/restoration.dart:158` | 状态恢复的独立职责。 |
| Flutter Android 保存实例状态 | `/Users/zhangfan/develop/github.com/flutter/engine/src/flutter/shell/platform/android/io/flutter/embedding/android/FlutterActivityAndFragmentDelegate.java:692` | 平台实例状态与框架、插件恢复数据的连接。 |
| React Native `MountingCoordinator` | `/Users/zhangfan/develop/github.com/react-native/packages/react-native/ReactCommon/react/renderer/mounting/MountingCoordinator.h:44–133` | revision、pending transaction、提交确认和撤销边界。这里应借提交语义，不必借它的 ShadowTree diff。 |
| React Native `Scheduler` | `/Users/zhangfan/develop/github.com/react-native/packages/react-native/ReactCommon/react/renderer/scheduler/Scheduler.h:41` | UIManager 与提交调度的接口边界。 |
| React Native `EventBeat` | `/Users/zhangfan/develop/github.com/react-native/packages/react-native/ReactCommon/react/renderer/core/EventBeat.h:40` | 事件触发与运行时调度之间的边界。 |

## 八、建议的目标结构

以下是建议方向，不是现有代码已经具备的能力。

```text
Solid
  │ 属性更新 / 事件订阅
  ▼
生成的、带版本的绑定契约
  │
  ▼
语言会话
  ├─ JS 执行与异步工作
  └─ 事件分发
  │
  ▼
通用 UI 帧管线
  ├─ 有序输入 / Focus / TextInputClient
  ├─ Tree mutation
  ├─ Layout 与失效传播
  ├─ Gesture / Scroll / ListDriver
  ├─ Picture 与资源依赖
  └─ Semantics
  │
  ▼
带 revision 的帧提交
  ├─ LayerTree
  ├─ 可交互几何
  ├─ Semantics patch
  └─ 资源 revision
  │
  ▼
Engine / 平台宿主
```

最重要的不是新增多少模块，而是四个约束：

1. **同一能力只有一个权威状态。** 编辑状态不能同时由 Rust editor、TS controller、Java editable 各自解释。
2. **所有缓存都能解释为什么失效。** source 一样、picture generation 一样，不足以覆盖资源替换。
3. **所有异步工作都能解释归谁所有、何时取消。** 请求、socket、watchdog、模块调用不能只是启动后丢掉句柄。
4. **帧失败必须有一致的外部行为。** 不能画旧内容，却让输入和无障碍操作新内容。

## 九、建议的推进顺序

不建议先补齐 Flutter 控件。

### 第一阶段：先使现有功能可信

优先处理：

- D01：存储丢写。
- D02、D04、D06、D07、D08：真实网络链路。
- D03：图片 arrival/reload 失效。
- D13、D14、D15：Solid 属性契约。
- D05：正常微任务失败。
- T01、T02：测试不能继续假绿。

**验收标准：这些复现全部转成永久回归测试，且 CI 确实执行真实实现。**

### 第二阶段：修正输入和帧状态模型

- 统一输入顺序。
- 建立 TextInputClient 身份与 revision。
- 修复编辑事务、选区和 IME 几何。
- 建立提交版本与失败行为。
- 修正语义坐标和 Android accessibility focus。

**验收标准：快速切焦点、输入法组合、滚动、动画和资源等待同时发生时，画面与交互仍一致。**

### 第三阶段：做规模化，而不是继续增加局部分支

- List 数据提交从平方复杂度降下来。
- 去掉无必要的全树扫描和扁平镜像。
- 图片依赖、语义 patch、任务取消形成统一机制。
- 用同一个 schema 生成真实类型契约。
- 建立长时运行和多次挂载/销毁测试。

### 第四阶段：再建设框架产品能力

- 动态包安装、版本协商与回滚。
- 自定义绘制与平台扩展面。
- 应用构建工具与 release 产物。
- 状态恢复、开发重载、调试协议。
- 按明确平台矩阵扩展宿主。

## 最终判断

**不应该把“能调用某个 API”作为“支持某项能力”的标准。**

审查中最能说明问题的不是某个文件很长，而是：

- 写入返回成功，数据却丢了。
- 测试返回成功，内部却抛了异常。
- 图片请求完成，画面却没有失效。
- 节点位置改变，语义位置却没改变。
- 声明式属性消失，旧行为却还在。

**先把这些跨层契约做成可验证的闭环，Impasto 才能从 demo 进入框架阶段；否则增加的每项功能，都只是扩大需要同时维护的不一致状态。**
