# Engine

播放器把 `protocol::DrawList` 画成 Impeller DisplayList。它不解析场景字节，也不依赖 `core`。`engine.pin` 里的 Flutter commit 对应一份不带 Dart VM 的预编译包。

```sh
engine/fetch-impeller-sdk.sh
just host
```

`UIFW_DUMP=/tmp/uifw-frame.png just host` 会在画出第一帧后把 Metal 纹理写成 PNG 并退出。

没有 SDK 时 `Engine::create_metal` 返回 `NotLinked`。macOS 调用 `draw_metal`。Android 目标调用 `create_vulkan` 和 `draw_vulkan`。图片像素由宿主解码后以 RGBA8888 交给 `Engine::submit`。Impeller 不接受 PNG 字节。文字命令自带字号和段落宽。版本 1 映射把没有样式的夹具文字填成字号 28、段落宽 280。播放器按命令绘制，不另设默认字号。命令和失败码见 `docs/native-phase.md`。
