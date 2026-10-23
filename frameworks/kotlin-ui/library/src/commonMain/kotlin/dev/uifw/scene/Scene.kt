package dev.uifw.scene

/** Version 1 scene bytes. The magic is UIFW. See docs/native-phase.md. */

data class Color(val r: Int, val g: Int, val b: Int, val a: Int = 255) {
    init {
        require(r in 0..255 && g in 0..255 && b in 0..255 && a in 0..255)
    }

    companion object {
        val Clear = Color(0, 0, 0, 0)
        val White = Color(255, 255, 255, 255)
    }
}

enum class NodeKind(val code: Int) {
    Layer(1),
    Rect(2),
    Text(3),
    Image(4),
}

data class Node(
    val id: Int,
    val parent: Int,
    val kind: NodeKind,
    val x: Float,
    val y: Float,
    val w: Float,
    val h: Float,
    val color: Color,
    val text: String,
)

class Scene(val nodes: List<Node>) {
    fun encode(): ByteArray {
        val out = Buf()
        out.u8('U'.code)
        out.u8('I'.code)
        out.u8('F'.code)
        out.u8('W'.code)
        out.u32(1)
        out.u32(nodes.size)
        for (node in nodes) {
            val utf8 = node.text.encodeToByteArray()
            out.u32(node.id)
            out.u32(node.parent)
            out.u8(node.kind.code)
            out.u8(0)
            out.u8(0)
            out.u8(0)
            out.f32(node.x)
            out.f32(node.y)
            out.f32(node.w)
            out.f32(node.h)
            out.u8(node.color.r)
            out.u8(node.color.g)
            out.u8(node.color.b)
            out.u8(node.color.a)
            out.u32(utf8.size)
            out.bytes(utf8)
        }
        return out.toByteArray()
    }

    fun debug(): String = buildString {
        append("UIFW version=1 nodes=")
        append(nodes.size)
        append('\n')
        for (node in nodes) {
            append(node.id)
            append(' ')
            append(node.kind.name.lowercase())
            append(" parent=")
            append(node.parent)
            append(" box=")
            append(node.x)
            append(',')
            append(node.y)
            append(',')
            append(node.w)
            append(',')
            append(node.h)
            append(" rgba=")
            append(node.color.r)
            append(',')
            append(node.color.g)
            append(',')
            append(node.color.b)
            append(',')
            append(node.color.a)
            append(" \"")
            append(node.text)
            append("\"\n")
        }
    }
}

@DslMarker
annotation class SceneDsl

@SceneDsl
class SceneBuilder {
    private val nodes = mutableListOf<Node>()
    private val parents = ArrayDeque<Int>()

    private fun parent(): Int = parents.lastOrNull() ?: 0

    private fun add(kind: NodeKind, x: Float, y: Float, w: Float, h: Float, color: Color, text: String): Int {
        val id = nodes.size + 1
        nodes += Node(id, parent(), kind, x, y, w, h, color, text)
        return id
    }

    fun layer(block: SceneBuilder.() -> Unit = {}): Int {
        val id = add(NodeKind.Layer, 0f, 0f, 0f, 0f, Color.Clear, "")
        parents.addLast(id)
        try {
            block()
        } finally {
            parents.removeLast()
        }
        return id
    }

    fun rect(x: Float, y: Float, w: Float, h: Float, color: Color): Int =
        add(NodeKind.Rect, x, y, w, h, color, "")

    fun text(x: Float, y: Float, text: String, color: Color): Int =
        add(NodeKind.Text, x, y, 0f, 0f, color, text)

    fun image(x: Float, y: Float, w: Float, h: Float, source: String): Int =
        add(NodeKind.Image, x, y, w, h, Color.Clear, source)

    internal fun build(): Scene = Scene(nodes.toList())
}

fun scene(block: SceneBuilder.() -> Unit): Scene = SceneBuilder().apply(block).build()

fun counterScene(): Scene = scene {
    layer {
        rect(0f, 0f, 320f, 180f, Color(17, 17, 17))
        text(24f, 36f, "Count", Color.White)
        text(24f, 72f, "0", Color(124, 255, 107))
        image(240f, 24f, 56f, 56f, "asset:logo")
    }
}

private class Buf {
    private var data = ByteArray(256)
    private var size = 0

    fun u8(value: Int) {
        if (size == data.size) {
            data = data.copyOf(data.size * 2)
        }
        data[size++] = value.toByte()
    }

    fun u32(value: Int) {
        u8(value)
        u8(value ushr 8)
        u8(value ushr 16)
        u8(value ushr 24)
    }

    fun f32(value: Float) {
        u32(value.toBits())
    }

    fun bytes(value: ByteArray) {
        for (byte in value) {
            u8(byte.toInt() and 0xff)
        }
    }

    fun toByteArray(): ByteArray = data.copyOf(size)
}
