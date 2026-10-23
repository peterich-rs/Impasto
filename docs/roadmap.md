# 已收口的缺口

下面这些已经落在当前代码里。场景字节仍是版本 1。

- 右键单独记一次按下。滑出阈值后抬起不产生点击。`Gesture::SecondaryClick` 的 kind 是 14，TypeScript 属性是 `onSecondaryClick`。macOS 内容视图用 tracking area，在 `mouseExited:` 投递窗口外的 `Move`。
- QuickJS 栈上限是 1 MiB。帧回调失败时记下脚本错误并返回上一帧的显示列表，不在这次失败上布局。
- `core/src/tree/` 按节点、槽位、布局、命中、手势、录制拆开。`Tree` 的私有字段仍在 `mod.rs`。
- 一帧要不要提交、重画或停掉 vsync，由 `frame_step` 和 `present_followup` 决定。测试不创建 `Engine`。
- Android 把 `MotionAction::Scroll` 的滚轮轴换成场景点。`HSCROLL` / `VSCROLL` 的一格是 48 点。脚本先读 intent extra `uifw.script`，再是 `app.js`，最后是 `counter.js`。
- `pixels_close` 比较解码后的 RGBA。`just pixel-test` 用 `UIFW_DUMP` 对 `hosts/macos/tests/pixels/counter.png`，不进 `just check`。
- `protocol::Error::BadVersion` 携带读到的版本号。
- `quickjs::open` 接收脚本文件名。macOS 用启动参数里的路径，Android 用实际读到的 asset 名。`build.mjs` 打开 sourcemap，保留 `--watch`。渲染线程不读 `.map`。
- `Switch` 和 `Dialog` 在 `frameworks/solid-ui`。列表仍用 `solid-js` 的 `<For>`，重排测试在宿主树测试里；没有第二套 key 表。
- Kotlin `jvmMain` 钉 `com.lightningkite:reactive` 5.1.2。信号里的计数值通过现有 `scene { }` 重建。`count == 0` 与 `counter.bin` 逐字节相同。

## 明确不做

- 多窗口。
- 场景字节 v2。必须下发样式或变换、且 Rust、Kotlin、TypeScript 和金样四路同时改时，先写 `docs/protocol-v2.md`。
- 场景 diff。
- iOS 宿主。上游预构建矩阵还没有这一列。
- 把脚本移出渲染线程。
- 滚轮惯性。`onWheel` 现在会停掉 coasting，这条不改。
