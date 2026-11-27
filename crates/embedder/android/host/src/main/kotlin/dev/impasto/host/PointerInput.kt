package dev.impasto.host

import android.view.KeyEvent
import android.view.MotionEvent
import android.view.ScaleGestureDetector

internal class PointerInput(private val view: ImpastoView) {
    private val history = DoubleArray(HISTORY_LIMIT * 3)
    private val forwarded = mutableSetOf<Int>()
    private var suppressPointers = false
    private var magnification = 1f

    private val scale = ScaleGestureDetector(
        view.context,
        object : ScaleGestureDetector.SimpleOnScaleGestureListener() {
            override fun onScale(detector: ScaleGestureDetector): Boolean {
                magnification = detector.scaleFactor
                return true
            }
        },
    ).apply {
        isQuickScaleEnabled = false
        isStylusScaleEnabled = false
    }

    fun touch(event: MotionEvent): Boolean {
        val action = event.actionMasked
        if (action == MotionEvent.ACTION_DOWN) {
            forwarded.clear()
            suppressPointers = false
        }
        magnification = 1f
        scale.onTouchEvent(event)
        if (scale.isInProgress && !suppressPointers) {
            suppressPointers = true
            cancelForwarded(event)
        }
        if (magnification != 1f) {
            NativeBridge.nativeZoom(
                view.handle, magnification, scale.focusX, scale.focusY,
                scale.eventTime, view.density,
            )
        }
        // Once pinch wins, the contacts belong to that gesture through the last lift.
        if (suppressPointers) return true
        when (action) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN -> {
                forwarded.add(event.getPointerId(event.actionIndex))
                emit(event, event.actionIndex, DOWN)
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> {
                val phase = if (event.flags and MotionEvent.FLAG_CANCELED != 0) CANCEL else UP
                if (forwarded.remove(event.getPointerId(event.actionIndex))) {
                    emit(event, event.actionIndex, phase)
                }
            }
            MotionEvent.ACTION_MOVE -> {
                for (index in 0 until event.pointerCount) {
                    if (event.getPointerId(index) in forwarded) emit(event, index, MOVE)
                }
            }
            MotionEvent.ACTION_CANCEL -> cancelForwarded(event)
            else -> return false
        }
        return true
    }

    private fun cancelForwarded(event: MotionEvent) {
        for (index in 0 until event.pointerCount) {
            if (event.getPointerId(index) in forwarded) emit(event, index, CANCEL)
        }
        forwarded.clear()
    }

    fun hover(event: MotionEvent): Boolean = when (event.actionMasked) {
        MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE -> {
            emit(event, 0, HOVER)
            true
        }
        MotionEvent.ACTION_HOVER_EXIT -> {
            if (event.buttonState == 0) emit(event, 0, LEAVE)
            true
        }
        else -> false
    }

    fun scroll(event: MotionEvent): Boolean {
        if (event.actionMasked != MotionEvent.ACTION_SCROLL) return false
        emit(event, 0, WHEEL)
        return true
    }

    private fun emit(event: MotionEvent, index: Int, phase: Int) {
        if (index !in 0 until event.pointerCount) return
        val kind = when (event.getToolType(index)) {
            MotionEvent.TOOL_TYPE_MOUSE -> 1
            MotionEvent.TOOL_TYPE_STYLUS, MotionEvent.TOOL_TYPE_ERASER -> 2
            else -> 0
        }
        val state = event.buttonState
        val pressed = phase == DOWN || phase == MOVE
        val button = when {
            phase == HOVER || phase == LEAVE || phase == CANCEL -> 0
            kind == 0 -> 1
            event.actionButton == MotionEvent.BUTTON_SECONDARY || state and MotionEvent.BUTTON_SECONDARY != 0 -> 2
            event.actionButton == MotionEvent.BUTTON_TERTIARY || state and MotionEvent.BUTTON_TERTIARY != 0 -> 3
            state and MotionEvent.BUTTON_PRIMARY != 0 || phase == DOWN || phase == UP -> 1
            else -> 0
        }
        val primary = state and MotionEvent.BUTTON_PRIMARY != 0 || (kind == 0 && pressed)
        val buttons = (if (primary) 1 else 0) or
            (if (state and MotionEvent.BUTTON_SECONDARY != 0) 2 else 0) or
            (if (state and MotionEvent.BUTTON_TERTIARY != 0) 4 else 0)
        val packed = (event.deviceId.toLong() shl 32) or
            (event.getPointerId(index).toLong() and 0xFFFF_FFFFL)
        val pointer = if (packed == 0L) Long.MAX_VALUE else packed
        val historyCount = if (phase == MOVE) event.historySize.coerceAtMost(HISTORY_LIMIT) else 0
        for (offset in 0 until historyCount) {
            val position = event.historySize - historyCount + offset
            history[offset * 3] = event.getHistoricalX(index, position).toDouble()
            history[offset * 3 + 1] = event.getHistoricalY(index, position).toDouble()
            history[offset * 3 + 2] = event.getHistoricalEventTime(position).toDouble()
        }
        NativeBridge.nativePointer(
            view.handle, phase, pointer, event.getX(index), event.getY(index), event.eventTime,
            kind, button, buttons, modifiers(event.metaState),
            event.getAxisValue(MotionEvent.AXIS_HSCROLL, index),
            event.getAxisValue(MotionEvent.AXIS_VSCROLL, index), view.density,
            if (historyCount > 0) history else null, historyCount,
        )
    }

    companion object {
        // Must fit in the native PointerHistory.
        private const val HISTORY_LIMIT = 12
        private const val DOWN = 0
        private const val MOVE = 1
        private const val UP = 2
        private const val CANCEL = 3
        private const val HOVER = 4
        private const val WHEEL = 5
        private const val LEAVE = 6

        fun modifiers(raw: Int): Int {
            val meta = KeyEvent.normalizeMetaState(raw)
            return (if (meta and KeyEvent.META_SHIFT_ON != 0) 1 else 0) or
                (if (meta and KeyEvent.META_CTRL_ON != 0) 2 else 0) or
                (if (meta and KeyEvent.META_ALT_ON != 0) 4 else 0) or
                (if (meta and KeyEvent.META_META_ON != 0) 8 else 0)
        }
    }
}
