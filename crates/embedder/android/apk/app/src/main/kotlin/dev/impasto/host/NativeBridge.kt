package dev.impasto.host

import android.content.res.AssetManager
import android.view.Surface
import androidx.annotation.Keep

/** JNI entry points. Handles belong to the Activity that opened them. */
@Keep
internal object NativeBridge {
    init {
        System.loadLibrary("impasto")
    }

    external fun nativeAttach(
        view: ImpastoView,
        assets: AssetManager,
        script: String?,
        launchUrl: String?,
        savedState: String?,
        replayedIntent: Boolean,
        renderer: String?,
        locale: String,
        textScale: Float,
        dark: Boolean,
        reduceMotion: Boolean,
        boldText: Boolean,
        highContrast: Boolean,
        windowSession: Long,
    ): Long

    external fun nativeCreateWindowSession(): Long
    external fun nativeCloseWindowSession(id: Long)
    external fun nativeDetach(handle: Long)
    external fun nativeReleaseWindow(handle: Long, keepPermissionResult: Boolean)
    external fun nativeAttachWindow(handle: Long, view: ImpastoView): Boolean
    external fun nativeSavedState(handle: Long): String?
    external fun nativeUpdateSource(handle: Long, revision: String, source: ByteArray)
    external fun nativeHardwareTexture(handle: Long, name: String, width: Int, height: Int, lease: Any): Boolean
    external fun nativeExternalTexture(handle: Long, name: String, width: Int, height: Int, pixels: ByteArray?)

    external fun nativeSessionReady(handle: Long)

    external fun nativeDocumentPicked(handle: Long, id: Long, path: String?, error: String?)
    external fun nativePermissionResult(handle: Long, id: Long, statuses: Array<String>?, error: String?)

    external fun nativeIsCurrentSurface(handle: Long, surface: Long): Boolean

    external fun nativeResume(handle: Long)

    external fun nativeInactive(handle: Long)

    external fun nativeHidden(handle: Long)

    external fun nativeTrimMemory(handle: Long)

    external fun nativeFocus(handle: Long, focused: Boolean)
    external fun nativeDismissIme(handle: Long)

    external fun nativeBack(handle: Long)
    external fun nativeBackGesture(handle: Long, phase: Int, value: Float)

    external fun nativeSettings(
        handle: Long,
        locale: String,
        textScale: Float,
        dark: Boolean,
        reduceMotion: Boolean,
        boldText: Boolean,
        highContrast: Boolean,
    )

    external fun nativeLaunchUrl(handle: Long, url: String?)

    external fun nativeInvalidate(handle: Long)

    external fun nativeSurfaceCreated(handle: Long, surface: Surface): Boolean

    external fun nativeSurfaceChanged(handle: Long, width: Int, height: Int)

    external fun nativeSurfaceDestroyed(handle: Long)

    external fun nativeMetrics(
        handle: Long,
        width: Int,
        height: Int,
        density: Float,
        padLeft: Int,
        padTop: Int,
        padRight: Int,
        padBottom: Int,
        imeLeft: Int,
        imeTop: Int,
        imeRight: Int,
        imeBottom: Int,
        gestureLeft: Int,
        gestureTop: Int,
        gestureRight: Int,
        gestureBottom: Int,
    )
    external fun nativeWindowDisplay(
        handle: Long, displayId: Int, refreshRate: Float, density: Float, features: IntArray,
    )

    external fun nativePointer(
        handle: Long,
        phase: Int,
        pointerId: Long,
        x: Float,
        y: Float,
        timeMs: Long,
        kind: Int,
        button: Int,
        buttons: Int,
        modifiers: Int,
        wheelX: Float,
        wheelY: Float,
        density: Float,
    )

    external fun nativeZoom(handle: Long, scale: Float, x: Float, y: Float, timeMs: Long, density: Float)

    external fun nativeKey(handle: Long, action: Int, keyCode: Int, meta: Int, unicode: Int)

    external fun nativeSync(
        handle: Long,
        text: String,
        selStart: Int,
        selEnd: Int,
        compStart: Int,
        compEnd: Int,
    )

    external fun nativeAction(handle: Long)

    external fun nativeContext(handle: Long, kind: Int)

    external fun nativeAccept(
        handle: Long,
        op: Int,
        text: String,
        selStart: Int,
        selEnd: Int,
        compStart: Int,
        compEnd: Int,
    ): Boolean


    external fun nativeOnVsync(
        handle: Long,
        token: Long,
        frameTimeNanos: Long,
        targetNanos: Long,
        intervalNanos: Long,
    )

    external fun nativeSemanticsEnabled(handle: Long, enabled: Boolean, autofill: Boolean)

    external fun nativeActivate(handle: Long, id: Long)

    external fun nativeScroll(handle: Long, id: Long, forward: Boolean)

    external fun nativeAutofill(handle: Long, id: Long, text: String)
    external fun nativeSetText(handle: Long, id: Long, text: String)

    external fun nativeSetSelection(handle: Long, id: Long, start: Int, end: Int)
}
