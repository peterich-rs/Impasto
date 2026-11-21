package dev.impasto.host

import android.view.KeyEvent
import android.view.MotionEvent
import android.view.ScaleGestureDetector

internal class PointerInput(private val view: ImpastoView) {
    private val scale = ScaleGestureDetector(
        view.context,
        object : ScaleGestureDetector.SimpleOnScaleGestureListener() {
            override fun onScale(detector: ScaleGestureDetector): Boolean {
                NativeBridge.nativeZoom(
                    view.handle, detector.scaleFactor, detector.focusX, detector.focusY,
                    detector.eventTime, view.density,
                )
                return true
            }
        },
    )

    fun touch(event: MotionEvent): Boolean {
        scale.onTouchEvent(event)
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN -> emit(event, event.actionIndex, DOWN)
            MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> {
                val phase = if (event.flags and MotionEvent.FLAG_CANCELED != 0) CANCEL else UP
                emit(event, event.actionIndex, phase)
            }
            MotionEvent.ACTION_MOVE -> (0 until event.pointerCount).forEach { emit(event, it, MOVE) }
            MotionEvent.ACTION_CANCEL -> (0 until event.pointerCount).forEach { emit(event, it, CANCEL) }
            else -> return false
        }
        return true
    }

    fun hover(event: MotionEvent): Boolean = when (event.actionMasked) {
        MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE -> {
            emit(event, 0, HOVER)
            true
        }
        MotionEvent.ACTION_HOVER_EXIT -> {
            emit(event, 0, LEAVE)
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
        val pointer = (event.deviceId.toLong() shl 32) or event.getPointerId(index).toLong()
        NativeBridge.nativePointer(
            view.handle, phase, pointer, event.getX(index), event.getY(index), event.eventTime,
            kind, button, buttons, modifiers(event.metaState),
            event.getAxisValue(MotionEvent.AXIS_HSCROLL, index),
            event.getAxisValue(MotionEvent.AXIS_VSCROLL, index), view.density,
        )
    }

    companion object {
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
