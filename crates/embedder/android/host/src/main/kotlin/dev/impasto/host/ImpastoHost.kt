package dev.impasto.host

import android.app.Activity
import android.content.Intent
import android.content.res.Configuration
import android.database.ContentObserver
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.view.View
import androidx.activity.BackEventCompat
import androidx.activity.OnBackPressedCallback
import androidx.activity.OnBackPressedDispatcher
import androidx.core.os.ConfigurationCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.savedstate.SavedStateRegistry
import androidx.savedstate.SavedStateRegistryOwner
import dev.impasto.host.WindowLifecycle.State

/** Owns the session and its current View until close or an explicit window transfer. */
class ImpastoHost private constructor(
    val activity: Activity,
    private val owner: LifecycleOwner,
    val view: ImpastoView,
    private val savedState: SavedStateRegistry?,
    private val restorationKey: String?,
    backDispatcher: OnBackPressedDispatcher?,
    private val windowSession: ImpastoWindowSession?,
    private val retainedPluginState: MutableMap<String, Any>,
) : DefaultLifecycleObserver, AutoCloseable {
    enum class Renderer(internal val wireName: String) { AUTO("auto"), VULKAN("vulkan"), GLES("gles") }

    fun interface ActivityResultListener {
        fun onResult(requestCode: Int, resultCode: Int, data: Intent?): Boolean
    }

    fun interface RequestPermissionsResultListener {
        fun onResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray): Boolean
    }

    fun interface NewIntentListener {
        fun onNewIntent(intent: Intent): Boolean
    }

    private val activityResults = linkedSetOf<ActivityResultListener>()
    private val permissionResults = linkedSetOf<RequestPermissionsResultListener>()
    private val newIntents = linkedSetOf<NewIntentListener>()

    fun addActivityResultListener(listener: ActivityResultListener): AutoCloseable = listen(activityResults, listener)

    fun addRequestPermissionsResultListener(listener: RequestPermissionsResultListener): AutoCloseable =
        listen(permissionResults, listener)

    fun addNewIntentListener(listener: NewIntentListener): AutoCloseable = listen(newIntents, listener)

    private fun <T> listen(listeners: MutableSet<T>, listener: T): AutoCloseable {
        check(Looper.myLooper() == Looper.getMainLooper())
        check(view.handle != 0L) { "The Impasto window has detached" }
        check(listeners.add(listener)) { "Listener is already registered" }
        return AutoCloseable {
            check(Looper.myLooper() == Looper.getMainLooper())
            listeners.remove(listener)
        }
    }

    fun dispatchActivityResult(requestCode: Int, resultCode: Int, data: Intent?): Boolean =
        activityResults.toList().any { it.onResult(requestCode, resultCode, data) }

    fun dispatchRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray): Boolean =
        permissionResults.toList().any { it.onResult(requestCode, permissions, grantResults) }

    fun dispatchNewIntent(intent: Intent): Boolean = newIntents.toList().any { it.onNewIntent(intent) }

    private class NativePlugin(
        val handle: (ImpastoNativeRequest) -> Unit,
        val hasPendingState: () -> Boolean,
        val detach: (Boolean) -> Any?,
    )

    private val nativePlugins = linkedMapOf<String, NativePlugin>()
    internal val hasRetainablePluginState: Boolean
        get() = nativePlugins.values.any { it.hasPendingState() }

    /** Registers named operations for the lifetime of this window attachment. */
    fun registerNativePlugin(
        name: String,
        hasPendingState: () -> Boolean = { false },
        detach: (Boolean) -> Any? = { null },
        handle: (ImpastoNativeRequest) -> Unit,
    ) {
        check(Looper.myLooper() == Looper.getMainLooper())
        check(view.handle != 0L)
        check(name !in nativePlugins) { "Native plugin $name is already registered" }
        nativePlugins[name] = NativePlugin(handle, hasPendingState, detach)
    }

    /** Consumes state retained by this plugin during recreation of the same Activity. */
    fun takeRetainedPluginState(name: String): Any? = retainedPluginState.remove(name)

    internal fun isNativeRequestPending(id: Long): Boolean =
        view.handle != 0L && NativeBridge.nativePlatformPending(view.handle, id)

    fun completeNativeRequest(id: Long, payload: String? = null, error: String? = null): Boolean =
        view.handle != 0L && NativeBridge.nativePlatformResult(view.handle, id, payload, error)

    private fun dispatchNativeRequest(id: Long, operation: String, payload: String) {
        if (!isNativeRequestPending(id)) return
        val request = ImpastoNativeRequest(id, operation, payload, this)
        val plugin = nativePlugins[operation.substringBeforeLast('/')]
        if (plugin == null) {
            request.fail("No platform plugin handles $operation")
        } else {
            try {
                plugin.handle(request)
            } catch (error: RuntimeException) {
                request.fail("Platform request failed (${error.javaClass.simpleName})")
            }
        }
    }

    private fun detachPlugins(retain: Boolean): MutableMap<String, Any> {
        val state = linkedMapOf<String, Any>()
        val plugins = nativePlugins.toMap()
        nativePlugins.clear()
        for ((name, plugin) in plugins) {
            plugin.detach(retain)?.let { state[name] = it }
        }
        return state
    }

    private val resolver = activity.contentResolver
    private val windowLifecycle = WindowLifecycle(false, initiallyAttached = false) { state ->
        when (state) {
            State.RESUMED -> NativeBridge.nativeResume(view.handle)
            State.INACTIVE -> NativeBridge.nativeInactive(view.handle)
            State.HIDDEN -> NativeBridge.nativeHidden(view.handle)
        }
    }
    private val settingsObserver = object : ContentObserver(Handler(Looper.getMainLooper())) {
        override fun onChange(selfChange: Boolean) = publishSettings()
    }
    private var observingSettings = false
    private val backCallback = backDispatcher?.let { dispatcher ->
        WindowBack(view, dispatcher).also { dispatcher.addCallback(owner, it) }
    }

    init {
        view.nativeRequests = ::dispatchNativeRequest
        view.windowFocusChanged = ::windowFocusChanged
        view.windowAttachmentChanged = { attached ->
            windowLifecycle.attachmentChanged(attached)
            backCallback?.setAttached(attached)
        }
        view.backEnabledChanged = { enabled -> backCallback?.setAvailable(enabled) }
        view.backFallback = { backCallback?.dispatchFallback() }
        backCallback?.setAvailable(view.backEnabled)
    }

    override fun onStart(owner: LifecycleOwner) {
        windowLifecycle.activityChanged(State.INACTIVE)
        view.visibility = View.VISIBLE
        resolver.registerContentObserver(
            Settings.Global.getUriFor(Settings.Global.ANIMATOR_DURATION_SCALE), false, settingsObserver,
        )
        resolver.registerContentObserver(
            Settings.Secure.getUriFor(HIGH_CONTRAST), false, settingsObserver,
        )
        observingSettings = true
        publishSettings()
    }

    override fun onResume(owner: LifecycleOwner) = windowLifecycle.activityChanged(State.RESUMED)
    override fun onPause(owner: LifecycleOwner) {
        backCallback?.handleOnBackCancelled()
        windowLifecycle.activityChanged(State.INACTIVE)
    }

    override fun onStop(owner: LifecycleOwner) {
        windowLifecycle.activityChanged(State.HIDDEN)
        view.visibility = View.INVISIBLE
        stopObservingSettings()
    }

    override fun onDestroy(owner: LifecycleOwner) = close()

    fun back() = NativeBridge.nativeBack(view.handle)
    fun trimMemory() = NativeBridge.nativeTrimMemory(view.handle)
    private fun windowFocusChanged(focused: Boolean) {
        windowLifecycle.focusChanged(focused)
        NativeBridge.nativeFocus(view.handle, focused)
    }
    fun launchUrl(url: String?) = NativeBridge.nativeLaunchUrl(view.handle, url)

    fun configurationChanged() {
        publishSettings()
        view.refreshViewport()
    }

    override fun close() {
        detachPlugins(false)
        val handle = release()
        if (handle != 0L) NativeBridge.nativeDetach(handle)
    }

    /** Releases this window and transfers ownership of the live session to the caller. */
    fun retainSession(): ImpastoSession = retainSession(keepPluginState = false)

    /**
     * Transfers a session to an Activity using the restored Activity Result registry
     * of the same logical Activity, or to a replacement View in the same Activity.
     */
    fun retainSessionForRecreation(): ImpastoSession = retainSession(keepPluginState = true)

    private fun retainSession(keepPluginState: Boolean): ImpastoSession {
        check(view.sessionReady) { "Wait for the session to start before retaining it" }
        val pending = detachPlugins(keepPluginState)
        val handle = release()
        NativeBridge.nativeReleaseWindow(handle, pending.isNotEmpty())
        return ImpastoSession(handle, pending, windowSession).also { windowSession?.retained(it) }
    }

    private fun release(): Long {
        check(Looper.myLooper() == Looper.getMainLooper())
        val handle = view.handle
        if (handle == 0L) return 0L
        activityResults.clear()
        permissionResults.clear()
        newIntents.clear()
        owner.lifecycle.removeObserver(this)
        windowSession?.disconnected(this)
        backCallback?.handleOnBackCancelled()
        backCallback?.remove()
        if (restorationKey != null) savedState?.unregisterSavedStateProvider(restorationKey)
        stopObservingSettings()
        view.release()
        return handle
    }

    private fun stopObservingSettings() {
        if (!observingSettings) return
        observingSettings = false
        resolver.unregisterContentObserver(settingsObserver)
    }

    private fun publishSettings() {
        if (view.handle == 0L) return
        val settings = readSettings(activity)
        NativeBridge.nativeSettings(
            view.handle, settings.locale, settings.textScale, settings.dark,
            settings.reduceMotion, settings.boldText, settings.highContrast,
        )
    }

    companion object {
        private const val HIGH_CONTRAST = "high_text_contrast_enabled"

        /** Call on the main thread, then add [ImpastoHost.view] to the native hierarchy. */
        fun attach(
            activity: Activity,
            owner: LifecycleOwner,
            script: String? = null,
            initialUrl: String? = null,
            renderer: Renderer = Renderer.AUTO,
            onStartupFailure: (Int) -> Unit = {},
            onFirstFrameSubmitted: () -> Unit = {},
            onExit: () -> Unit = {},
            restorationId: String? = null,
            backDispatcher: OnBackPressedDispatcher? = null,
            session: ImpastoSession? = null,
            replayedIntent: Boolean = false,
            platformViewFactories: Map<String, PlatformViewFactory> = emptyMap(),
            windowSession: ImpastoWindowSession? = null,
        ): ImpastoHost? {
            check(Looper.myLooper() == Looper.getMainLooper())
            check(owner.lifecycle.currentState != Lifecycle.State.DESTROYED)
            require(session == null || windowSession == null)
            val group = windowSession ?: session?.windowSession
            val groupId = group?.idForAttach() ?: 0L
            val registry = (owner as? SavedStateRegistryOwner)?.savedStateRegistry
            val restorationKey = restorationId?.let { "dev.impasto.ui.$it" }
            require(restorationKey == null || registry != null) {
                "A restorationId requires a SavedStateRegistryOwner"
            }
            val restored = restorationKey?.let { registry?.consumeRestoredStateForKey(it) }
                ?.getString("snapshot")
            lateinit var host: ImpastoHost
            val view = ImpastoView(activity, { status ->
                host.close()
                onStartupFailure(status)
            }, onFirstFrameSubmitted, onExit, platformViewFactories.toMap())
            val settings = readSettings(activity)
            val handle = if (session == null) {
                NativeBridge.nativeAttach(
                    view, activity.assets, script, initialUrl, restored, replayedIntent, renderer.wireName,
                    settings.locale, settings.textScale, settings.dark, settings.reduceMotion,
                    settings.boldText, settings.highContrast, groupId,
                ).also { if (it != 0L) view.attach(it) }
            } else {
                session.attach(view)
            }
            if (handle == 0L) return null
            if (restorationKey != null && registry != null) {
                try {
                    registry.registerSavedStateProvider(restorationKey) {
                        Bundle().apply {
                            putString("snapshot", NativeBridge.nativeSavedState(handle))
                        }
                    }
                } catch (error: IllegalArgumentException) {
                    view.release()
                    NativeBridge.nativeDetach(handle)
                    throw error
                }
            }
            host = ImpastoHost(
                activity, owner, view, registry, restorationKey, backDispatcher, group,
                session?.takePluginState() ?: linkedMapOf(),
            )
            group?.connected(host)
            if (session != null && initialUrl != null) host.launchUrl(initialUrl)
            owner.lifecycle.addObserver(host)
            return host
        }

        private fun readSettings(activity: Activity): PlatformSettings {
            val config = activity.resources.configuration
            val resolver = activity.contentResolver
            return PlatformSettings(
                locale = ConfigurationCompat.getLocales(config)[0]?.toLanguageTag().orEmpty(),
                textScale = config.fontScale,
                dark = config.uiMode and Configuration.UI_MODE_NIGHT_MASK == Configuration.UI_MODE_NIGHT_YES,
                reduceMotion = Settings.Global.getFloat(
                    resolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f,
                ) == 0f,
                boldText = Build.VERSION.SDK_INT >= 31 && config.fontWeightAdjustment >= 300,
                highContrast = Settings.Secure.getInt(resolver, HIGH_CONTRAST, 0) == 1,
            )
        }
    }

    private data class PlatformSettings(
        val locale: String,
        val textScale: Float,
        val dark: Boolean,
        val reduceMotion: Boolean,
        val boldText: Boolean,
        val highContrast: Boolean,
    )
}

