package dev.impasto.plugins.filesystem

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import androidx.activity.result.ActivityResultRegistry
import androidx.activity.result.contract.ActivityResultContract
import androidx.activity.result.contract.ActivityResultContracts
import dev.impasto.host.ImpastoNativeRequest
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.util.UUID
import java.util.concurrent.Executors

internal class DocumentPicker(
    activity: Activity,
    registry: ActivityResultRegistry,
    private val state: State,
    complete: (Long, Any?, Throwable?) -> Boolean,
) {
    enum class Operation { Files, Directory, Save }
    data class Pending(val id: Long, val operation: Operation, val source: File? = null)
    private data class Save(val name: String, val mime: String)
    data class Reply(val id: Long, val value: Any?, val failure: Throwable?, val imports: List<File>)

    class State : AutoCloseable {
        val key = "impasto.document.${UUID.randomUUID()}"
        var pending: Pending? = null
        var reply: Reply? = null
        var delivery: ((Long, Any?, Throwable?) -> Boolean)? = null

        fun finish(reply: Reply) {
            if (pending?.id == reply.id) {
                this.reply = reply
                deliver()
            } else {
                discard(reply.imports)
            }
        }

        fun deliver() {
            val send = delivery ?: return
            val result = reply ?: return
            pending = null
            reply = null
            if (!send(result.id, result.value, result.failure)) discard(result.imports)
        }

        fun cancel() {
            val current = pending ?: return
            discard(reply?.imports.orEmpty())
            reply = null
            finish(Reply(current.id, null, Cancelled(), emptyList()))
        }

        override fun close() {
            delivery = null
            discard(reply?.imports.orEmpty())
            pending = null
            reply = null
        }
    }

    class Cancelled : IOException("Document picker detached")

    private val app = activity.applicationContext
    private val resolver = app.contentResolver
    private val single = registry.register("${state.key}.single", ActivityResultContracts.OpenDocument()) {
        receive(Operation.Files, listOfNotNull(it))
    }
    private val multiple = registry.register("${state.key}.multiple", ActivityResultContracts.OpenMultipleDocuments()) {
        receive(Operation.Files, it)
    }
    private val directory = registry.register("${state.key}.directory", ActivityResultContracts.OpenDocumentTree()) {
        receive(Operation.Directory, listOfNotNull(it))
    }
    private val save = registry.register("${state.key}.save", object : ActivityResultContract<Save, Uri?>() {
        override fun createIntent(context: Context, input: Save) =
            ActivityResultContracts.CreateDocument(input.mime).createIntent(context, input.name)
        override fun parseResult(resultCode: Int, intent: Intent?): Uri? =
            ActivityResultContracts.CreateDocument("*/*").parseResult(resultCode, intent)
    }) { receive(Operation.Save, listOfNotNull(it)) }

    init {
        state.delivery = complete
        handler.post(state::deliver)
    }

    fun open(request: ImpastoNativeRequest, action: String) {
        if (state.pending != null) {
            request.fail("host-unavailable", "A document picker is already open")
            return
        }
        try {
            when (action) {
                "pickFiles" -> {
                    val payload = JSONObject(request.payload)
                    val types = mimeTypes(payload.optJSONArray("accept"))
                    state.pending = Pending(request.id, Operation.Files)
                    if (payload.optBoolean("multiple")) multiple.launch(types) else single.launch(types)
                }
                "pickDirectory" -> {
                    state.pending = Pending(request.id, Operation.Directory)
                    directory.launch(null)
                }
                "saveFile" -> {
                    val payload = JSONObject(request.payload)
                    state.pending = Pending(request.id, Operation.Save, File(payload.getString("path")))
                    val mime = payload.optString("mimeType", "*/*").takeUnless { it == "null" } ?: "*/*"
                    save.launch(Save(payload.getString("suggestedName"), mime))
                }
            }
        } catch (error: Exception) {
            state.pending = null
            request.fail("io", error.message ?: "Could not open the document picker")
        }
    }

    private fun receive(operation: Operation, uris: List<Uri>) {
        val pending = state.pending?.takeIf { it.operation == operation } ?: return
        worker.execute {
            val imported = mutableListOf<File>()
            val reply = try {
                val value = when (operation) {
                    Operation.Files -> JSONArray(uris.map { import(it, imported) })
                    Operation.Directory -> uris.firstOrNull()?.let { uri ->
                        val id = DocumentsContract.getTreeDocumentId(uri)
                        val document = DocumentsContract.buildDocumentUriUsingTree(uri, id)
                        resolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
                        JSONObject().put("path", uri.toString())
                            .put("name", displayName(document) ?: id.substringAfterLast('/').substringAfterLast(':'))
                    }
                    Operation.Save -> uris.firstOrNull()?.let { uri ->
                        val source = pending.source ?: throw IOException("Export source is unavailable")
                        val output = resolver.openOutputStream(uri, "wt") ?: throw IOException("No writable export stream")
                        output.use { stream -> source.inputStream().use { it.copyTo(stream) } }
                        true
                    } ?: false
                }
                Reply(pending.id, value, null, imported.toList())
            } catch (error: Exception) {
                discard(imported)
                Reply(pending.id, null, error, emptyList())
            }
            handler.post { state.finish(reply) }
        }
    }

    private fun displayName(uri: Uri): String? =
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst()) it.getString(0) else null
        }

    private fun import(uri: Uri, imports: MutableList<File>): JSONObject {
        val name = displayName(uri).orEmpty().take(72).map { ch ->
            if (ch in "/\\:" || ch.isISOControl()) '_' else ch
        }.joinToString("").takeUnless { it.isBlank() || it == "." || it == ".." } ?: "document"
        val base = app.filesDir.canonicalFile
        val root = File(base, "Imports")
        root.mkdir()
        if (root.canonicalFile != root || !root.isDirectory) throw IOException("Invalid import directory")
        val parent = File(root, UUID.randomUUID().toString())
        if (!parent.mkdir()) throw IOException("Could not create the import directory")
        imports.add(parent)
        val file = File(parent, name)
        val input = resolver.openInputStream(uri) ?: throw IOException("No readable document stream")
        val size = input.use { stream -> file.outputStream().use { stream.copyTo(it) } }
        return JSONObject().put("name", name).put("relativePath", file.relativeTo(base).invariantSeparatorsPath)
            .put("size", size).put("mimeType", resolver.getType(uri) ?: JSONObject.NULL)
    }

    fun detach(retain: Boolean): State? {
        if (!retain) state.cancel()
        state.delivery = null
        single.unregister()
        multiple.unregister()
        directory.unregister()
        save.unregister()
        return state.takeIf { retain && it.pending != null }
    }

    companion object {
        private val handler = Handler(Looper.getMainLooper())
        private val worker = Executors.newFixedThreadPool(2)
        private fun discard(imports: List<File>) {
            if (imports.isNotEmpty()) worker.execute { imports.forEach { it.deleteRecursively() } }
        }
        private fun mimeTypes(values: JSONArray?): Array<String> {
            if (values == null || values.length() == 0) return arrayOf("*/*")
            return Array(values.length()) { position ->
                val value = values.getString(position).lowercase()
                when (value) {
                    "image", "audio", "video" -> "$value/*"
                    "document" -> "*/*"
                    else -> when {
                        value.startsWith('.') -> MimeTypeMap.getSingleton().getMimeTypeFromExtension(value.drop(1))
                            ?: throw IOException("Unknown file extension $value")
                        '/' in value -> value
                        else -> throw IOException("Unknown file type $value")
                    }
                }
            }
        }
    }
}
