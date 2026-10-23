package dev.uifw.scene

import com.lightningkite.reactive.core.Signal
import java.nio.file.Files
import java.nio.file.Path
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.fail

class CounterEncodeTest {
    @Test
    fun matchesProtocolEncoder() {
        val path = System.getenv("UIFW_COUNTER_BYTES") ?: fail("UIFW_COUNTER_BYTES is not set")
        val expected = Files.readAllBytes(Path.of(path))
        assertContentEquals(expected, counterScene().encode())
    }

    @Test
    fun signalZeroMatchesTheGoldenAndOneRewritesTheGreenText() {
        val path = System.getenv("UIFW_COUNTER_BYTES") ?: fail("UIFW_COUNTER_BYTES is not set")
        val expected = Files.readAllBytes(Path.of(path))
        val count = Signal(0)
        assertContentEquals(expected, counterScene(count).encode())
        count.value = 1
        val next = counterScene(count)
        val green = next.nodes[3]
        assertEquals(counterScene().nodes[3].copy(text = "1"), green)
        next.nodes.forEachIndexed { index, node ->
            if (index != 3) {
                assertEquals(counterScene().nodes[index], node)
            }
        }
    }
}