/** Holds a detached live session until it is attached to a new host or closed. */
class ImpastoSession internal constructor(
    private var handle: Long,
    private var pending: MutableMap<String, Any>?,
    internal val windowSession: ImpastoWindowSession?,
) : AutoCloseable {
    internal fun takePluginState(): MutableMap<String, Any>? = pending.also { pending = null }

    internal fun attach(view: ImpastoView): Long {
        check(handle != 0L) { "This retained session has already been consumed or closed" }
        val attached = handle
        view.attach(attached, ready = true)
        if (!NativeBridge.nativeAttachWindow(attached, view)) {
            view.release()
            return 0L
        }
        handle = 0L
        windowSession?.released(this)
        return attached
    }

    override fun close() {
        check(Looper.myLooper() == Looper.getMainLooper())
        if (handle != 0L) NativeBridge.nativeDetach(handle)
        handle = 0L
        pending = null
        windowSession?.released(this)
    }
}

/** An application session with separate UI, input and lifecycle for each attached window. */
class ImpastoWindowSession : AutoCloseable {
    private var id = 0L
    private val hosts = mutableSetOf<ImpastoHost>()
    private val detached = mutableSetOf<ImpastoSession>()

    init {
        check(Looper.myLooper() == Looper.getMainLooper())
        id = NativeBridge.nativeCreateWindowSession()
    }

