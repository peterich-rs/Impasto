package dev.impasto.host

import android.app.Activity
import android.graphics.Rect
import android.hardware.display.DisplayManager
import android.os.Handler
import android.os.Looper
import android.view.ViewTreeObserver
import androidx.core.content.ContextCompat
import androidx.core.graphics.Insets
import androidx.core.util.Consumer
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsAnimationCompat
import androidx.core.view.WindowInsetsCompat
import androidx.window.java.layout.WindowInfoTrackerCallbackAdapter
import androidx.window.layout.FoldingFeature
import androidx.window.layout.WindowInfoTracker
import androidx.window.layout.WindowLayoutInfo

/** Sends one viewport in physical pixels. Rust converts it to scene points. */
internal class ViewportObserver(private val activity: Activity, private val view: ImpastoView) {
    private var insets: WindowInsetsCompat? = null
    private var lastSent: Viewport? = null
    private val imeAnimations = mutableSetOf<WindowInsetsAnimationCompat>()
    private val displayManager = activity.getSystemService(DisplayManager::class.java)
    private val tracker = WindowInfoTrackerCallbackAdapter(WindowInfoTracker.getOrCreate(activity))
    private var folds = emptyList<FoldingFeature>()
    private var lastDisplay: Display? = null
    private var tracking = false
    private val globalLayout = ViewTreeObserver.OnGlobalLayoutListener { publishDisplay() }
    private val listener = Consumer<WindowLayoutInfo> { layout ->
        if (tracking) {
            folds = layout.displayFeatures.filterIsInstance<FoldingFeature>()
            publishDisplay()
        }
    }
    private val displayListener = object : DisplayManager.DisplayListener {
        override fun onDisplayAdded(id: Int) = publishDisplay()
        override fun onDisplayRemoved(id: Int) = publishDisplay()
        override fun onDisplayChanged(id: Int) {
            if (id == view.display?.displayId) publishDisplay()
        }
    }

    init {
        ViewCompat.setOnApplyWindowInsetsListener(view) { _, value ->
            insets = value
            if (imeAnimations.isEmpty()) publish(value)
            publishDisplay()
            value
        }
        ViewCompat.setWindowInsetsAnimationCallback(
            view,
            object : WindowInsetsAnimationCompat.Callback(DISPATCH_MODE_CONTINUE_ON_SUBTREE) {
                override fun onPrepare(animation: WindowInsetsAnimationCompat) {
                    if (animation.typeMask and WindowInsetsCompat.Type.ime() != 0) {
                        imeAnimations.add(animation)
                    }
                }

                override fun onProgress(
                    insets: WindowInsetsCompat,
                    runningAnimations: MutableList<WindowInsetsAnimationCompat>,
                ): WindowInsetsCompat {
                    if (imeAnimations.isNotEmpty()) publish(insets)
                    return insets
                }

                override fun onEnd(animation: WindowInsetsAnimationCompat) {
                    if (imeAnimations.remove(animation) && imeAnimations.isEmpty()) refresh()
                }
            },
        )
    }

    fun refresh() {
        if (imeAnimations.isEmpty()) (insets ?: ViewCompat.getRootWindowInsets(view))?.let(::publish)
        publishDisplay()
    }

    fun attach() {
        if (tracking) return
        tracking = true
        tracker.addWindowLayoutInfoListener(activity, ContextCompat.getMainExecutor(activity), listener)
        displayManager?.registerDisplayListener(displayListener, Handler(Looper.getMainLooper()))
        view.viewTreeObserver.addOnGlobalLayoutListener(globalLayout)
        refresh()
    }

    fun detach() {
        if (tracking) {
            tracking = false
            view.viewTreeObserver.removeOnGlobalLayoutListener(globalLayout)
            displayManager?.unregisterDisplayListener(displayListener)
            tracker.removeWindowLayoutInfoListener(listener)
        }
        folds = emptyList()
        lastDisplay = null
        insets = null
        lastSent = null
        imeAnimations.clear()
    }

    private fun publishDisplay() {
        if (!tracking || view.handle == 0L || view.width <= 0 || view.height <= 0) return
        val screen = view.display ?: return
        val location = IntArray(2)
        view.getLocationInWindow(location)
        val features = mutableListOf<Feature>()
        fun add(bounds: Rect, kind: Int, posture: Int, separating: Boolean, offset: IntArray) {
            val x = bounds.left - offset[0]
            val y = bounds.top - offset[1]
            val right = bounds.right - offset[0]
            val bottom = bounds.bottom - offset[1]
            if (x >= view.width || y >= view.height || right <= 0 || bottom <= 0) return
            val left = x.coerceAtLeast(0)
            val top = y.coerceAtLeast(0)
            features += Feature(
                left, top, right.coerceAtMost(view.width) - left,
                bottom.coerceAtMost(view.height) - top, kind, posture, separating,
            )
        }
        val decor = activity.window.decorView
        if (folds.isNotEmpty() && decor.display?.displayId == screen.displayId) {
            val activityOffset = IntArray(2)
            val origin = IntArray(2)
            view.getLocationOnScreen(activityOffset)
            decor.getLocationOnScreen(origin)
            activityOffset[0] -= origin[0]
            activityOffset[1] -= origin[1]
            for (feature in folds) {
                val kind = if (feature.occlusionType == FoldingFeature.OcclusionType.FULL) 2 else 1
                val posture = when (feature.state) {
                    FoldingFeature.State.FLAT -> 1
                    FoldingFeature.State.HALF_OPENED -> 2
                    else -> 0
                }
                add(feature.bounds, kind, posture, feature.isSeparating, activityOffset)
            }
        }
        for (cutout in insets?.displayCutout?.boundingRects.orEmpty()) add(cutout, 3, 0, false, location)
        val display = Display(screen.displayId, screen.refreshRate, view.density, features)
        if (display == lastDisplay) return
        lastDisplay = display
        val data = features.flatMap { feature ->
            listOf(
                feature.x, feature.y, feature.width, feature.height, feature.kind, feature.posture,
                if (feature.separating) 1 else 0,
            )
        }.toIntArray()
        NativeBridge.nativeWindowDisplay(view.handle, display.id, display.refreshRate, display.density, data)
    }

    private data class Feature(
        val x: Int,
        val y: Int,
        val width: Int,
        val height: Int,
        val kind: Int,
        val posture: Int,
        val separating: Boolean,
    )

    private data class Display(
        val id: Int,
        val refreshRate: Float,
        val density: Float,
        val features: List<Feature>,
    )

    private fun publish(insets: WindowInsetsCompat) {
        if (view.handle == 0L || view.width <= 0 || view.height <= 0) return
        val viewport = Viewport(
            view.width, view.height, view.resources.displayMetrics.density,
            insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()),
            insets.getInsets(WindowInsetsCompat.Type.ime()),
            insets.getInsets(WindowInsetsCompat.Type.systemGestures()),
        )
        if (viewport == lastSent) return
        lastSent = viewport
        val (width, height, density, safe, ime, gestures) = viewport
        NativeBridge.nativeMetrics(
            view.handle, width, height, density, safe.left, safe.top, safe.right, safe.bottom,
            ime.left, ime.top, ime.right, ime.bottom,
            gestures.left, gestures.top, gestures.right, gestures.bottom,
        )
    }

    private data class Viewport(
        val width: Int,
        val height: Int,
        val density: Float,
        val safe: Insets,
        val ime: Insets,
        val gestures: Insets,
    )
}
