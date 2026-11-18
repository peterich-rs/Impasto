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

Solid 用来对照 `packages/reactive` 的作者 API。KiteUI 用来对照 Kotlin lambda 的写法。运行界面更新 `uicore::Tree`。

Impeller SDK 由 `crates/engine/fetch-impeller-sdk.sh` 解到 `third_party/impeller-sdk/`，不进 git。JavaScript 解释器使用 Cargo 发布依赖 quickjs-jit 0.14.2，不从本目录编译源码。
