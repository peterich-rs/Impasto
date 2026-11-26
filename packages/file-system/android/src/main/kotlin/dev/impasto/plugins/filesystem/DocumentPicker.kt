package dev.impasto.plugins.filesystem

import android.app.Activity
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.OpenableColumns
import androidx.activity.result.ActivityResultRegistry
import androidx.activity.result.contract.ActivityResultContracts
import java.io.File
import java.io.IOException
import java.util.UUID
import java.util.concurrent.Executors

/** One Activity Result registration and at most one request for a View attachment. */
internal class DocumentPicker(
    private val activity: Activity,
    registry: ActivityResultRegistry,
    private val complete: (Long, String?, String?) -> Unit,
) : AutoCloseable {
    private val handler = Handler(Looper.getMainLooper())
    private var request: Long? = null
    private val launcher = registry.register(
        "impasto.document.${UUID.randomUUID()}",
        ActivityResultContracts.OpenDocument(),
        ::selected,
    )

    fun open(id: Long) {
        if (request != null) {
            complete(id, null, "A document picker is already open")
            return
        }
        request = id
        try {
            launcher.launch(arrayOf("*/*"))
        } catch (error: RuntimeException) {
            finish(id, error = "Could not open the document picker (${error.javaClass.simpleName})")
        }
    }

    private fun selected(uri: Uri?) {
        val id = request ?: return
        if (uri == null) {
            finish(id)
            return
        }
        worker.execute {
            val file = try {
                import(uri)
            } catch (error: Exception) {
                handler.post { finish(id, error = "Could not read the selected document (${error.javaClass.simpleName})") }
                return@execute
            }
            handler.post {
                if (request == id) {
                    finish(id, path = file.relativeTo(activity.filesDir).invariantSeparatorsPath)
                } else {
                    worker.execute { file.parentFile?.deleteRecursively() }
                }
            }
        }
    }

    private fun import(uri: Uri): File {
        val resolver = activity.contentResolver
        val original = resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
            ?.use { if (it.moveToFirst()) it.getString(0) else null }
        val name = original.orEmpty().take(72).map { character ->
            if (character == '/' || character == '\\' || character.isISOControl()) '_' else character
        }.joinToString("")
        val directory = File(activity.filesDir, "picked/${UUID.randomUUID()}")
        if (!directory.mkdirs()) throw IOException("could not create the import directory")
        val file = File(directory, name.takeUnless { it.isBlank() || it == "." || it == ".." } ?: "document")
        try {
            val input = resolver.openInputStream(uri) ?: throw IOException("no readable document stream")
            input.use { stream -> file.outputStream().use { stream.copyTo(it) } }
            return file
        } catch (error: Exception) {
            directory.deleteRecursively()
            throw error
        }
    }

    private fun finish(id: Long, path: String? = null, error: String? = null) {
        if (request != id) return
        request = null
        complete(id, path, error)
    }

    override fun close() {
        request = null
        launcher.unregister()
    }

    private companion object {
        val worker = Executors.newFixedThreadPool(2)
    }
}
