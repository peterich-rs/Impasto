# Engine

`engine` 把 `protocol::LayerTree` 画成 Impeller DisplayList。它不解析版本 1 场景字节，也不依赖 `core`。`Engine::submit_layers` 按 `PictureId` 和 generation 缓存子 display list，根 builder 用 `DrawDisplayList` 摊进去。`engine.pin` 里的 Flutter 提交对应一份不带 Dart VM 的预编译包。

```sh
engine/fetch-impeller-sdk.sh
just host
```

`UIFW_DUMP=/tmp/uifw-frame.png just host` 会在画出第一帧后把 Metal 纹理写成 PNG 并退出。

没有 SDK 时 `Engine::create_metal` 返回 `NotLinked`。macOS 调用 `draw_metal`。Android 目标调用 `create_vulkan` 和 `draw_vulkan`。图片像素由宿主解码后以 RGBA8888 交给 `submit_layers`。Impeller 不接受 PNG 字节。文字命令自带字号和段落宽。渲染引擎按命令绘制，不另设默认字号。测量和段落缓存在 `Typography`，`Engine` 只编排 `submit_layers`。命令以 `protocol/src/draw_list.rs` 为准，层树以 `protocol/src/layer.rs` 为准，失败码以 `engine/src/status.rs` 为准。
