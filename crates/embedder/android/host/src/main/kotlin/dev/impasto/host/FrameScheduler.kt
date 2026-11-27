package dev.impasto.host

import android.os.Build
import android.view.Choreographer
import androidx.annotation.RequiresApi

/** Coalesces queued requests into a single Choreographer callback. */
internal class FrameScheduler(private val view: ImpastoView) {
    private val choreographer = Choreographer.getInstance()
    private val frameCallback = Choreographer.FrameCallback { frame ->
        val interval = frameInterval()
        deliver(frame, frame + interval, interval)
    }
    private val timeline = if (Build.VERSION.SDK_INT >= 33) Api33(choreographer) else null
    private var pending: Long? = null
    private var latestOrder = 0L
    private var posted = false
    private var primedByTouch = false

    fun update(order: Long, token: Long): Boolean {
        if (java.lang.Long.compareUnsigned(order, latestOrder) <= 0) return false
        latestOrder = order
        if (token == 0L) {
            cancel()
        } else {
            pending = token
            postCallback()
        }
        return true
    }

    fun primeTouch() {
        primedByTouch = true
        postCallback()
    }

    private fun postCallback() {
        if (posted) return
        posted = true
        if (Build.VERSION.SDK_INT >= 33 && timeline != null) {
            timeline.post { frame, target -> deliver(frame, target, frameInterval()) }
        } else {
            choreographer.postFrameCallback(frameCallback)
        }
    }

    fun cancel() {
        primedByTouch = false
        pending = null
        if (!posted) return
        posted = false
        if (Build.VERSION.SDK_INT >= 33 && timeline != null) {
            timeline.cancel()
        } else {
            choreographer.removeFrameCallback(frameCallback)
        }
    }

    private fun deliver(frame: Long, target: Long, interval: Long) {
        posted = false
        // Rust can have armed a newer token while its Handler message is still queued.
        val token = if (primedByTouch) 0L else pending
        primedByTouch = false
        pending = null
        if (token == null) return
        if (view.handle != 0L) NativeBridge.nativeOnVsync(view.handle, token, frame, target, interval)
    }

    private fun frameInterval(): Long {
        val rate = view.display?.refreshRate ?: 60f
        return (1_000_000_000.0 / if (rate.isFinite() && rate > 1f) rate else 60f).toLong()
    }

    @RequiresApi(33)
    private class Api33(private val choreographer: Choreographer) {
        private var posted: Choreographer.VsyncCallback? = null

        fun post(deliver: (Long, Long) -> Unit) {
            val callback = Choreographer.VsyncCallback { data ->
                posted = null
                deliver(data.frameTimeNanos, data.preferredFrameTimeline.expectedPresentationTimeNanos)
            }
            posted = callback
            choreographer.postVsyncCallback(callback)
        }

        fun cancel() {
            posted?.let(choreographer::removeVsyncCallback)
            posted = null
        }
    }
}
