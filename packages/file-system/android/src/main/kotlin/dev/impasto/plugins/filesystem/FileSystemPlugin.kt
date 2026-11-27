package dev.impasto.plugins.filesystem

import androidx.activity.result.ActivityResultRegistryOwner
import dev.impasto.host.ImpastoHost
import org.json.JSONObject

/** Imports files and presents document dialogs through the attached Activity. */
object FileSystemPlugin {
    private const val name = "@impasto/file-system"

    fun register(host: ImpastoHost) {
        val state = host.takeRetainedPluginState(name) as? DocumentPicker.State ?: DocumentPicker.State()
        val picker = (host.activity as? ActivityResultRegistryOwner)?.activityResultRegistry?.let { registry ->
            DocumentPicker(host.activity, registry, state) { id, value, failure ->
                val error = failure?.let {
                    val code = when (it) {
                        is DocumentPicker.Cancelled -> "cancelled"
                        is SecurityException -> "permission-denied"
                        else -> "io"
                    }
                    JSONObject().put("code", code).put("message", it.message ?: "Document operation failed").toString()
                }
                host.completeNativeRequest(id, value?.toString(), error)
            }
        }
        host.registerNativePlugin(name, hasPendingState = { state.pending != null }, detach = { picker?.detach(it) }) { request ->
            val action = request.operation.substringAfterLast('/')
            if (picker == null) request.fail("host-unavailable", "The window has no Activity Result registry")
            else if (action in setOf("pickFiles", "pickDirectory", "saveFile")) picker.open(request, action)
            else request.fail("Unknown document operation")
        }
    }
}
