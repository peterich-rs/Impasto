# 工具链对照

`tools/toolchain-baseline.py` 对同一份 gallery 执行点击和滚轮输入。旧版本从提交 `18a509fa03cd2556453ea0adba9ae991e7a8800e` 导出，只补计数；新版本使用工作区实现。旧版直接点击市场标签，新版从更多菜单进入市场，计数都采集打开市场的帧。旧 bundle 包含 `solid-js` 和 `@solidjs/universal`，由 JS 调用 `flush`；新 bundle 使用 `reactive` 和 `scene` 原生模块。

## 结果

当前解释器由 Cargo 的 quickjs-jit 0.14.2 依赖提供。本文不保留其他解释器的耗时数据；当前版本的计数和耗时以运行脚本生成的 `target/toolchain-baseline/results.json` 为准。

setter 在场景变更进入 `Tree` 的边界计数，不含节点创建、挂载、布局内部操作、文字测量、`motions` 的弹簧采样和效果服务。`JsSession::frame` 在帧监听前、布局监听后各冲刷一次响应式图，空队列不计更新轮次。

## 复现

当前实现由 Cargo 构建解释器。安装 JS 依赖后运行：

```sh
just gallery-dist
python3 tools/toolchain-baseline.py
```

也可以运行 `just toolchain-baseline`。脚本把旧提交导出到 `target/toolchain-baseline/before`，按旧 lockfile 安装依赖并构建；不会改产品源码。两边使用 `tools/toolchain-benchmark.rs` 的同一组输入，分别在独立进程与独立图里执行。结果写入 `target/toolchain-baseline/results.json`，完整输出写入同目录的 `before.log` 和 `after.log`。结果记录当前工作区的 `HEAD`。新版场景通过 `cargo test -p javascript --features ui --lib toolchain_baseline_counts -- --ignored --nocapture` 运行。

`impasto run --trace` 输出运行时每帧的 `script_us`、`updates`、`sets`；缺省关闭。Android 的 trace 开关随运行构建写入 `app.trace`，macOS 通过 `IMPASTO_TRACE` 传给会话。滚动偏移与弹簧仍在 `uicore::Tree::motions`，不经 signal 每帧写回。
