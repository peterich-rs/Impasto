# 前置改造 3：全链路网络采集（OpenWire / iOS / WebSocket / Image / NetBodyPool）

> **适用范围与前置约束**
> - 本文档细化网络全链路可观测改造方案，统一覆盖 **JS `fetch`（`@impasto/http`）、Rust 跨包 `impasto_http::client(ctx)`、`@impasto/websocket` 双向帧** 以及 **引擎远程图片加载（`https://`）**，提供 9 阶段毫秒级链路瀑布与完整请求/回包体按需查看。
> - **依赖隔离边界**：`packages/http/rust` 与 `packages/websocket/rust` 保持仅依赖 `impasto` SDK（及平台传输库 `openwire` / `objc2-foundation` / `tokio-tungstenite`），通过 `impasto::DiagnosticsSink::record` 上报自有结构，严禁直接依赖 `embedder` 或 `engine`（[check-javascript-boundaries.py](../tools/check-javascript-boundaries.py)）。
> - **隐私与内存守卫**：敏感凭证请求/响应头默认脱敏；大体积 Body 统一留在有界 LRU `NetBodyPool`（总上限 `16 MiB`），列表实时流不直接推送完整包体。

---

## 1. 改造目标与文件清单

| 目标文件 | 动作 | 核心职责 |
|---|---|---|
| [packages/http/rust/src/trace.rs](../packages/http/rust/src/trace.rs) | 修改 | 升级为 `network.start` / `network.head` / `network.finish` 三段流式上报，携带 9 阶段耗时、连接元数据、Redirect 跳列表与完整 Body |
| [packages/http/rust/src/wire/openwire_transport.rs](../packages/http/rust/src/wire/openwire_transport.rs) | 修改 | 实现 `openwire_core::EventListener` + `EventListenerFactory`，按跳提取精确耗时、远端地址、连接复用与 TLS 握手信息 |
| [packages/http/rust/src/lib.rs](../packages/http/rust/src/lib.rs) | 修改 | 为 Rust 跨包 `impasto_http::client(ctx)`（`perform` & `stream`）补全 `RequestTrace` 接入（`initiator = "rust-http"`） |
| [packages/http/rust/src/ios.rs](../packages/http/rust/src/ios.rs) · [Cargo.toml](../packages/http/rust/Cargo.toml) | 修改 | 在 `Receiver` 实现 `URLSession:task:didFinishCollectingMetrics:`，将 `NSURLSessionTaskTransactionMetrics` 映射为同构 `NetworkTiming` |
| [packages/websocket/rust/src/socket.rs](../packages/websocket/rust/src/socket.rs) · [trace.rs](../packages/websocket/rust/src/trace.rs) | 修改 | 补齐上行 `WS SEND` 追踪、帧方向/Opcode 记录与独立帧缓存事件 |
| [crates/embedder/common/src/images/remote.rs](../crates/embedder/common/src/images/remote.rs) · [host.rs](../crates/embedder/common/src/host.rs) | 修改 | 为远程图片拉取挂接 `openwire` `EventListener`，以 `initiator = "image"` 写入 `TelemetryHub` |
| [hub.rs](../crates/embedder/common/src/diagnostics/hub.rs) · [plugin_record.rs](../crates/embedder/common/src/diagnostics/plugin_record.rs) | 修改 | 支持按 `token` 对 `NetEntry` 做就地更新（upsert）；新增有界 LRU `NetBodyPool`；解析三段事件与 `network.ws_frame` |

---

## 2. 统一网络事件与 9 阶段计时模型

### 2.1 三段流式事件（向后兼容旧 `"network"`）

通过 `DiagnosticsSink::record("http", event, payload)` 发送（保留现有 `"network"` 兼容解析）：

1. **`"network.start"`**（请求提交瞬间发出）：
   - `token`（全局唯一递增请求 ID）、`initiator`（`"fetch"` | `"rust-http"` | `"image"` | `"websocket"`）、`method`、`url`、`request_headers`（脱敏）、`request_body_preview`（前 512 B）、`request_body_full`（<= 1 MiB 写入 `NetBodyPool`）、`at_mono_ms`。
2. **`"network.head"`**（收到最终响应头时发出，流式下载期间表格立即更新状态码与 TTFB）：
   - `token`、`final_url`、`redirected`、`status`、`response_headers`（脱敏）、`head_elapsed_ms`。
