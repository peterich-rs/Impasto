package dev.impasto.host

import android.content.ComponentCallbacks2
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.res.Configuration
import android.graphics.Color
import android.os.Bundle
import android.os.FileObserver
import android.util.Log
import java.io.File
import java.io.IOException
import androidx.fragment.app.FragmentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge

/** Entry point for an Impasto window and its engine session. */
open class ImpastoActivity : FragmentActivity() {
    private var host: ImpastoHost? = null
    protected open val platformViewFactories: Map<String, PlatformViewFactory> = emptyMap()
    protected open fun registerPlugins(host: ImpastoHost) {}

    private var deliveredUrl: String? = null
    private var updateObserver: FileObserver? = null
    private var developmentUpdates: DevelopmentUpdates? = null
    @Volatile private var updateHandle: Long? = null
    private var developmentEnabled = false

    private fun isDebuggable() = applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // The drawable leaves the system-bar areas clear over the dark window backdrop.
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
        )
        val requestedRenderer = if (isDebuggable()) {
            intent?.getStringExtra("impasto.renderer")
        } else {
            null
        }
        val renderer = ImpastoHost.Renderer.entries.firstOrNull { it.wireName == requestedRenderer }
        val launchUrl = intent?.dataString
        deliveredUrl = launchUrl
        @Suppress("DEPRECATION")
        val retained = lastCustomNonConfigurationInstance as? ImpastoSession
        developmentEnabled = if (isDebuggable()) {
            val session = runCatching { File(filesDir, "impasto-debug/session").readLines() }.getOrNull()
            val token = session?.getOrNull(0)
            token != null && TOKEN.matches(token) &&
                NativeBridge.nativePrepareDevelopment(token, cacheDir.path, session?.getOrNull(1) == "1")
        } else {
            false
        }
        val attached = ImpastoHost.attach(
            this, this,
            script = intent?.getStringExtra("script"),
            initialUrl = launchUrl.takeUnless {
                retained != null && it == savedInstanceState?.getString(DELIVERED_URL)
            },
            replayedIntent = savedInstanceState != null && launchUrl == savedInstanceState.getString(DELIVERED_URL),
            renderer = renderer ?: ImpastoHost.Renderer.AUTO,
            onStartupFailure = { finish() },
            onFirstFrameSubmitted = {
                reportFullyDrawn()
                startDevelopment()
            },
            onExit = ::finish,
            restorationId = "main",
            backDispatcher = onBackPressedDispatcher,
            session = retained,
            platformViewFactories = platformViewFactories,
        )
        if (attached == null) {
            retained?.close()
            finish()
            return
        }
        host = attached
        registerPlugins(attached)
        setContentView(attached.view)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        deliveredUrl = intent.dataString
        host?.let { if (!it.dispatchNewIntent(intent)) it.launchUrl(intent.dataString) }
    }

    @Deprecated("Use the Activity Result API for new plugins")
    @Suppress("DEPRECATION")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        if (host?.dispatchActivityResult(requestCode, resultCode, data) != true) {
            super.onActivityResult(requestCode, resultCode, data)
        }
    }

    @Deprecated("Use the Activity Result API for new plugins")
    @Suppress("DEPRECATION")
    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<String>, grantResults: IntArray) {
        if (host?.dispatchRequestPermissionsResult(requestCode, permissions, grantResults) != true) {
            super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        }
    }

    @Suppress("DEPRECATION") // The path constructor also supports devices below API 29.
    private fun startDevelopment() {
        if (!developmentEnabled || updateHandle != null) return
        val handle = host?.view?.handle ?: return
        updateHandle = handle
        val port = NativeBridge.nativeStartDevelopment(handle, packageName)
        if (port != 0) {
            developmentUpdates = try {
                DevelopmentUpdates(packageName, port)
            } catch (error: IOException) {
                Log.w("Impasto", "Could not open the ADB development socket", error)
                null
            }
        }
        val directory = File(filesDir, "impasto-debug")
        fun submit(path: String) {
            if (!path.startsWith("request-")) return
            val revision = path.removePrefix("request-").toLongOrNull() ?: return
            updateHandle?.let { NativeBridge.nativeDevelopmentFile(it, revision) }
        }
        updateObserver = object : FileObserver(directory.path, FileObserver.MOVED_TO) {
            override fun onEvent(event: Int, path: String?) {
                if (path != null) submit(path)
            }
        }.also { it.startWatching() }
        directory.list()?.forEach(::submit)
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putString(DELIVERED_URL, deliveredUrl)
        super.onSaveInstanceState(outState)
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        host?.configurationChanged()
    }

    @Suppress("DEPRECATION")
    override fun onTrimMemory(level: Int) {
        super.onTrimMemory(level)
        if (level >= ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW &&
            level != ComponentCallbacks2.TRIM_MEMORY_UI_HIDDEN
        ) {
            host?.trimMemory()
        }
    }

    @Deprecated("Android dispatches memory pressure through onTrimMemory")
    @Suppress("DEPRECATION")
    override fun onLowMemory() {
        super.onLowMemory()
        host?.trimMemory()
    }

    @Deprecated("Activity retains the session while a plugin owns a pending result")
    override fun onRetainCustomNonConfigurationInstance(): Any? =
        host?.takeIf { it.view.sessionReady && it.hasRetainablePluginState }
            ?.retainSessionForRecreation()

    override fun onDestroy() {
        val handle = updateHandle
        updateHandle = null
        developmentUpdates?.close()
        if (handle != null) NativeBridge.nativeStopDevelopment(handle)
        developmentUpdates = null
        updateObserver?.stopWatching()
        updateObserver = null
        host?.close()
        host = null
        super.onDestroy()
    }

    private companion object {
        val TOKEN = Regex("[a-f0-9]{64}")
        const val DELIVERED_URL = "dev.impasto.delivered_url"
    }
}
