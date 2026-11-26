package dev.impasto.host

import android.content.Context
import android.graphics.Rect
import android.util.Log
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import kotlin.math.roundToInt

/** Creates a native view on the main thread when a matching scene node is mounted. */
fun interface PlatformViewFactory {
    fun create(context: Context): View

    /** Releases resources owned by the view after it leaves the hierarchy. */
    fun dispose(view: View) {}
}

internal class PlatformViewLayer(context: Context, private val factories: Map<String, PlatformViewFactory>) :
    FrameLayout(context) {
    private class Mounted(
        val kind: String,
        val factory: PlatformViewFactory,
        val view: View,
        val clip: FrameLayout,
    )

    private val mounted = mutableMapOf<Long, Mounted>()
    private val failed = mutableMapOf<Long, String>()

    init {
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
        clipChildren = true
    }

    fun update(ids: LongArray, kinds: Array<String>, geometry: FloatArray, density: Float) {
        val live = ids.toSet()
        for (id in mounted.keys.toList()) {
            if (id !in live) remove(id)
        }
        failed.keys.retainAll(live)

        ids.forEachIndexed { index, id ->
            val kind = kinds[index]
            if (mounted[id]?.kind != kind) remove(id)
            val slot = mounted[id] ?: mount(id, kind) ?: return@forEachIndexed
            val base = index * 8
            val rect = pixels(geometry, base, density)
            val clip = pixels(geometry, base + 4, density)
            val visible = clip.intersect(0, 0, width, height) && !rect.isEmpty
            slot.clip.visibility = if (visible) VISIBLE else INVISIBLE
            if (!visible) return@forEachIndexed
            val outer = slot.clip.layoutParams as LayoutParams
            if (outer.width != clip.width() || outer.height != clip.height() ||
                outer.leftMargin != clip.left || outer.topMargin != clip.top
            ) {
                outer.width = clip.width()
                outer.height = clip.height()
                outer.leftMargin = clip.left
                outer.topMargin = clip.top
                slot.clip.layoutParams = outer
            }
            val inner = slot.view.layoutParams
            if (inner.width != rect.width() || inner.height != rect.height()) {
                inner.width = rect.width()
                inner.height = rect.height()
                slot.view.layoutParams = inner
            }
            slot.clip.scrollTo(clip.left - rect.left, clip.top - rect.top)
        }
        val order = ids.map { mounted[it]?.clip }.filterNotNull()
        if (order.indices.any { getChildAt(it) !== order[it] }) {
            order.forEach { it.bringToFront() }
        }
    }

    fun containsPoint(x: Float, y: Float): Boolean = mounted.values.any {
        val clip = it.clip
        clip.visibility == VISIBLE && x >= clip.left && x < clip.right && y >= clip.top && y < clip.bottom
    }

    fun clear() {
        mounted.keys.toList().forEach(::remove)
        failed.clear()
    }

    private fun mount(id: Long, kind: String): Mounted? {
        if (failed[id] == kind) return null
        val factory = factories[kind] ?: return null
        return try {
            val view = factory.create(context)
            check(view.parent == null) { "Platform view factories must return a detached View" }
            val clip = FrameLayout(context)
            clip.importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
            clip.addView(view, LayoutParams(0, 0, Gravity.TOP or Gravity.LEFT))
            addView(clip, LayoutParams(0, 0, Gravity.TOP or Gravity.LEFT))
            Mounted(kind, factory, view, clip).also { mounted[id] = it }
        } catch (error: RuntimeException) {
            failed[id] = kind
            Log.e("Impasto", "Could not mount platform view $kind", error)
            null
        }
    }

    private fun remove(id: Long) {
        val slot = mounted.remove(id) ?: return
        removeView(slot.clip)
        slot.clip.removeView(slot.view)
        try {
            slot.factory.dispose(slot.view)
        } catch (error: RuntimeException) {
            Log.e("Impasto", "Could not dispose platform view ${slot.kind}", error)
        }
    }

    private fun pixels(values: FloatArray, index: Int, density: Float) = Rect(
        (values[index] * density).roundToInt(),
        (values[index + 1] * density).roundToInt(),
        ((values[index] + values[index + 2]) * density).roundToInt(),
        ((values[index + 1] + values[index + 3]) * density).roundToInt(),
    )
}