3. **`"network.finish"`**（请求完成或失败取消时发出）：
   - `token`、`status`、`error`、`response_bytes`、`response_preview`（前 512 B）、`response_body_full`（<= 1 MiB 写入 `NetBodyPool`）、`timing: NetworkTiming`、`connection: ConnectionMeta`、`redirects: Vec<RedirectHop>`。

### 2.2 9 阶段耗时与连接元数据（同构覆盖 OpenWire 与 iOS NSURLSession）

```rust
#[derive(Clone, Debug, Default, ImpastoType)]
pub struct NetworkTiming {
    pub queue_ms: Option<f64>,         // dispatcher_queue_start -> dispatcher_queue_end
    pub proxy_ms: Option<f64>,         // proxy_select_start -> proxy_select_end
    pub dns_ms: Option<f64>,           // dns_start -> dns_end
    pub tcp_ms: Option<f64>,           // connect_start -> tls_start (无 TLS 时为 connect_end)
    pub tls_ms: Option<f64>,           // tls_start -> tls_end
    pub request_sent_ms: Option<f64>,  // request_headers_start -> request_body_end (或 headers_end)
    pub waiting_ttfb_ms: Option<f64>,  // request 发送完成 -> response_headers_start
    pub download_ms: Option<f64>,      // response_body_start -> response_body_end (或 call_end)
    pub total_ms: f64,                 // 整次调用（含重定向与队列）总墙钟耗时
}

#[derive(Clone, Debug, Default, ImpastoType)]
pub struct ConnectionMeta {
    pub reused: bool,
    pub protocol: Option<String>,      // "h2" / "http/1.1" / "h3"
    pub remote_addr: Option<String>,   // 例如 "93.184.216.34:443"
    pub resolved_ips: Vec<String>,
    pub tls_version: Option<String>,
    pub tls_cipher: Option<String>,
    pub alpn: Option<String>,
}

#[derive(Clone, Debug, Default, ImpastoType)]
pub struct RedirectHop {
    pub index: u32,
    pub method: String,
    pub url: String,
    pub status: u16,
    pub location: Option<String>,
    pub timing: NetworkTiming,
    pub connection: ConnectionMeta,
}
```

---

## 3. 非 iOS 实现：接入 `openwire 0.1.2` `EventListener`

当前 `packages/http` 与 `crates/embedder/common` 均使用 `openwire = "0.1.2"`，其上游 `openwire_core::EventListener` 已暴露完整生命周期回调（`dispatcher_queue_*`、`proxy_select_*`、`dns_*`、`connect_*`、`tls_*`、`tls_handshake`、`request_headers_*`、`request_body_*`、`response_headers_*`、`response_body_*`、`pool_lookup`、`connection_acquired`、`call_end`/`call_failed`）。

### 3.1 避免并发串扰：按请求 `Extensions` 绑定监听器状态

由于 `OpenWireClient` 与 `images::remote` 均复用全局共享的 `openwire::Client`，绝不能在 `Client` 上存单条全局状态。具体做法：
1. 实现 `ImpastoEventListenerFactory: openwire::EventListenerFactory` 并注册到 `Client::builder().event_listener_factory(...)`；
2. 每次构建 `http::Request`（每一跳）时，生成一个每跳专属的 `Arc<Mutex<HopTimingState>>`，写入 `wire_request.extensions_mut().insert(Arc::clone(&hop_state))`；
3. `EventListenerFactory::create(&Request<RequestBody>)` 从请求扩展取出该跳的状态。HTTP 插件和远程图片共用 SDK 的监听器，每次调用各自持有状态。

### 3.2 OpenWire 回调 → 9 阶段换算规则

在 `HopTimingState` 中仅记录各事件发生时的 `Instant`：
- `queue_ms = diff(queue_start, queue_end)`
- `proxy_ms = diff(proxy_start, proxy_end)`
- `dns_ms = diff(dns_start, dns_end)`
- `tcp_ms`：**若连接复用（`reused == true` 或无 `connect_start`）则为 `None`**；否则为 `diff(connect_start, tls_start.or(connect_end))`（注：需按实测确认 OpenWire 的 `tls_start` 是在 `connect_end` 之前还是紧接 `connect_end` 之后，若 `tls_start >= connect_end` 则直接取 `diff(connect_start, connect_end)`，确保与 `tls_ms = diff(tls_start, tls_end)` 不重叠也不漏算）。
- `request_sent_ms = diff(req_headers_start, req_body_end.or(req_headers_end))`
- `waiting_ttfb_ms = diff(req_sent_end, resp_headers_start)`
- `download_ms = diff(resp_body_start.or(resp_headers_end), resp_body_end.or(call_end))`

