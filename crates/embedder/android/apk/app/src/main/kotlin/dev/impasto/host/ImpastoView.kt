package dev.impasto.host

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.SparseArray
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.ViewStructure
import android.view.accessibility.AccessibilityNodeProvider
import android.view.autofill.AutofillValue
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.widget.FrameLayout
import androidx.annotation.Keep
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat

/** Owns the render surface, viewport, and input endpoints for one native session. */
@Keep
class ImpastoView internal constructor(
    private val activity: Activity,
    private val startupFailure: (Int) -> Unit,
    private val firstFrameSubmitted: () -> Unit,
    private val exit: () -> Unit,
    platformViewFactories: Map<String, PlatformViewFactory>,
) : FrameLayout(activity) {
    private val handler = Handler(Looper.getMainLooper())
    private val surface = ImpastoSurfaceView(this)
    private val frames = FrameScheduler(this)
    private val viewport = ViewportObserver(activity, this)
    private val pointer = PointerInput(this)
    private val text = TextInput(this)
    private val textures = mutableMapOf<String, ExternalTexture>()
    private val platformViews = PlatformViewLayer(activity, platformViewFactories)
    private val semanticsHost = object : View(activity) {
        override fun getAccessibilityNodeProvider(): AccessibilityNodeProvider? = access.nodeProvider
    }
    private val access: AccessibilityBridge = AccessibilityBridge(this, semanticsHost)
    private val autofillBridge = if (Build.VERSION.SDK_INT >= 26) AutofillBridge(this) else null
    private var accessibilityEnabled = false
    private var firstFrameReported = false
    internal var windowFocusChanged: ((Boolean) -> Unit)? = null
    internal var windowAttachmentChanged: ((Boolean) -> Unit)? = null
    internal var backEnabledChanged: ((Boolean) -> Unit)? = null
    internal var backFallback: (() -> Unit)? = null
    internal var backEnabled = false
        private set
    internal var sessionReady = false
        private set
    internal var documentPicker: DocumentPicker? = null
    internal var permissions: Permissions? = null
    @Volatile
    internal var handle: Long = 0L
        private set
    val density: Float get() = resources.displayMetrics.density
    val insetsController get() = WindowCompat.getInsetsController(activity.window, this)

    init {
        isFocusable = true
        isFocusableInTouchMode = true
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
        semanticsHost.importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_YES
        if (Build.VERSION.SDK_INT >= 26) importantForAutofill = IMPORTANT_FOR_AUTOFILL_YES
        addView(surface, fillParent())
        addView(semanticsHost, fillParent())
        addView(platformViews, fillParent())
    }

    private fun fillParent() = LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT)

    internal fun attach(handle: Long, ready: Boolean = false) {
        check(this.handle == 0L && handle != 0L)
        this.handle = handle
        sessionReady = ready
    }

    internal fun release() {
        textures.values.toList().forEach(ExternalTexture::close)
        platformViews.clear()
        surface.release()
        frames.cancel()
        viewport.detach()
        text.release()
        if (Build.VERSION.SDK_INT >= 26) autofillBridge?.clear()
        access.detach()
        windowFocusChanged = null
        windowAttachmentChanged = null
        backEnabledChanged = null
        backFallback = null
        documentPicker?.close()
        documentPicker = null
        permissions?.close()
        permissions = null
        handle = 0L
        sessionReady = false
    }

    /** Call on the main thread. Stop the producer before closing its surface. */
    fun openExternalTexture(
        name: String,
        width: Int,
        height: Int,
        format: ExternalTexture.Format = ExternalTexture.Format.RGBA,
    ): ExternalTexture {
        check(Looper.myLooper() == Looper.getMainLooper() && handle != 0L)
        require(name.matches(Regex("[A-Za-z0-9_-]{1,64}")))
        require(width in 1..4096 && height in 1..4096 && width.toLong() * height <= 2_073_600L)
        check(name !in textures && textures.size < 4)
        return ExternalTexture(handle, name, width, height, format) {
            textures.remove(name)
        }.also { textures[name] = it }
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        windowFocusChanged?.invoke(hasWindowFocus())
        windowAttachmentChanged?.invoke(true)
        access.attach()
        viewport.attach()
        ViewCompat.requestApplyInsets(this)
    }

    override fun onDetachedFromWindow() {
        platformViews.clear()
        windowAttachmentChanged?.invoke(false)
        if (Build.VERSION.SDK_INT >= 26) autofillBridge?.clear()
        access.detach()
        frames.cancel()
        text.release()
        viewport.detach()
        super.onDetachedFromWindow()
    }

    override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
        super.onWindowFocusChanged(hasWindowFocus)
        windowFocusChanged?.invoke(hasWindowFocus)
        if (Build.VERSION.SDK_INT >= 26) autofillBridge?.windowFocusChanged(hasWindowFocus)
        if (hasWindowFocus) updateSemanticsSubscription()
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        viewport.refresh()
    }

    fun refreshViewport() {
        viewport.refresh()
        ViewCompat.requestApplyInsets(this)
    }

    internal fun accessibilityChanged(enabled: Boolean) {
        accessibilityEnabled = enabled
        updateSemanticsSubscription()
    }

    private fun updateSemanticsSubscription() {
        if (handle == 0L) return
        val autofillEnabled = Build.VERSION.SDK_INT >= 26 && autofillBridge?.enabled == true
        NativeBridge.nativeSemanticsEnabled(
            handle, isAttachedToWindow && accessibilityEnabled, isAttachedToWindow && autofillEnabled,
        )
    }

    @androidx.annotation.RequiresApi(26)
    override fun onProvideAutofillVirtualStructure(structure: ViewStructure, flags: Int) {
        super.onProvideAutofillVirtualStructure(structure, flags)
        autofillBridge?.provide(structure)
    }

    @androidx.annotation.RequiresApi(26)
    override fun autofill(values: SparseArray<AutofillValue>) {
        autofillBridge?.fill(values)
    }

    override fun onCheckIsTextEditor(): Boolean = text.active

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection? =
        text.createConnection(outAttrs) ?: super.onCreateInputConnection(outAttrs)

    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (handle == 0L) return false
        if (event.actionMasked == MotionEvent.ACTION_DOWN) {
            requestFocus()
            requestUnbufferedDispatch(event)
        }
        return pointer.touch(event) || super.onTouchEvent(event)
    }

    override fun dispatchHoverEvent(event: MotionEvent): Boolean {
        val native = platformViews.containsPoint(event.x, event.y)
        if (native) {
            access.clearHover()
            return super.dispatchHoverEvent(event)
        }
        return access.hover(event) || super.dispatchHoverEvent(event)
    }

    override fun requestChildFocus(child: View, focused: View) {
        super.requestChildFocus(child, focused)
        if (child === platformViews && text.active && handle != 0L) {
            text.release(keepKeyboard = true)
            NativeBridge.nativeDismissIme(handle)
        }
    }

    override fun onHoverEvent(event: MotionEvent): Boolean =
        (handle != 0L && pointer.hover(event)) || super.onHoverEvent(event)

    override fun onGenericMotionEvent(event: MotionEvent): Boolean =
        (handle != 0L && pointer.scroll(event)) || super.onGenericMotionEvent(event)

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        if (!isFocused || handle == 0L || event.keyCode == KeyEvent.KEYCODE_BACK) {
            return super.dispatchKeyEvent(event)
        }
        if (text.handleKey(event)) return true
        if (event.action != KeyEvent.ACTION_DOWN && event.action != KeyEvent.ACTION_UP) {
            return super.dispatchKeyEvent(event)
        }
        NativeBridge.nativeKey(
            handle, if (event.action == KeyEvent.ACTION_DOWN) 0 else 1, event.keyCode,
            PointerInput.modifiers(event.metaState), event.unicodeChar,
        )
        return true
    }

    // Called by the native Bridge. View updates are dispatched to the main thread.
    fun scheduleFrame(token: Long) = onMain { frames.schedule(token) }

    fun onSessionStarted(status: Int) = onMain {
        if (status == 0) {
            sessionReady = true
            NativeBridge.nativeSessionReady(handle)
        } else startupFailure(status)
    }

    fun onFirstFrameSubmitted(surfaceId: Long) = onMain {
        if (!firstFrameReported && NativeBridge.nativeIsCurrentSurface(handle, surfaceId)) {
            firstFrameReported = true
            firstFrameSubmitted()
        }
    }

    fun finishHost() = onMain(exit)
    fun setBackEnabled(enabled: Boolean) = onMain {
        backEnabled = enabled
        backEnabledChanged?.invoke(enabled)
    }
    fun dispatchBackFallback() = onMain { backFallback?.invoke() }

    fun openUrl(url: String): Boolean {
        if (handle == 0L || url.isBlank()) return false
        return try {
            activity.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
            true
        } catch (_: ActivityNotFoundException) {
            false
        } catch (_: SecurityException) {
            false
        }
    }

    fun shareText(text: String): Boolean {
        if (handle == 0L || text.isBlank()) return false
        val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text)
        return try {
            activity.startActivity(Intent.createChooser(send, null))
            true
        } catch (_: RuntimeException) {
            false
        }
    }

    fun permissionStatus(name: String): String = permissions?.status(name) ?: "denied"
    fun permissionRationale(name: String): Boolean = permissions?.rationale(name) ?: false
    fun permissionService(name: String): String = permissions?.service(name) ?: "notApplicable"
    fun openAppSettings(): Boolean = permissions?.openSettings() ?: false

    fun requestPermissions(id: Long, names: Array<String>): Boolean {
        val owner = handle
        if (owner == 0L) return false
        return handler.post {
            val service = permissions.takeIf { handle == owner }
            if (service == null) {
                NativeBridge.nativePermissionResult(owner, id, null, "permission request lost its window")
            } else {
                service.request(id, names)
            }
        }
    }

    fun documentDirectory(): String = context.filesDir.absolutePath

    fun readClipboard(): String =
        context.getSystemService(ClipboardManager::class.java)?.primaryClip
            ?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.text?.toString().orEmpty()

    fun writeClipboardNow(text: String) {
        context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(
            ClipData.newPlainText("text", text),
        )
    }

    fun postNative(work: Long): Boolean =
        handler.post { NativeBridge.nativeRunWork(work, handle != 0L) }

    fun requestDocument(id: Long): Boolean = handler.post {
        if (handle != 0L) {
            val picker = documentPicker
            if (picker != null) {
                picker.open(id)
            } else {
                NativeBridge.nativeDocumentPicked(handle, id, null, "host does not provide Activity Results")
            }
        }
    }

    fun placePlatformViews(surfaceId: Long, ids: LongArray, kinds: Array<String>, geometry: FloatArray) = onMain {
        if (isAttachedToWindow && NativeBridge.nativeIsCurrentSurface(handle, surfaceId)) {
            platformViews.update(ids, kinds, geometry, density)
        }
    }

    fun publishSemantics(
        ids: LongArray,
        roles: IntArray,
        labels: Array<String>,
        values: Array<String>,
        flags: IntArray,
        actions: IntArray,
        selection: IntArray,
        scroll: FloatArray,
        descriptions: Array<String>,
        parents: LongArray,
        bounds: FloatArray,
        autofillHints: Array<String>,
        autofillValues: Array<String>,
    ) = onMain {
        access.update(ids, roles, labels, values, flags, actions, selection, scroll, descriptions, parents, bounds)
        if (Build.VERSION.SDK_INT >= 26 && autofillBridge?.enabled == true && isAttachedToWindow) {
            autofillBridge.update(ids, autofillHints, autofillValues, flags, bounds)
        }
    }

    fun applyIme(
        op: Int, value: String, selStart: Int, selEnd: Int, compStart: Int, compEnd: Int,
        caretX: Float, caretY: Float, caretW: Float, caretH: Float, inputType: Int, imeAction: Int,
    ) = onMain {
        text.apply(op, value, selStart, selEnd, compStart, compEnd, caretX, caretY, caretW, caretH, inputType, imeAction)
    }

    private fun onMain(action: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            if (handle != 0L) action()
        } else {
            handler.post { if (handle != 0L) action() }
        }
    }
}
