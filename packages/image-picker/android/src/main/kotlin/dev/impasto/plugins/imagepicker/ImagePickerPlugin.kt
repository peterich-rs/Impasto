package dev.impasto.plugins.imagepicker

import android.app.Activity
import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.graphics.BitmapFactory
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import androidx.activity.result.ActivityResultRegistryOwner
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContract
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.FileProvider
import dev.impasto.host.ImpastoHost
import dev.impasto.host.ImpastoNativeRequest
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.util.UUID
import java.util.concurrent.Executors

class ImagePickerFileProvider : FileProvider()

object ImagePickerPlugin {
    private const val name = "@impasto/image-picker"
    private val main = Handler(Looper.getMainLooper())
    private val worker = Executors.newFixedThreadPool(2)

    private data class Pending(val id: Long, val operation: String, val directory: File, val limit: Int, val capture: File? = null, val process: Boolean = false)
    private data class Capture(val uri: Uri, val video: Boolean, val duration: Int, val quality: String, val front: Boolean)
    private class State : AutoCloseable {
        val key = "impasto.image." + UUID.randomUUID()
        var pending: Pending? = null
        var reply: Pair<JSONArray?, String?>? = null
        var delivery: ((Long, JSONArray?, String?) -> Unit)? = null
        fun finish(operation: Pending, result: JSONArray?, error: String?) {
            if (pending !== operation) return
            reply = Pair(result, error)
            deliver()
        }
        fun deliver() {
            val operation = pending ?: return
            val result = reply ?: return
            val send = delivery ?: return
            pending = null
            reply = null
            send(operation.id, result.first, result.second)
        }
        override fun close() { pending = null; reply = null; delivery = null }
    }

