package dev.impasto.plugins.permissions

import androidx.activity.result.ActivityResultRegistryOwner
import dev.impasto.host.ImpastoHost
import org.json.JSONArray

/** Android permission operations for the current Activity. */
object PermissionsPlugin {
    private const val NAME = "@impasto/permissions"

    fun register(host: ImpastoHost) {
        val permissions = Permissions(
            host.activity,
            (host.activity as? ActivityResultRegistryOwner)?.activityResultRegistry,
            host.takeRetainedPluginState(NAME) as? Permissions.Pending,
        ) { id, statuses, error ->
            host.completeNativeRequest(id, statuses?.let { JSONArray(it.toList()).toString() }, error)
        }
        host.registerNativePlugin(NAME, hasPendingState = { permissions.hasPendingRequest }, detach = { retain ->
            if (retain) permissions.unbind() else { permissions.close(); null }
        }) { request ->
            when (request.operation) {
                "$NAME/status" -> request.resolve(permissions.status(request.payload))
                "$NAME/rationale" -> request.resolve(permissions.rationale(request.payload).toString())
                "$NAME/service" -> request.resolve(permissions.service(request.payload))
                "$NAME/settings" -> request.resolve(permissions.openSettings().toString())
                "$NAME/request" -> {
                    val names = JSONArray(request.payload)
                    permissions.request(request.id, Array(names.length()) { names.getString(it) })
                }
                else -> request.fail("Unknown permission operation")
            }
        }
    }
}
