package dev.impasto.plugins.share

import android.app.Activity
import android.app.PendingIntent
import android.content.ActivityNotFoundException
import android.content.BroadcastReceiver
import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.webkit.MimeTypeMap
import androidx.activity.result.ActivityResultRegistryOwner
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import dev.impasto.host.ImpastoHost
import org.json.JSONObject
import java.io.File
import java.util.UUID

class ShareFileProvider : FileProvider()

object SharePlugin {
    private const val name = "@impasto/share"
    private val main = Handler(Looper.getMainLooper())

    private class State(private val app: Context) : AutoCloseable {
        val key = "impasto.share.${UUID.randomUUID()}"
        var request: Long? = null
        var outcome: String? = null
        var delivery: ((Long, String) -> Unit)? = null
        var receiver: BroadcastReceiver? = null
        var callback: PendingIntent? = null

        fun selection(): PendingIntent {
            val action = "${app.packageName}.$key.${UUID.randomUUID()}"
            val observer = object : BroadcastReceiver() {
                override fun onReceive(context: Context, intent: Intent) { finish("success") }
            }
            ContextCompat.registerReceiver(app, observer, IntentFilter(action), ContextCompat.RECEIVER_NOT_EXPORTED)
            receiver = observer
            val flags = PendingIntent.FLAG_ONE_SHOT or if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0
            return PendingIntent.getBroadcast(app, 0, Intent(action).setPackage(app.packageName), flags).also { callback = it }
        }

        fun finish(status: String) {
            if (request == null || outcome != null) return
            release()
            outcome = status
            deliver()
        }

        fun release() {
            receiver?.let(app::unregisterReceiver)
            receiver = null
            callback?.cancel()
            callback = null
        }

        override fun close() {
            request = null
            outcome = null
            delivery = null
            release()
        }

        fun deliver() {
            val send = delivery ?: return
            val id = request ?: return
            val status = outcome ?: return
            request = null
            outcome = null
            send(id, status)
        }
    }

    fun register(host: ImpastoHost) {
        val activity = host.activity
        val state = host.takeRetainedPluginState(name) as? State ?: State(activity.applicationContext)
        val launcher = (activity as? ActivityResultRegistryOwner)?.activityResultRegistry?.register(
            state.key, ActivityResultContracts.StartActivityForResult(),
        ) { result ->
            val id = state.request
            if (result.resultCode == Activity.RESULT_OK) state.finish("success")
            else main.postDelayed({ if (id != null && state.request == id) state.finish("dismissed") }, 350)
        }
        state.delivery = { id, status -> host.completeNativeRequest(id, JSONObject().put("status", status).toString()) }
        main.post(state::deliver)
        host.registerNativePlugin(name, hasPendingState = { state.request != null }, detach = { retain ->
            if (!retain) {
                state.request?.let { id -> host.completeNativeRequest(id, null,
                    JSONObject().put("code", "host-unavailable").put("message", "The share sheet detached").toString()) }
                state.close()
            }
            state.delivery = null
            launcher?.unregister()
            state.takeIf { retain && it.request != null }
        }) { request ->
            when {
                request.operation != "$name/share" -> request.fail("Unknown share operation")
                state.request != null -> request.fail("busy", "A share sheet is already open")
                launcher == null -> request.fail("host-unavailable", "The window has no Activity Result registry")
                else -> {
                    try {
                        val options = JSONObject(request.payload)
                        val intent = send(activity, options)
                        state.request = request.id
                        val chooser = Intent.createChooser(intent, options.nonNull("title"), state.selection().intentSender)
                        launcher.launch(chooser)
                    } catch (_: ActivityNotFoundException) {
                        state.finish("unavailable")
                    } catch (error: Exception) {
                        state.release()
                        state.request = null
                        request.fail("platform", error.message ?: "Could not open the share sheet")
                    }
                }
            }
        }
    }

    private fun send(activity: Activity, options: JSONObject): Intent {
        val files = options.optJSONArray("files")
        val uris = ArrayList<Uri>()
        val types = ArrayList<String>()
        if (files != null) for (index in 0 until files.length()) {
            val input = files.getJSONObject(index)
            val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.impasto.share.fileprovider",
                File(input.getString("path")), input.getString("name"))
            uris.add(uri)
            types.add(input.nonNull("mimeType") ?: MimeTypeMap.getSingleton().getMimeTypeFromExtension(
                input.getString("name").substringAfterLast('.', "").lowercase(),
            ) ?: "application/octet-stream")
        }
        val text = listOfNotNull(options.nonNull("text"), options.nonNull("url")).joinToString("\n")
        val intent = Intent(if (uris.size > 1) Intent.ACTION_SEND_MULTIPLE else Intent.ACTION_SEND)
            .setType(mime(types)).putExtra(Intent.EXTRA_TEXT, text).putExtra(Intent.EXTRA_SUBJECT, options.nonNull("subject"))
        if (uris.isNotEmpty()) {
            val clip = ClipData.newRawUri(options.nonNull("title") ?: "Attachment", uris.first())
            for (uri in uris.drop(1)) clip.addItem(ClipData.Item(uri))
            intent.clipData = clip
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            if (uris.size == 1) intent.putExtra(Intent.EXTRA_STREAM, uris.first())
            else intent.putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris)
        }
        return intent
    }

    private fun mime(types: List<String>): String {
        if (types.isEmpty()) return "text/plain"
        val first = types.first()
        if (types.all { it == first }) return first
        val category = first.substringBefore('/')
        if (types.all { it.substringBefore('/') == category }) return "$category/*"
        return "*/*"
    }

    private fun JSONObject.nonNull(key: String): String? = if (isNull(key)) null else optString(key)
}
