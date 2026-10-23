# 上游对照

这两个目录是本地的 git 检出，不进本仓库。产品构建不编译它们。

| 目录 | 远程 | 分支 |
|---|---|---|
| `solid` | https://github.com/solidjs/solid | `next`（Solid 2.0） |
| `kiteui` | https://github.com/lightningkite/kiteui | `version-8.3` |

需要对照时再克隆到本目录：

```sh
git clone --branch next https://github.com/solidjs/solid.git third_party/solid
git clone --branch version-8.3 https://github.com/lightningkite/kiteui.git third_party/kiteui
```

Solid 用来对照 `@solidjs/universal` 和 `packages/signals`。KiteUI 用来对照 Kotlin lambda 的写法。跑起来的界面改 `core` 的渲染树。版本 1 字节只留给夹具编码器。

Impeller SDK 和 quickjs-ng 源码另外取：`engine/fetch-impeller-sdk.sh` 解到 `third_party/impeller-sdk/`，`just quickjs` 解到 `third_party/quickjs/`。这两处也不进 git。
