# 上游对照

对照 `packages/reactive` 的作者 API 时，将 Solid 检出到本地 `third_party/solid`。

```sh
git clone --branch next https://github.com/solidjs/solid.git third_party/solid
```

Impeller SDK 由 `crates/engine/fetch-impeller-sdk.sh` 解到 `third_party/impeller-sdk/`，不进 git。JavaScript 解释器使用 Cargo 发布依赖 quickjs-jit 0.14.2，不从本目录编译源码。