### 3.3 Rust 跨包 `impasto_http::client(ctx)` 补盲

当前 [packages/http/rust/src/lib.rs](../packages/http/rust/src/lib.rs#L168-L205) 仅在 `HttpPlugin::send`（JS `fetch` 通道）创建了 `RequestTrace`，而直接给其他 Rust 插件（如 `examples/native-order`）使用的 `impasto_http::client(ctx)` 未接入追踪：
- 在 `HttpPlugin::init` 中把 `ctx.diagnostics_sink()` 一并捕获进 `perform` 和 `stream` 闭包，为其创建 `RequestTrace::new_with_initiator(..., "rust-http")`；
- 同时调整 [openwire_transport.rs](../packages/http/rust/src/wire/openwire_transport.rs) 中的 `HttpClient::perform` / `stream_sync`，使其与 `execute_to_sink` 共用一套重定向循环与 `HopTimingState` 采集辅助函数（避免重复逻辑）。

---

## 4. iOS 实现：接入 `NSURLSessionTaskTransactionMetrics`

1. **`packages/http/rust/Cargo.toml`**：在 `[target.'cfg(target_os = "ios")'.dependencies]` 下为 `objc2-foundation` 补充所需 feature（如 `"NSDate"`、`"NSURLSession"` 已存在，需按 `objc2-foundation 0.3.2` 精确检查 `NSURLSessionTaskTransactionMetrics` / `NSURLSessionTaskMetrics` 所属 feature 开关）。
2. **在 [ios.rs](../packages/http/rust/src/ios.rs) 的 `Receiver` 实现代理回调**：
   - 新增 `URLSession:task:didFinishCollectingMetrics:`（注意在 `Receiver` 中通过 `Option<NSURLSessionTaskMetrics>` 线程安全槽位暂存，并与 `URLSession:task:didCompleteWithError:` 协调——因为 iOS 保证 `didFinishCollectingMetrics:` 先于或紧邻 `didCompleteWithError:` 投递在同一串行 delegate queue 上，但 `stream()` 返回后若因取消提前退出也要确保不丢失已采集数据，建议在 `didCompleteWithError:` 前将 `metrics` 连同 `Received::Completed` 一起送出，或在 `didFinishCollectingMetrics:` 先发出 `Received::Metrics` 再发 `Completed`）。
3. **同构字段映射**：遍历 `metrics.transactionMetrics()`（每条对应一个 Redirect 跳或主请求）：
   - `dns_ms`：`domainLookupStartDate` → `domainLookupEndDate`
   - `tcp_ms`：`connectStartDate` → `secureConnectionStartDate`（或无 TLS 时 `connectEndDate`）
   - `tls_ms`：`secureConnectionStartDate` → `secureConnectionEndDate`
   - `request_sent_ms`：`requestStartDate` → `requestEndDate`
   - `waiting_ttfb_ms`：`requestEndDate` → `responseStartDate`
   - `download_ms`：`responseStartDate` → `responseEndDate`
   - `ConnectionMeta`：`isReusedConnection()`、`networkProtocolName()`、`remoteAddress()`（若可用）以及 TLS 套件枚举常量。

---

## 5. WebSocket 双向帧与引擎远程图片归集

### 5.1 WebSocket（[packages/websocket/rust/src/socket.rs](../packages/websocket/rust/src/socket.rs) & [trace.rs](../packages/websocket/rust/src/trace.rs)）

- 在 `SocketSession` 成功发送出站消息（`WebSocketCommand::Send` 实际写入 socket 后）调用 `trace::record_outbound(sink, id, url, text, &data)`；
- 在 [trace.rs](../packages/websocket/rust/src/trace.rs) 扩展字段：`event = "network.ws_frame"`，`direction`（`"in"` | `"out"`），`opcode`（`"text"` | `"binary"` | `"open"` | `"close"` | `"error"`），`bytes_len`，`preview`（前 512 B）与完整帧体（<= 64 KiB 存入 `TelemetryHub` 的 WS 帧缓冲）；
- `TelemetryHub` 除在主网络环登记一条聚合 `NetKind::Socket` 会话行（汇总连接状态、收发帧数与总字节数）外，另设每连接有界帧环（例如最近 200 帧），供 `net.get_ws_frames(token, since)` 按需拉取。

### 5.2 引擎远程图片加载（[images/remote.rs](../crates/embedder/common/src/images/remote.rs) & [host.rs](../crates/embedder/common/src/host.rs)）

- 将 `Option<Arc<TelemetryHub>>` 传入 `decode_one` → `images::remote::fetch(source, telemetry.as_deref())`；
- 为 `images::remote::fetch` 内的共享 `openwire::Client` 注册同款 `ImpastoEventListenerFactory`，并记录 `network.start` / `network.head` / `network.finish`（`initiator = "image"`）；图片原始字节本身已在图像缓存中，可标记跳过存入 `NetBodyPool` 或仅存 128 B 魔数预览以节省内存。

---

## 6. `TelemetryHub` Upsert 与分离式 `NetBodyPool`

### 6.1 按 `token` 就地更新（Upsert）

在 [hub.rs](../crates/embedder/common/src/diagnostics/hub.rs) 的 `NetEntry` 增加 `changed_at: u64`（对齐 `FrameSample::changed_at` 的增量游标模式）与 `state: NetState`（`Pending` | `Streaming` | `Completed` | `Failed` | `Canceled`）：
- `SeqField for NetEntry` 返回 `self.changed_at`；
- 收到 `network.start` 时 `push` 新条目；收到 `network.head` 或 `network.finish` 时逆序查找同 `token` 条目，就地更新其状态/响应头/计时并将 `changed_at` 置为新 `seq`（若原条目已被环覆盖则不回溯，按新条目插入或忽略），同步 `self.bump()`。

### 6.2 分离式有界 `NetBodyPool` 与安全脱敏

```rust
pub const NET_BODY_POOL_BYTES: usize = 16 * 1024 * 1024; // 总上限 16 MiB
pub const NET_SINGLE_BODY_LIMIT: usize = 1 * 1024 * 1024; // 单条上限 1 MiB

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BodySide {
    Request,
    Response,
}

pub(crate) struct NetBodyPool {
    budget_bytes: usize,
    used_bytes: usize,
    order: VecDeque<(u64, BodySide)>,
    bodies: HashMap<(u64, BodySide), Arc<[u8]>>,
}
```

- 插入新 Body 超过 `NET_SINGLE_BODY_LIMIT` 时截断并标注 `truncated = true`；
- 总字节超出 `16 MiB` 时从 `order` 头部淘汰最旧项，并在对应 `NetEntry` 标记 `request_body_evicted` / `response_body_evicted = true`；
- **安全脱敏守卫**：`redact` 默认继续对 `Authorization`、`Proxy-Authorization`、`Cookie`、`Set-Cookie` 等敏感头打码 `***`（见 [trace.rs:86](../packages/http/rust/src/trace.rs#L86)）；仅当 Debug 构建显式设置 `IMPASTO_DEVTOOLS_RAW_HEADERS=1` 时透出原文，防止误把线上凭证录进离线会话包。

---

## 7. 测试与验证清单

1. **HTTP / WS 单元测试**：
   - `cargo test -p impasto-http`：构造本地 TCP 测试桩，覆盖直连、302 重定向两跳、Chunked 分块流、超时/取消，断言 `NetworkTiming` 各阶段非负且与总耗时匹配、复用长连接时 `reused == true` 且 `dns_ms/tcp_ms/tls_ms` 为 `None`、Rust `impasto_http::client` 同样产出 `RequestTrace`。
   - `cargo test -p impasto-websocket`：验证上下行帧均记录 `direction` 与字节数。
2. **TelemetryHub 单元测试**：
   - `cargo test -p embedder`：验证 `token` upsert 推进 `changed_at` 游标、超出 `16 MiB` 时 `NetBodyPool` 按 FIFO 精确淘汰旧 Body 且内存严格封顶。
3. **边界与 Apple 构建验证**：
   ```sh
   just boundaries
   just check
   just ios-build-check
   ```
