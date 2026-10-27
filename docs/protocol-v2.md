# Picture 字节

未排期。渲染引擎已经能提交一张 picture，缓存的 display list 把段落留在自己的 `held` 里，`evict_unused` 不会释放它们。编解码器仍按下面的消费者顺序再排，现在不写，也不新增金样。

## 序列化什么

序列化的是一个 `Picture` 的 `DrawList`：局部坐标的矩形、文字、图片，以及这张 picture 内部的裁剪 push/pop。滚动、变换、透明度和跨 picture 的层关系属于 `LayerTree`，不写进这串字节。

没有场景字节。交互帧在内存里走 `LayerTree`。

整帧扁平 `DrawList`（绝对坐标、把滚动烘焙进子命令）不另做一套字节格式。那种格式跟合成层替换的是同一张表。

## 做什么用

只在 Rust 里编解码，入口放在 `protocol`：

- 引擎回放：解码一个 picture，直接交给 `Engine::submit_layers`，像素断言不依赖 QuickJS、会话或整棵 `Tree`。
- 渲染线程 panic 时，把当前还留着的 picture 命令写进日志，替代克隆整份列表。

TypeScript 和 Kotlin 不写对照编解码器。没有第二份实现要对字节时，不提交金样文件。

## 载荷约束

文字命令同时带 `ParagraphId` 和绘制载荷（文本、样式、颜色、spans）。回放进程里没有活的段落缓存，渲染引擎用载荷重建段落，不按 id 去取上一进程的对象。

未知 opcode 用长度前缀跳过。这条规则等真正写编解码器时再定死字段布局。现在不定 opcode 表，避免把还没稳定的 `Command` 冻成规格。
