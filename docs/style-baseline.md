# 样式验证

布局与样式的可观察合同在 `crates/engine/src/style_contract_test.rs`、`style/tests.rs` 和 `crates/engine/tests/fixtures/ui/style.tsx`。QuickJS fixture 经实际 Babel/esbuild 编译后在原生会话中执行，不使用 Node 布局替身。

```sh
cargo test --locked --workspace
cargo clippy --locked -p engine --all-targets --all-features -- -D warnings
cargo fmt --all --check
cargo run --locked -p engine --bin style-types -- --check --root .
cargo doc --locked --no-deps -p engine --features js
pnpm -r test
pnpm exec tsc --noEmit -p packages/solid-renderer
pnpm exec tsc --noEmit -p packages/solid-ui
pnpm exec tsc --noEmit -p packages/solid-router
```

本机验证：Rust workspace 492 项通过、8 项既有忽略；JS 包测试、上述三个 TypeScript 源码工程、生成一致性、rustfmt、uicore 严格 Clippy 和两份库文档通过。全仓 Clippy 尚未通过，quickjs、engine、reactive 等模块仍有 unsafe 注释、参数数量和测试类型复杂度等 lint 错误，不能把 `just check` 记为通过。

回归覆盖来源跨节点隔离、旧句柄拒绝、独立清理、失败提交回滚、clear/reset 恢复展示属性、片段与复杂值传输、同步几何读、环境与交互依赖、overflow 命中和组合效果作用域。连续一千次 patch 同一字段后只保留当前声明。纯色不触发布局，已存在的透明层改值复用 picture。

macOS 通过 `IMPASTO_DUMP` 导出 Impeller 首帧，检查自选标题、指数卡片、虚拟列表、价格与底栏。Android 截图与交互、多轮固定负载性能记录尚未完成。首帧冷启动计时不能代替稳态性能基线；现有 `record_bench` 只衡量录制，不是 Taffy 冷布局证明。
