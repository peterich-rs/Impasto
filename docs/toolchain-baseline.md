# 工具链对照

`tools/toolchain-baseline.py` 对同一份 gallery 执行点击和滚轮输入。旧版本从提交 `18a509fa03cd2556453ea0adba9ae991e7a8800e` 导出，只补计数；新版本使用工作区实现。旧 bundle 包含 `solid-js` 和 `@solidjs/universal`，由 JS 调用 `flush`；新 bundle 使用 `reactive` 和 `scene` 原生模块。

## 结果

2026-11-16，在 macOS arm64、Rust 1.97.0、钉住的 quickjs-ng 上测得。两个场景分别创建会话，使用 `MonoShaper`、360×640 的视口。首帧后，在 16 ms 点击「市场」，或者在 `(180, 300)` 输入 Y 增量 48 的滚轮事件，只统计这次输入后的帧。

| 场景与指标 | 修改前 | 修改后 |
|---|---:|---:|
| 点击：`flush` / 非空 `run_updates` | 2 | 2 |
| 点击：场景 setter | 355 | 341 |
| 点击：QuickJS 微秒 | 22236.5 | 6257.375 |
| 滚动：`flush` / 非空 `run_updates` | 2 | 1 |
| 滚动：场景 setter | 282 | 267 |
| 滚动：QuickJS 微秒 | 5779.624 | 3456.584 |
| 合计：更新轮次 | 4 | 3 |
| 合计：场景 setter | 637 | 608 |

更新轮次合计减少 25%，setter 合计减少约 4.6%。点击仍需要两轮更新。帧监听之前与布局监听之后的两个冲刷位置都保留，空队列不算更新轮次。时间是一次运行的记录，不是性能保证。

setter 在 QuickJS 场景变更进入 `Tree` 的边界计数，包括 `Style`、`SetText`、`SetImage`、`Recognizers`。两边使用相同口径；不含节点创建、挂载、布局内部操作、文字测量、`motions` 的弹簧采样和效果服务。新桥先读 `Tree` 的现值，跳过相同的纯文字和颜色写入；没有新增属性缓存或视图树。

## 复现

先取仓库钉住的 QuickJS 与 Impeller SDK，并安装 JS 依赖，然后运行：

```sh
just gallery-dist
python3 tools/toolchain-baseline.py
```

也可以运行 `just toolchain-baseline`。脚本把旧提交导出到 `target/toolchain-baseline/before`，按旧 lockfile 安装依赖并构建；不会改产品源码。两边使用 `tools/toolchain-benchmark.rs` 的同一组输入，分别在独立进程与独立图里执行。结果写入 `target/toolchain-baseline/results.json`，完整输出写入同目录的 `before.log` 和 `after.log`。对照脚本使用 `HEAD`，换提交后应同时记录新的提交号。

`impasto run --trace` 输出运行时每帧的 `script_us`、`updates`、`sets`；缺省关闭。Android 的 trace 开关随运行构建写入 `app.trace`，macOS 通过 `IMPASTO_TRACE` 传给会话。滚动偏移与弹簧仍在 `uicore::Tree::motions`，不经 signal 每帧写回。
