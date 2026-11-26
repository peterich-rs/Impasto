package dev.impasto.plugins.linking

import android.content.Intent
import android.net.Uri
import dev.impasto.host.ImpastoHost

/** Outbound URL and share operations for the current Activity. */
object LinkingPlugin {
    fun register(host: ImpastoHost) {
        host.registerNativePlugin("@impasto/linking") { request ->
            if (request.payload.isBlank()) {
                request.fail("The URL or shared text is empty")
            } else {
                val intent = when (request.operation) {
                    "@impasto/linking/open" -> Intent(Intent.ACTION_VIEW, Uri.parse(request.payload))
                    "@impasto/linking/share" -> Intent.createChooser(
                        Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, request.payload), null,
                    )
                    else -> null
                }
                if (intent == null) {
                    request.fail("Unknown linking operation")
                } else {
                    host.activity.startActivity(intent)
                    request.resolve()
                }
            }
        }
    }
}
