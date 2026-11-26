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
import java.util.concurrent.Executors
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge

/** Entry point for an Impasto window and its engine session. */
open class ImpastoActivity : ComponentActivity() {
    private var host: ImpastoHost? = null
    protected open val platformViewFactories: Map<String, PlatformViewFactory> = emptyMap()
    protected open fun registerPlugins(host: ImpastoHost) {}

    private var deliveredUrl: String? = null
    private var submitted = false
    private var update: String? = null
    private var updateObserver: FileObserver? = null
    private var developmentUpdates: DevelopmentUpdates? = null
    @Volatile private var updateHandle: Long? = null

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
                submitted = true
                updateHandle = host?.view?.handle
                reportFullyDrawn()
                submitUpdate()
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
        watchUpdates()
        receiveUpdate(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        if (receiveUpdate(intent)) return
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

    private fun receiveUpdate(intent: Intent?): Boolean {
        if (!isDebuggable()) return false
        val revision = intent?.getStringExtra(DEBUG_UPDATE) ?: return false
        return receiveUpdate(revision)
    }

    private fun receiveUpdate(revision: String): Boolean {
        if (!REVISION.matches(revision)) return false
        update = revision
        submitUpdate()
        return true
    }

    @Suppress("DEPRECATION") // The path constructor also supports devices below API 29.
    private fun watchUpdates() {
        if (!isDebuggable()) return
        val directory = File(filesDir, "impasto-debug")
        if (!directory.isDirectory && !directory.mkdirs()) return
        val watcher = object : FileObserver(directory.path, FileObserver.MOVED_TO) {
            override fun onEvent(event: Int, path: String?) {
                if (path == null || !path.startsWith("v1-") || !path.endsWith(".js")) return
                val revision = path.removePrefix("v1-").removeSuffix(".js")
                runOnUiThread {
                    if (updateObserver === this) receiveUpdate(revision)
                }
            }
        }
        updateObserver = watcher
        watcher.startWatching()
        developmentUpdates = try {
            DevelopmentUpdates(packageName, directory) { revision, source, accepted ->
                val handle = updateHandle
                accepted(handle != null)
                if (handle != null) NativeBridge.nativeUpdateSource(handle, revision, source)
            }
        } catch (error: IOException) {
            Log.w("Impasto", "Could not open debug source socket", error)
            null
        }
    }

    private fun submitUpdate() {
        if (!submitted) return
        val handle = host?.view?.handle ?: return
        val revision = update ?: return
        update = null
        val dir = File(filesDir, "impasto-debug")
        updateReader.execute {
            val file = File(dir, "v1-$revision.js")
            if (!file.isFile) return@execute
            try {
                val length = file.length()
                if (length !in 1..MAX_SOURCE_BYTES) throw IOException("invalid source size")
                val bytes = ByteArray(length.toInt())
                file.inputStream().use {
                    java.io.DataInputStream(it).readFully(bytes)
                    if (it.read() != -1) throw IOException("source size changed")
                }
                file.delete()
                NativeBridge.nativeUpdateSource(handle, revision, bytes)
            } catch (error: IOException) {
                Log.e("Impasto", "Could not read debug source update", error)
                File(dir, "v1-$revision.result").writeText("error\nunreadable update")
            }
        }
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
        updateHandle = null
        developmentUpdates?.close()
        developmentUpdates = null
        updateObserver?.stopWatching()
        updateObserver = null
        host?.close()
        host = null
        super.onDestroy()
    }

    private companion object {
        const val DEBUG_UPDATE = "impasto.debug.update"
        const val MAX_SOURCE_BYTES = 16L * 1024 * 1024
        val REVISION = Regex("[0-9]{1,20}")
        val updateReader = Executors.newSingleThreadExecutor()
        const val DELIVERED_URL = "dev.impasto.delivered_url"
    }
}
