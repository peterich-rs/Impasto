package dev.impasto.host

import android.graphics.PixelFormat
import android.graphics.Region
import android.view.SurfaceHolder
import android.view.SurfaceView

/** SurfaceHolder callbacks only publish native window changes. The render thread owns Vulkan. */
internal class ImpastoSurfaceView(private val host: ImpastoView) :
    SurfaceView(host.context), SurfaceHolder.Callback2 {

    private val location = IntArray(2)
    private var connected = false

    init {
        isFocusable = false
        isClickable = false
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
        holder.setFormat(PixelFormat.OPAQUE)
        holder.addCallback(this)
    }

    override fun surfaceCreated(holder: SurfaceHolder) = connect(holder)

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        connect(holder)
        if (connected) NativeBridge.nativeSurfaceChanged(host.handle, width, height)
    }

    override fun surfaceRedrawNeeded(holder: SurfaceHolder) {
        if (connected) NativeBridge.nativeInvalidate(host.handle)
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        if (!connected) return
        connected = false
        NativeBridge.nativeSurfaceDestroyed(host.handle)
    }

    fun release() {
        holder.removeCallback(this)
        surfaceDestroyed(holder)
    }

    private fun connect(holder: SurfaceHolder) {
        if (connected || host.handle == 0L || !holder.surface.isValid) return
        connected = NativeBridge.nativeSurfaceCreated(host.handle, holder.surface)
    }

    override fun gatherTransparentRegion(region: Region?): Boolean {
        if (alpha < 1f) return false
        getLocationInWindow(location)
        region?.op(
            location[0], location[1], location[0] + width, location[1] + height, Region.Op.DIFFERENCE,
        )
        return true
    }
}
