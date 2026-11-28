package dev.impasto.host

import org.json.JSONObject

/** A one-shot call from a Rust module to an installed platform plugin. */
class ImpastoNativeRequest internal constructor(
    val id: Long,
    val operation: String,
    val payload: String,
    private val host: ImpastoHost,
) {
    val isPending: Boolean get() = host.isNativeRequestPending(id)
    fun resolve(payload: String? = null): Boolean = host.completeNativeRequest(id, payload, null)
    fun fail(message: String): Boolean = host.completeNativeRequest(id, null, message)
    fun resolveJson(value: Any?): Boolean = resolve(
        if (value is String) JSONObject.quote(value) else value?.toString() ?: "null",
    )
    fun fail(code: String, message: String): Boolean =
        fail(JSONObject().put("code", code).put("message", message).toString())
}
