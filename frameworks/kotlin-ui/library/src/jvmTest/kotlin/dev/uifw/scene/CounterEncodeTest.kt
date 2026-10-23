package dev.uifw.scene

import java.nio.file.Files
import java.nio.file.Path
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.fail

class CounterEncodeTest {
    @Test
    fun matchesProtocolEncoder() {
        val path = System.getenv("UIFW_COUNTER_BYTES") ?: fail("UIFW_COUNTER_BYTES is not set")
        val expected = Files.readAllBytes(Path.of(path))
        assertContentEquals(expected, counterScene().encode())
    }
}
