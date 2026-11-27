package dev.impasto.plugins.permissions

import androidx.activity.result.ActivityResultRegistryOwner
import dev.impasto.host.ImpastoHost
import org.json.JSONArray
import org.json.JSONObject

/** Android permission operations for the current Activity. */
object PermissionsPlugin {
    private const val NAME = "@impasto/permissions"

    fun register(host: ImpastoHost, enabled: Set<String>? = null) {
        val permissions = Permissions(
            host.activity,
            (host.activity as? ActivityResultRegistryOwner)?.activityResultRegistry,
            host.takeRetainedPluginState(NAME) as? Permissions.Pending,
            enabled,
            host::isNativeRequestPending,
        ) { id, statuses, error ->
            host.completeNativeRequest(id, statuses?.let { JSONArray(it.toList()).toString() }, error?.encode())
        }
        host.registerNativePlugin(NAME, hasPendingState = { permissions.hasPendingRequest }, detach = { retain ->
            if (retain) permissions.unbind() else { permissions.close(); null }
        }) { request ->
            val capabilities = permissions.capabilities
            try {
                when (request.operation) {
                    "$NAME/status" -> request.resolve(capabilities.status(request.payload))
                    "$NAME/rationale" -> request.resolve(capabilities.rationale(request.payload).toString())
                    "$NAME/service" -> request.resolve(capabilities.service(request.payload))
                    "$NAME/settings" -> request.resolve(capabilities.openSettings(request.payload).toString())
                    "$NAME/limitedPicker" -> request.resolve("false")
                    "$NAME/request" -> {
                        val names = JSONObject(request.payload).getJSONArray("names")
                        permissions.request(request.id, List(names.length()) { names.getString(it) })
                    }
                    else -> request.fail("Unknown permission operation")
                }
            } catch (failure: PermissionFailure) {
                request.fail(failure.encode())
            }
        }
    }

    private fun PermissionFailure.encode(): String =
        JSONObject().put("code", code).put("message", message).toString()
}
