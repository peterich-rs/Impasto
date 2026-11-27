package dev.impasto.plugins.permissions

import android.app.Activity
import android.os.Handler
import android.os.Looper
import androidx.activity.result.ActivityResultRegistry
import androidx.activity.result.contract.ActivityResultContracts
import java.util.UUID

/** Owns the FIFO and rebinds its in-flight Activity Result across recreation. */
internal class Permissions(
    activity: Activity,
    registry: ActivityResultRegistry?,
    pending: Pending?,
    enabled: Set<String>?,
    private val isPending: (Long) -> Boolean,
    private val complete: (Long, Array<String>?, PermissionFailure?) -> Unit,
) : AutoCloseable {
    data class PendingRequest(val id: Long, val names: List<String>)

    enum class Pipeline { Runtime, FollowUp, Settings }

    data class Active(val request: PendingRequest, val next: Int = 0, val pipeline: Pipeline? = null)

    sealed interface Reply {
        data class Runtime(val result: Map<String, Boolean>) : Reply
        data object Settings : Reply
    }

    data class Pending(val key: String, val active: Active?, val queue: List<PendingRequest>, val reply: Reply?)

    val capabilities = PermissionCapabilities(activity, enabled)
    private val key = pending?.key ?: "impasto.permissions.${UUID.randomUUID()}"
    private val queue = ArrayDeque(pending?.queue.orEmpty())
    private var active = pending?.active
    private var reply = pending?.reply
    private var bound = true
    private val main = Handler(Looper.getMainLooper())
    val hasPendingRequest: Boolean get() = active != null || queue.isNotEmpty()

    private val runtimeLauncher = registry?.register(key, ActivityResultContracts.RequestMultiplePermissions()) { result ->
        receive(Reply.Runtime(result))
    }
    private val settingsLauncher = registry?.register("$key.settings", ActivityResultContracts.StartActivityForResult()) {
        receive(Reply.Settings)
    }

    init {
        if (pending != null) {
            if (registry == null) {
                failAll(PermissionFailure("host-unavailable", "Replacement host does not provide Activity Results"))
            } else if (reply != null) {
                main.post { consumeReply() }
            } else if (active?.pipeline == null) {
                main.post { advance() }
            }
        }
    }

    fun request(id: Long, names: List<String>) {
        queue.addLast(PendingRequest(id, names))
        if (active == null) advance()
    }

    private fun advance() {
        while (bound) {
            val current = active ?: queue.removeFirstOrNull()?.let { Active(it) } ?: return
            active = current
            if (current.pipeline != null) return
            val request = current.request
            if (!isPending(request.id)) {
                active = null
                continue
            }
            try {
                if (current.next == 0) {
                    request.names.forEach(capabilities::validateRequest)
                    val candidates = request.names.flatMap { name ->
                        if (capabilities.requiresFollowUp(name)) {
                            if (capabilities.hasForeground(name)) emptyList() else capabilities.foreground(name)
                        } else capabilities.runtime(name)
                    }
                    if (launchRuntime(current.copy(next = 1), Pipeline.Runtime, candidates)) return
                    active = current.copy(next = 1)
                    continue
                }

                val name = request.names.distinct().getOrNull(current.next - 1)
                if (name == null) {
                    val statuses = request.names.map(capabilities::status).toTypedArray()
                    active = null
                    complete(request.id, statuses, null)
                    continue
                }
                val next = current.copy(next = current.next + 1)
                active = next
                if (capabilities.status(name) == "granted") continue
                if (capabilities.requiresFollowUp(name) && !capabilities.hasForeground(name)) continue
                val settings = capabilities.requestSettings(name)
                if (settings != null) {
                    val launcher = settingsLauncher
                        ?: throw PermissionFailure("host-unavailable", "Host does not provide Activity Results")
                    active = next.copy(pipeline = Pipeline.Settings)
                    launcher.launch(settings)
                    return
                }
                if (capabilities.requiresFollowUp(name) &&
                    launchRuntime(next, Pipeline.FollowUp, capabilities.runtime(name))
                ) return
            } catch (failure: RuntimeException) {
                active = null
                complete(request.id, null, failure as? PermissionFailure ?: PermissionFailure(
                    "platform", "Could not request permissions (${failure.javaClass.simpleName})",
                ))
            }
        }
    }

    private fun launchRuntime(current: Active, pipeline: Pipeline, candidates: List<String>): Boolean {
        val permissions = capabilities.promptPermissions(candidates)
        if (permissions.isEmpty()) return false
        val launcher = runtimeLauncher ?: throw PermissionFailure("host-unavailable", "Host does not provide Activity Results")
        active = current.copy(pipeline = pipeline)
        launcher.launch(permissions)
        return true
    }

    private fun receive(value: Reply) {
        if (!bound) return
        reply = value
        main.post { consumeReply() }
    }

    private fun consumeReply() {
        val value = reply ?: return
        reply = null
        val current = active ?: return
        when (value) {
            is Reply.Runtime -> {
                if (current.pipeline != Pipeline.Runtime && current.pipeline != Pipeline.FollowUp) return
                capabilities.recordResult(value.result)
            }
            Reply.Settings -> if (current.pipeline != Pipeline.Settings) return
        }
        active = current.copy(pipeline = null)
        advance()
    }

    fun unbind(): Pending? {
        if (!bound) return null
        bound = false
        val saved = if (hasPendingRequest) Pending(key, active, queue.toList(), reply) else null
        active = null
        reply = null
        queue.clear()
        runtimeLauncher?.unregister()
        settingsLauncher?.unregister()
        return saved
    }

    private fun failAll(error: PermissionFailure) {
        val saved = unbind() ?: return
        saved.active?.let { complete(it.request.id, null, error) }
        for (request in saved.queue) complete(request.id, null, error)
    }

    override fun close() = failAll(PermissionFailure("cancelled", "Permission request cancelled with host"))
}
