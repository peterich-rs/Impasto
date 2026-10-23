package dev.uifw.gallery

import com.lightningkite.reactive.core.Signal
import dev.uifw.scene.counterScene

fun main() {
    val count = Signal(0)
    print(counterScene(count).debug())
    count.value = 1
    print(counterScene(count).debug())
}
