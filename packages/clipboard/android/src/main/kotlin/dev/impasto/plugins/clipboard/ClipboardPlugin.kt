package dev.impasto.plugins.clipboard

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Base64
import androidx.core.content.FileProvider
import dev.impasto.host.ImpastoHost
import dev.impasto.host.ImpastoNativeRequest
import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.OutputStream
import java.util.UUID
import java.util.concurrent.Executors

class ClipboardFileProvider : FileProvider()

object ClipboardPlugin {
    private const val name = "@impasto/clipboard"
    private const val topic = "$name/change"
    private const val imageLimit = 32 * 1024 * 1024
    private const val cacheLifetimeMs = 24 * 60 * 60 * 1000L
    private val worker = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())

    fun register(host: ImpastoHost) {
        val session = Session(host)
        val retained = host.takeRetainedPluginState(name) == true
        host.registerNativePlugin(name, hasPendingState = { session.listening }, detach = { retain ->
            val listening = session.listening
            session.stop()
            (retain && listening).takeIf { it }
        }) { request -> session.handle(request) }
        if (retained) session.start()
    }

    private class Session(private val host: ImpastoHost) {
        private val app = host.activity.applicationContext
        private val clipboard = checkNotNull(host.activity.getSystemService(ClipboardManager::class.java))
        private val listener = ClipboardManager.OnPrimaryClipChangedListener {
            if (listening) host.emitPluginEvent(topic, JSONArray(formats()).toString())
        }
        var listening = false
            private set

        fun start() {
            if (listening) return
            clipboard.addPrimaryClipChangedListener(listener)
            listening = true
        }

        fun stop() {
            if (!listening) return
            listening = false
            clipboard.removePrimaryClipChangedListener(listener)
        }

        fun handle(request: ImpastoNativeRequest) {
            when (request.operation) {
                "$name/hasStrings" -> request.resolveJson(formats().contains("text"))
                "$name/hasImages" -> request.resolveJson(formats().contains("image"))
                "$name/hasHtml" -> request.resolveJson(formats().contains("html"))
                "$name/availableFormats" -> request.resolveJson(JSONArray(formats()))
                "$name/read" -> {
                    val clip = clipboard.primaryClip
                    run(request, { read(app, clip) }) { request.resolveJson(it) }
                }
                "$name/write" -> {
                    val value = JSONObject(request.payload)
                    run(request, { write(app, value) }) { clip ->
                        clipboard.setPrimaryClip(clip)
                        request.resolve()
                    }
                }
                "$name/clear" -> {
                    if (Build.VERSION.SDK_INT >= 28) clipboard.clearPrimaryClip()
                    else clipboard.setPrimaryClip(ClipData.newPlainText("", ""))
                    request.resolve()
                }
                "@impasto/events/start" -> {
                    if (JSONObject(request.payload).optString("topic") != topic) request.fail("Unknown clipboard topic")
                    else { start(); request.resolve() }
                }
                "@impasto/events/stop" -> { stop(); request.resolve() }
                else -> request.fail("Unknown clipboard operation")
            }
        }

        private fun formats(): List<String> {
            val info = clipboard.primaryClipDescription ?: return emptyList()
            return listOfNotNull(
                "text".takeIf { info.hasMimeType(ClipDescription.MIMETYPE_TEXT_PLAIN) },
                "html".takeIf { info.hasMimeType(ClipDescription.MIMETYPE_TEXT_HTML) },
                "uri".takeIf { info.hasMimeType(ClipDescription.MIMETYPE_TEXT_URILIST) },
                "image".takeIf { info.hasMimeType("image/*") },
            )
        }
    }

    private fun <T> run(request: ImpastoNativeRequest, work: () -> T, finish: (T) -> Unit) {
        fun fail(error: Exception) {
            request.fail(if (error is InvalidImage) "invalid-image" else "platform", error.message ?: "Clipboard operation failed")
        }
        worker.execute {
            try {
                val result = work()
                main.post {
                    if (request.isPending) try { finish(result) } catch (error: Exception) { fail(error) }
                }
            } catch (error: Exception) { main.post { fail(error) } }
        }
    }

    private fun read(app: Context, clip: ClipData?): JSONObject {
        val result = JSONObject()
        if (clip == null) return result
        val resolver = app.contentResolver
        for (index in 0 until clip.itemCount) {
            val item = clip.getItemAt(index)
            if (!result.has("text")) item.text?.toString()?.let { result.put("text", it) }
            if (!result.has("html")) item.htmlText?.let { result.put("html", it) }
            val uri = item.uri ?: continue
            val image = uri.scheme == "content" &&
                (resolver.getType(uri)?.startsWith("image/") == true || clip.description.hasMimeType("image/*") && clip.itemCount == 1)
            if (image) {
                if (!result.has("imagePng")) result.put("imagePng", image(app, uri))
            } else if (!result.has("uri")) result.put("uri", uri.toString())
        }
        return result
    }

    private fun write(app: Context, value: JSONObject): ClipData {
        fun field(key: String): String? = if (value.isNull(key)) null else value.getString(key)
        val text = field("text")
        val html = field("html")
        val uri = field("uri")?.let(Uri::parse)
        val png = field("imagePng")
        val items = mutableListOf<ClipData.Item>()
        val types = mutableListOf<String>()
        if (text != null || html != null || uri != null) {
            items.add(ClipData.Item(text ?: if (html != null) "" else null, html, null, uri))
            if (text != null) types.add(ClipDescription.MIMETYPE_TEXT_PLAIN)
            if (html != null) types.add(ClipDescription.MIMETYPE_TEXT_HTML)
            if (uri != null) types.add(ClipDescription.MIMETYPE_TEXT_URILIST)
        }
        if (png != null) {
            if (png.length > (imageLimit + 2) / 3 * 4) throw InvalidImage()
            val bytes = Base64.decode(png, Base64.DEFAULT)
            if (bytes.size > imageLimit) throw InvalidImage()
            val folder = File(app.cacheDir.canonicalFile, "impasto-clipboard")
            folder.mkdir()
            check(folder.canonicalFile == folder && folder.isDirectory) { "Invalid clipboard image directory" }
            folder.listFiles()?.forEach { if (System.currentTimeMillis() - it.lastModified() > cacheLifetimeMs) it.delete() }
            val image = File(folder, "${UUID.randomUUID()}.png")
            try {
                image.writeBytes(bytes)
                val imageUri = FileProvider.getUriForFile(app, "${app.packageName}.impasto.clipboard.fileprovider", image)
                items.add(ClipData.Item(imageUri))
                types.add("image/png")
            } catch (error: Exception) {
                image.delete()
                throw error
            }
        }
        val clip = ClipData("Clipboard", types.toTypedArray(), items.first())
        for (item in items.drop(1)) clip.addItem(item)
        return clip
    }

    private class InvalidImage : IllegalArgumentException("Clipboard images must fit within 32 MiB")

    private fun image(app: Context, uri: Uri): String {
        val source = BoundedImageBuffer()
        app.contentResolver.openInputStream(uri)?.use { it.copyTo(source) } ?: throw InvalidImage()
        val encoded = source.toByteArray()
        val dimensions = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(encoded, 0, encoded.size, dimensions)
        if (dimensions.outWidth < 1 || dimensions.outHeight < 1 ||
            dimensions.outWidth.toLong() * dimensions.outHeight > imageLimit / 4
        ) throw InvalidImage()
        val bitmap = BitmapFactory.decodeByteArray(encoded, 0, encoded.size) ?: throw InvalidImage()
        try {
            val bounded = BoundedImageBuffer()
            if (!bitmap.compress(Bitmap.CompressFormat.PNG, 100, bounded)) throw InvalidImage()
            return Base64.encodeToString(bounded.toByteArray(), Base64.NO_WRAP)
        } finally { bitmap.recycle() }
    }

    private class BoundedImageBuffer : OutputStream() {
        private val bytes = ByteArrayOutputStream()

        override fun write(value: Int) {
            if (bytes.size() >= imageLimit) throw InvalidImage()
            bytes.write(value)
        }

        override fun write(value: ByteArray, offset: Int, length: Int) {
            if (length > imageLimit - bytes.size()) throw InvalidImage()
            bytes.write(value, offset, length)
        }

        fun toByteArray(): ByteArray = bytes.toByteArray()
    }
}
