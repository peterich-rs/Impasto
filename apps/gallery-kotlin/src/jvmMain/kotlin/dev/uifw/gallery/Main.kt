package dev.uifw.gallery

import dev.uifw.scene.counterScene

fun main() {
    val scene = counterScene()
    print(scene.debug())
    println("encoded ${scene.encode().size} bytes")
}
