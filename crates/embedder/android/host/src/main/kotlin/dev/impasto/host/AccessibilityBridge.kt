package dev.impasto.host

import android.graphics.Rect
import android.os.Bundle
import android.view.MotionEvent
import android.view.View
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityManager
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityNodeProvider
import androidx.core.view.accessibility.AccessibilityNodeInfoCompat
import androidx.core.view.accessibility.AccessibilityNodeProviderCompat
import kotlin.math.roundToInt

/** A native semantics identity maps to one Android virtual ID while the node is exposed. */
internal class AccessibilityBridge(
    private val view: ImpastoView,
    private val source: View,
) : AccessibilityNodeProviderCompat() {
    private val manager = view.context.getSystemService(AccessibilityManager::class.java)
    private val stateListener = AccessibilityManager.AccessibilityStateChangeListener(::enabledChanged)
    private var listening = false
    private var nextId = 1
    private var focus: Int? = null
    private var hovered: Int? = null
    private var ids = emptyMap<Long, Int>()
    private var nodes = emptyMap<Int, Node>()
    private var children = emptyMap<Int, List<Int>>()

    val nodeProvider: AccessibilityNodeProvider?
        get() = if (listening && manager?.isEnabled == true) provider as AccessibilityNodeProvider else null

    fun attach() {
        if (listening || manager == null || view.handle == 0L) return
        manager.addAccessibilityStateChangeListener(stateListener)
        listening = true
        enabledChanged(manager.isEnabled)
    }

    fun detach() {
        if (!listening) return
        manager?.removeAccessibilityStateChangeListener(stateListener)
        enabledChanged(false)
        listening = false
    }

    private fun enabledChanged(enabled: Boolean) {
        view.accessibilityChanged(enabled)
        if (!enabled) {
            focus = null
            hovered = null
            ids = emptyMap()
            nodes = emptyMap()
            children = emptyMap()
        }
    }

    fun update(
        ids: LongArray, roles: IntArray, labels: Array<String>, values: Array<String>,
        flags: IntArray, actions: IntArray, selection: IntArray, scroll: FloatArray, descriptions: Array<String>,
        parents: LongArray, bounds: FloatArray, reset: Boolean,
    ) {
        if (!listening || manager?.isEnabled != true) return
        val previous = nodes
        val nextIds = if (reset) ids.associateWith { this.ids[it] ?: nextId++ } else this.ids
        val nextNodes = if (reset) LinkedHashMap(ids.size) else LinkedHashMap(previous)
        val nextChildren = LinkedHashMap<Int, MutableList<Int>>()
        for (index in ids.indices) {
            val id = nextIds[ids[index]] ?: continue
            val parent = nextIds[parents[index]] ?: View.NO_ID
            val b = index * 4
            nextNodes[id] = Node(
                ids[index], parent, roles[index], labels[index], values[index], flags[index],
                actions[index], selection[index * 2], selection[index * 2 + 1],
                descriptions[index], bounds[b], bounds[b + 1], bounds[b + 2],
                bounds[b + 3], scroll[b], scroll[b + 1], scroll[b + 2], scroll[b + 3],
            )
            if (reset) nextChildren.getOrPut(parent, ::mutableListOf).add(id)
        }
        focus?.takeUnless(nextNodes::containsKey)?.let {
            send(it, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED)
            focus = null
        }
        hovered?.takeUnless(nextNodes::containsKey)?.let {
            send(it, AccessibilityEvent.TYPE_VIEW_HOVER_EXIT)
            hovered = null
        }
        val structureChanged = reset && children != nextChildren
        this.ids = nextIds
        nodes = nextNodes
        if (reset) children = nextChildren
        if (structureChanged) contentChanged(View.NO_ID, AccessibilityEvent.CONTENT_CHANGE_TYPE_SUBTREE)
        if (nextNodes.isEmpty()) return
        val changedIds = if (reset) nextNodes.keys else ids.asSequence().mapNotNull { nextIds[it] }.toList()
        for (id in changedIds) {
            val node = nextNodes[id] ?: continue
            val before = previous[id]
            if (node.flags and FOCUSED != 0 && (before?.flags ?: 0) and FOCUSED == 0) {
                send(id, AccessibilityEvent.TYPE_VIEW_FOCUSED)
            }
            if (before == null) continue
            if (node.flags and EDITABLE != 0 && node.value != before.value) {
                send(id, AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED) {
                    beforeText = before.value
                    fromIndex = 0
                    removedCount = before.value.length
                    addedCount = node.value.length
                    text.add(node.value)
                }
            }
            if (node.flags and EDITABLE != 0 && node.selectionStart >= 0 && node.selectionEnd >= 0 &&
                (node.selectionStart != before.selectionStart || node.selectionEnd != before.selectionEnd)
            ) {
                send(id, AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED) {
                    fromIndex = node.selectionStart
                    toIndex = node.selectionEnd
                    itemCount = node.value.length
                    text.add(node.value)
                }
            }
            if (before.scrollX != node.scrollX || before.scrollY != node.scrollY) {
                send(id, AccessibilityEvent.TYPE_VIEW_SCROLLED) {
                    scrollX = pixels(node.scrollX)
                    scrollY = pixels(node.scrollY)
                    maxScrollX = pixels((node.contentWidth - node.width).coerceAtLeast(0f))
                    maxScrollY = pixels((node.contentHeight - node.height).coerceAtLeast(0f))
                }
            }
            // Announce the node the service is inspecting. Other labels are read on query.
            if (focus == id && (before.value != node.value || before.label != node.label ||
                    before.description != node.description || before.flags != node.flags)
            ) {
                contentChanged(id, AccessibilityEvent.CONTENT_CHANGE_TYPE_UNDEFINED)
            }
        }
    }

    private fun contentChanged(id: Int, type: Int) {
        send(id, AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED) { contentChangeTypes = type }
    }

    override fun createAccessibilityNodeInfo(virtualViewId: Int): AccessibilityNodeInfoCompat? {
        if (!listening || manager?.isEnabled != true) return null
        if (virtualViewId == View.NO_ID) {
            @Suppress("DEPRECATION")
            val info = AccessibilityNodeInfoCompat.obtain(source)
            source.onInitializeAccessibilityNodeInfo(info.unwrap())
            children[View.NO_ID]?.forEach { info.addChild(source, it) }
            return info
        }
        val node = nodes[virtualViewId] ?: return null
        @Suppress("DEPRECATION")
        val info = AccessibilityNodeInfoCompat.obtain(source, virtualViewId)
        info.packageName = view.context.packageName
        info.className = className(node.role)
        if (node.role == ROLE_IMAGE) {
            info.contentDescription = node.label.ifEmpty { node.description }
        } else {
            info.text = node.value.ifEmpty { node.label }
            info.contentDescription = node.description.takeIf { it.isNotEmpty() }
            if (node.value.isNotEmpty() && node.label.isNotEmpty()) info.hintText = node.label
        }
        info.isEnabled = node.flags and DISABLED == 0
        info.isFocusable = node.flags and FOCUSABLE != 0
        info.isFocused = node.flags and FOCUSED != 0
        info.isEditable = node.flags and EDITABLE != 0
        info.isPassword = node.flags and PASSWORD != 0
        if (info.isEditable && node.selectionStart >= 0 && node.selectionEnd >= 0) {
            info.setTextSelection(node.selectionStart, node.selectionEnd)
        }
        info.isCheckable = node.role == ROLE_SWITCH || node.flags and CHECKED != 0
        info.isChecked = node.flags and CHECKED != 0
        info.isClickable = info.isEnabled && node.actions and ACTIVATE != 0
        info.isScrollable = info.isEnabled && node.actions and SCROLL != 0
        if (info.isClickable) info.addAction(AccessibilityNodeInfoCompat.AccessibilityActionCompat.ACTION_CLICK)
        if (info.isScrollable) {
            info.addAction(AccessibilityNodeInfoCompat.AccessibilityActionCompat.ACTION_SCROLL_FORWARD)
            info.addAction(AccessibilityNodeInfoCompat.AccessibilityActionCompat.ACTION_SCROLL_BACKWARD)
        }
        if (info.isEnabled && node.actions and SET_TEXT != 0) {
            info.addAction(AccessibilityNodeInfoCompat.AccessibilityActionCompat.ACTION_SET_TEXT)
        }
        if (info.isFocused && node.actions and SET_SELECTION != 0) {
            info.addAction(AccessibilityNodeInfoCompat.AccessibilityActionCompat.ACTION_SET_SELECTION)
        }
        info.isAccessibilityFocused = focus == virtualViewId
        info.addAction(
            if (info.isAccessibilityFocused) AccessibilityNodeInfoCompat.AccessibilityActionCompat.ACTION_CLEAR_ACCESSIBILITY_FOCUS
            else AccessibilityNodeInfoCompat.AccessibilityActionCompat.ACTION_ACCESSIBILITY_FOCUS,
        )
        if (node.parent == View.NO_ID) info.setParent(source) else info.setParent(source, node.parent)
        children[virtualViewId]?.forEach { info.addChild(source, it) }
        val rect = bounds(node)
        val localVisible = Rect()
        val visible = view.isShown && view.getLocalVisibleRect(localVisible) && rect.intersect(localVisible)
        val origin = IntArray(2)
        view.getLocationOnScreen(origin)
        rect.offset(origin[0], origin[1])
        info.setBoundsInScreen(rect)
        info.isVisibleToUser = visible
        return info
    }

    override fun findFocus(focus: Int): AccessibilityNodeInfoCompat? = when (focus) {
        AccessibilityNodeInfo.FOCUS_ACCESSIBILITY -> this.focus?.let(::createAccessibilityNodeInfo)
        AccessibilityNodeInfo.FOCUS_INPUT -> nodes.entries.firstOrNull { (_, node) ->
            node.flags and FOCUSED != 0
        }?.key?.let(::createAccessibilityNodeInfo)
        else -> null
    }

    override fun performAction(virtualViewId: Int, action: Int, arguments: Bundle?): Boolean {
        if (virtualViewId == View.NO_ID || view.handle == 0L) return false
        val node = nodes[virtualViewId] ?: return false
        val enabled = node.flags and DISABLED == 0
        when (action) {
            AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS -> {
                if (focus == virtualViewId) return false
                focus?.let { send(it, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED) }
                focus = virtualViewId
                send(virtualViewId, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUSED)
            }
            AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS -> {
                if (focus != virtualViewId) return false
                focus = null
                send(virtualViewId, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED)
            }
            AccessibilityNodeInfo.ACTION_CLICK -> {
                if (!enabled || node.actions and ACTIVATE == 0) return false
                NativeBridge.nativeActivate(view.handle, node.nativeId)
                send(virtualViewId, AccessibilityEvent.TYPE_VIEW_CLICKED)
            }
            AccessibilityNodeInfo.ACTION_SCROLL_FORWARD, AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD -> {
                if (!enabled || node.actions and SCROLL == 0) return false
                NativeBridge.nativeScroll(view.handle, node.nativeId, action == AccessibilityNodeInfo.ACTION_SCROLL_FORWARD)
            }
            AccessibilityNodeInfo.ACTION_SET_TEXT -> {
                if (!enabled || node.actions and SET_TEXT == 0) return false
                val text = arguments?.getCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE)
                    ?: return false
                NativeBridge.nativeSetText(view.handle, node.nativeId, text.toString())
            }
            AccessibilityNodeInfo.ACTION_SET_SELECTION -> {
                if (!enabled || node.flags and FOCUSED == 0 || node.actions and SET_SELECTION == 0) return false
                val start = arguments?.getInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, -1) ?: -1
                val end = arguments?.getInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, -1) ?: -1
                if (start !in 0..node.value.length || end !in 0..node.value.length) return false
                NativeBridge.nativeSetSelection(view.handle, node.nativeId, start, end)
            }
            else -> return false
        }
        return true
    }

    fun clearHover() {
        hovered?.let { send(it, AccessibilityEvent.TYPE_VIEW_HOVER_EXIT) }
        hovered = null
    }

    fun hover(event: MotionEvent): Boolean {
        if (manager?.isTouchExplorationEnabled != true || nodes.isEmpty() ||
            !event.isFromSource(android.view.InputDevice.SOURCE_TOUCHSCREEN)
        ) return false
        val next = when (event.actionMasked) {
            MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE ->
                hit(event.x, event.y, View.NO_ID)
            MotionEvent.ACTION_HOVER_EXIT -> null
            else -> return false
        }
        if (next != hovered) {
            hovered?.let { send(it, AccessibilityEvent.TYPE_VIEW_HOVER_EXIT) }
            hovered = next
            next?.let { send(it, AccessibilityEvent.TYPE_VIEW_HOVER_ENTER) }
        }
        return true
    }

    private fun hit(x: Float, y: Float, parent: Int): Int? {
        for (id in children[parent].orEmpty().asReversed()) {
            val node = nodes.getValue(id)
            if (!bounds(node).contains(x.toInt(), y.toInt())) continue
            hit(x, y, id)?.let { return it }
            return id
        }
        return null
    }

    private fun send(id: Int, type: Int, fill: AccessibilityEvent.() -> Unit = {}) {
        if (manager?.isEnabled != true) return
        val parent = source.parent ?: return
        @Suppress("DEPRECATION")
        val event = AccessibilityEvent.obtain(type)
        event.packageName = view.context.packageName
        event.className = nodes[id]?.let { className(it.role) } ?: view.javaClass.name
        if (id == View.NO_ID) event.setSource(source) else event.setSource(source, id)
        event.fill()
        parent.requestSendAccessibilityEvent(source, event)
    }

    private fun pixels(value: Float): Int = (value * view.density).roundToInt()

    private fun bounds(node: Node) = Rect(
        pixels(node.x), pixels(node.y), pixels(node.x + node.width), pixels(node.y + node.height),
    )

    private data class Node(
        val nativeId: Long, val parent: Int, val role: Int, val label: String, val value: String,
        val flags: Int, val actions: Int, val selectionStart: Int, val selectionEnd: Int, val description: String,
        val x: Float, val y: Float, val width: Float, val height: Float,
        val scrollX: Float, val scrollY: Float, val contentWidth: Float, val contentHeight: Float,
    )

    companion object {
        private const val ROLE_IMAGE = 3
        private const val ROLE_SWITCH = 5
        private const val ACTIVATE = 1
        private const val SCROLL = 2
        private const val SET_TEXT = 4
        private const val SET_SELECTION = 8
        private const val FOCUSABLE = 1
        private const val CHECKED = 2
        private const val DISABLED = 4
        private const val EDITABLE = 8
        private const val FOCUSED = 32
        private const val PASSWORD = 64

        private fun className(role: Int) = when (role) {
            1 -> "android.widget.Button"
            2 -> "android.widget.TextView"
            ROLE_IMAGE -> "android.widget.ImageView"
            4 -> "android.widget.ScrollView"
            ROLE_SWITCH -> "android.widget.Switch"
            6, 7, 8 -> "android.widget.EditText"
            else -> "android.view.View"
        }
    }
}
