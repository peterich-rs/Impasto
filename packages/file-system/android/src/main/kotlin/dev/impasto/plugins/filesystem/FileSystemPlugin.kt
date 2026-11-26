package dev.impasto.plugins.filesystem

import androidx.activity.result.ActivityResultRegistryOwner
import dev.impasto.host.ImpastoHost
import java.io.File

/** Uses the attached Activity's result registry and imports the result into app-private storage. */
object FileSystemPlugin {
    fun register(host: ImpastoHost) {
        val activity = host.activity
        val picker = (activity as? ActivityResultRegistryOwner)?.activityResultRegistry?.let { registry ->
            DocumentPicker(activity, registry) { id, path, error ->
                if (!host.completeNativeRequest(id, path, error) && path != null) {
                    File(activity.filesDir, path).parentFile?.deleteRecursively()
                }
            }
        }
        host.registerNativePlugin("@impasto/file-system", detach = { picker?.close(); null }) { request ->
            if (request.operation == "@impasto/file-system/pick" && picker != null) {
                picker.open(request.id)
            } else {
                request.fail("Host does not provide a document picker")
            }
        }
    }
}
