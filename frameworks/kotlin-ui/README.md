# kotlin-ui

Kotlin 声明式 UI。信号和原生目标还没接。现在这份 DSL 把场景收成版本 1 字节，用来和 `protocol/tests/fixtures/counter.bin` 对拍。

```kotlin
scene {
    layer {
        rect(0f, 0f, 320f, 180f, Color(17, 17, 17))
        text(24f, 36f, "Count", Color.White)
    }
}
```

跑起来的界面在 Solid 路线，直接改 `core` 的渲染树。Gradle 目前只开 JVM。`just kotlin` 会跑 `:library:jvmTest` 和 `:gallery:runGallery`。
