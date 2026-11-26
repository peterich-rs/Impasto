package dev.impasto.host

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
}
