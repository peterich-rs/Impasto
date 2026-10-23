package dev.uifw.scene

import com.lightningkite.reactive.core.Signal

/** Rebuilds the counter scene from the current signal value. The green text is the count. */
fun counterScene(count: Signal<Int>): Scene = scene {
    layer {
        rect(0f, 0f, 320f, 180f, Color(17, 17, 17))
        text(24f, 36f, "Count", Color.White)
        text(24f, 72f, count.value.toString(), Color(124, 255, 107))
        image(240f, 24f, 56f, 56f, "asset:logo")
    }
}