    internal fun idForAttach(): Long {
        check(id != 0L) { "This window session is closed" }
        return id
    }

    internal fun connected(host: ImpastoHost) { hosts.add(host) }
    internal fun disconnected(host: ImpastoHost) { hosts.remove(host) }
    internal fun retained(session: ImpastoSession) { detached.add(session) }
    internal fun released(session: ImpastoSession) { detached.remove(session) }

    override fun close() {
        check(Looper.myLooper() == Looper.getMainLooper())
        if (id == 0L) return
        for (host in hosts.toList()) host.close()
        for (session in detached.toList()) session.close()
        NativeBridge.nativeCloseWindowSession(id)
        id = 0L
    }
}

private class WindowBack(
    private val view: ImpastoView,
    private val dispatcher: OnBackPressedDispatcher,
) : OnBackPressedCallback(false) {
    private var available = false
    private var attached = false
    private var tracking = false

    fun setAvailable(enabled: Boolean) {
        available = enabled
        if (!tracking) isEnabled = attached && available
    }

    fun setAttached(attached: Boolean) {
        this.attached = attached
        if (!attached) handleOnBackCancelled()
        isEnabled = attached && available
    }

    fun dispatchFallback() {
        if (!attached) return
        isEnabled = false
        try {
            dispatcher.onBackPressed()
        } finally {
            isEnabled = attached && available
        }
    }

    override fun handleOnBackStarted(backEvent: BackEventCompat) {
        if (ViewCompat.getRootWindowInsets(view)?.isVisible(WindowInsetsCompat.Type.ime()) == true) return
        tracking = true
        gesture(0, if (backEvent.swipeEdge == BackEventCompat.EDGE_RIGHT) 1f else 0f)
    }

    override fun handleOnBackProgressed(backEvent: BackEventCompat) {
        if (tracking) gesture(1, backEvent.progress)
    }

    override fun handleOnBackCancelled() {
        if (!tracking) return
        tracking = false
        gesture(2)
        isEnabled = attached && available
    }

    override fun handleOnBackPressed() {
        if (tracking) {
            tracking = false
            gesture(3)
            isEnabled = attached && available
        } else {
            NativeBridge.nativeBack(view.handle)
        }
    }

    private fun gesture(phase: Int, value: Float = 0f) {
        NativeBridge.nativeBackGesture(view.handle, phase, value)
    }
}

/** Window focus refines the state of a resumed Activity, including in multi-window mode. */
internal class WindowLifecycle(
    private var focused: Boolean,
    initiallyAttached: Boolean = true,
    private val publish: (State) -> Unit,
) {
    enum class State { RESUMED, INACTIVE, HIDDEN }

    private var activityState: State? = null
    private var publishedState: State? = null
    private var attached = initiallyAttached

    fun activityChanged(state: State) {
        activityState = state
        update()
    }

    fun focusChanged(focused: Boolean) {
        this.focused = focused
        update()
    }

    fun attachmentChanged(attached: Boolean) {
        this.attached = attached
        update()
    }

    private fun update() {
        val activity = activityState ?: return
        val state = when {
            !attached -> State.HIDDEN
            activity == State.RESUMED && !focused -> State.INACTIVE
            else -> activity
        }
        if (state == publishedState) return
        publishedState = state
        publish(state)
    }
}
