# kotlin-ui

Kotlin 声明式 UI。界面 DSL、信号和原生目标还没接。Gradle 目前只开 JVM。`just kotlin` 跑 `:library:jvmTest`。

跑起来的界面在 Solid 路线，直接改 `uicore` 的渲染树。这条路线暂缓，见 `docs/refactor-plan.md`；重启时走 `crates/quickjs/abi.yml` 的变更缓冲，信号用已发布的 `com.lightningkite:reactive`。