    fun register(host: ImpastoHost, enabled: Set<String> = emptySet()) {
        val registry = (host.activity as? ActivityResultRegistryOwner)?.activityResultRegistry
        val state = host.takeRetainedPluginState(name) as? State ?: State()
        val context = host.activity.applicationContext
        val single = registry?.register(state.key + ".single", ActivityResultContracts.PickVisualMedia()) {
            receive(context, state, listOfNotNull(it))
        }
        val multiple = registry?.register(state.key + ".multiple", ActivityResultContracts.PickMultipleVisualMedia()) {
            receive(context, state, it)
        }
        val capture = registry?.register(state.key + ".capture", object : ActivityResultContract<Capture, Boolean>() {
            override fun createIntent(context: Context, input: Capture): Intent {
                val intent = if (input.video) ActivityResultContracts.CaptureVideo().createIntent(context, input.uri)
                else ActivityResultContracts.TakePicture().createIntent(context, input.uri)
                intent.clipData = ClipData.newRawUri("Capture", input.uri)
                intent.addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION or Intent.FLAG_GRANT_READ_URI_PERMISSION)
                if (input.front) intent.putExtra("android.intent.extras.CAMERA_FACING", 1)
                if (input.video) {
                    if (input.duration > 0) intent.putExtra(MediaStore.EXTRA_DURATION_LIMIT, input.duration)
                    intent.putExtra(MediaStore.EXTRA_VIDEO_QUALITY, if (input.quality == "low") 0 else 1)
                }
                return intent
            }
            override fun parseResult(resultCode: Int, intent: Intent?) = resultCode == Activity.RESULT_OK
        }) { success ->
            val operation = state.pending
            if (operation != null) {
                val file = operation.capture
                if (success && file != null) {
                    worker.execute {
                        val type = if (operation.operation == "video") "video/mp4" else "image/jpeg"
                        val result = runCatching { JSONArray().put(describe(file, file.name, type)) }
                        main.post { state.finish(operation, result.getOrNull(), if (result.isFailure) "io" else null) }
                    }
                } else state.finish(operation, JSONArray(), null)
            }
        }
        state.delivery = { id, result, error ->
            if (error == null) host.completeNativeRequest(id, result.toString())
            else host.completeNativeRequest(id, null, JSONObject().put("code", error).put("message", "Media import failed").toString())
        }
        main.post(state::deliver)
        host.registerNativePlugin(name, hasPendingState = { state.pending != null }, detach = { retain ->
            if (!retain) {
                state.pending?.let { state.finish(it, null, "cancelled") }
                state.close()
            }
            state.delivery = null
            single?.unregister()
            multiple?.unregister()
            capture?.unregister()
            state.takeIf { retain && it.pending != null }
        }) { request ->
            try {
                val action = request.operation.substringAfterLast('/')
                when {
                    registry == null -> request.fail("host-unavailable", "The host has no Activity Result registry")
                    action == "checkCamera" -> checkCamera(context, request, "camera" in enabled)
                    state.pending != null -> request.fail("busy", "A media picker is already open")
                    else -> {
                        val options = JSONObject(request.payload)
                        val base = File(context.cacheDir.canonicalFile, "impasto-image-picker-staging")
                        val directory = File(options.getString("staging")).canonicalFile
                        if (directory.parentFile != base || !directory.isDirectory) throw IOException()
                        if (action == "pick") {
                            val kind = when (options.optString("mediaType")) {
                                "video" -> ActivityResultContracts.PickVisualMedia.VideoOnly
                                "any" -> ActivityResultContracts.PickVisualMedia.ImageAndVideo
                                else -> ActivityResultContracts.PickVisualMedia.ImageOnly
                            }
                            val limit = options.optInt("maxSelection", Int.MAX_VALUE)
                            val process = options.optJSONObject("process")
                            val needsDecode = process != null && (
                                process.optBoolean("fixOrientation", true) || process.optBoolean("stripMetadata", true) ||
                                    !process.isNull("maxWidth") || !process.isNull("maxHeight") ||
                                    !process.isNull("quality") || process.optString("outputFormat", "original") != "original"
                                )
                            state.pending = Pending(request.id, action, directory, limit, process = needsDecode)
                            if (options.optBoolean("multiple") && limit > 1) {
                                val builder = PickVisualMediaRequest.Builder().setMediaType(kind)
                                if (limit < Int.MAX_VALUE) builder.setMaxItems(limit)
                                checkNotNull(multiple).launch(builder.build())
                            } else checkNotNull(single).launch(PickVisualMediaRequest(kind))
                        } else {
                            val video = action == "video"
                            val file = File(directory, if (video) "capture.mp4" else "capture.jpg")
                            val uri = FileProvider.getUriForFile(context, context.packageName + ".impasto.imagepicker.fileprovider", file)
                            state.pending = Pending(request.id, action, directory, 1, file)
                            checkNotNull(capture).launch(Capture(uri, video, options.optInt("maxDurationSec"), options.optString("quality"), options.optString("preferredCamera") == "front"))
                        }
                    }
                }
            } catch (_: android.content.ActivityNotFoundException) {
                state.pending = null
                request.fail("camera-unavailable", "No system media activity is available")
            } catch (_: Exception) {
                state.pending = null
                request.fail("io", "Could not open the system media picker")
            }
        }
    }

    private fun checkCamera(context: Context, request: ImpastoNativeRequest, enabled: Boolean) {
        @Suppress("DEPRECATION") val config = context.packageManager.getPackageInfo(context.packageName, android.content.pm.PackageManager.GET_PERMISSIONS)
        if (!enabled && config.requestedPermissions?.contains(android.Manifest.permission.CAMERA) != true) {
            request.fail("not-supported", "Enable camera permissions to capture media")
        } else if (!context.packageManager.hasSystemFeature(android.content.pm.PackageManager.FEATURE_CAMERA_ANY)) {
            request.fail("camera-unavailable", "No camera is available")
        } else request.resolve()
    }

    private fun receive(context: Context, state: State, selected: List<Uri>) {
        val operation = state.pending ?: return
        worker.execute {
            val result = runCatching {
                val resolver = context.contentResolver
                JSONArray(selected.take(operation.limit).map { uri ->
                    val mime = resolver.getType(uri) ?: throw IOException()
                    if (!mime.startsWith("image/") && !mime.startsWith("video/")) throw IOException()
                    val extension = MimeTypeMap.getSingleton().getExtensionFromMimeType(mime) ?: "bin"
                    val name = resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
                        if (it.moveToFirst()) it.getString(0) else null
                    } ?: "media.$extension"
                    val file = File(operation.directory, UUID.randomUUID().toString() + "." + extension)
                    resolver.openInputStream(uri)?.use { input -> file.outputStream().use { output -> input.copyTo(output) } } ?: throw IOException()
                    if (operation.process && mime in setOf("image/heic", "image/heif", "image/heic-sequence", "image/heif-sequence")
                        && android.os.Build.VERSION.SDK_INT >= 28
                    ) {
                        val decoded = decodeHeif(file)
                        describe(decoded, name, "image/png")
                    } else describe(file, name, mime)
                })
            }
            main.post { state.finish(operation, result.getOrNull(), if (result.isFailure) "io" else null) }
        }
    }

    @androidx.annotation.RequiresApi(28)
    private fun decodeHeif(file: File): File {
        val bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(file)) { decoder, info, _ ->
            val (width, height) = info.size.let { it.width to it.height }
            val dimension = maxOf(width, height)
            if (dimension <= 0) throw IOException()
            val sample = maxOf(1, (dimension - 1) / 4096 + 1)
            decoder.setTargetSampleSize(sample)
            decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
        }
        val converted = File(file.parentFile, UUID.randomUUID().toString() + ".png")
        try {
            converted.outputStream().use { if (!bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) throw IOException() }
        } finally { bitmap.recycle() }
        return converted
    }

    private fun describe(file: File, name: String, mime: String): JSONObject {
        if (!file.isFile || file.length() == 0L) throw IOException()
        val video = mime.startsWith("video/")
        val info = JSONObject().put("kind", if (video) "video" else "image").put("path", file.path)
            .put("fileName", name).put("mimeType", mime)
        if (video) {
            val media = MediaMetadataRetriever()
            try {
                media.setDataSource(file.path)
                info.put("width", media.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)?.toIntOrNull())
                info.put("height", media.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)?.toIntOrNull())
                info.put("durationMs", media.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull())
            } finally { media.release() }
        } else {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeFile(file.path, bounds)
            if (bounds.outWidth > 0 && bounds.outHeight > 0) info.put("width", bounds.outWidth).put("height", bounds.outHeight)
        }
        return info
    }
}
