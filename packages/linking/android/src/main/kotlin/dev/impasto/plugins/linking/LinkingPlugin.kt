package dev.impasto.plugins.linking

import android.content.ActivityNotFoundException
import android.content.Intent
import android.graphics.Color
import android.net.Uri
import androidx.browser.customtabs.CustomTabColorSchemeParams
import androidx.browser.customtabs.CustomTabsIntent
import dev.impasto.host.ImpastoHost
import org.json.JSONObject
import org.json.JSONTokener

object LinkingPlugin {
    private const val name = "@impasto/linking"

    fun register(host: ImpastoHost) {
        val activity = host.activity
        host.registerNativePlugin(name) { request ->
            when (request.operation) {
                "$name/canOpen" -> {
                    val uri = Uri.parse(JSONTokener(request.payload).nextValue() as String)
                    request.resolveJson(Intent(Intent.ACTION_VIEW, uri).resolveActivity(activity.packageManager) != null)
                }
                "$name/open" -> {
                    val input = JSONObject(request.payload)
                    val uri = Uri.parse(input.getString("url"))
                    val options = input.getJSONObject("options")
                    if (options.optString("mode") == "inAppBrowser") {
                        val builder = CustomTabsIntent.Builder()
                        if (!options.isNull("toolbarColor")) {
                            builder.setDefaultColorSchemeParams(CustomTabColorSchemeParams.Builder()
                                .setToolbarColor(Color.parseColor(options.getString("toolbarColor"))).build())
                        }
                        try {
                            builder.build().launchUrl(activity, uri)
                            request.resolveJson(true)
                            return@registerNativePlugin
                        } catch (_: ActivityNotFoundException) { /* Try the default handler. */ }
                    }
                    val opened = try { activity.startActivity(Intent(Intent.ACTION_VIEW, uri)); true }
                    catch (_: ActivityNotFoundException) { false }
                    request.resolveJson(opened)
                }
                else -> request.fail("Unknown linking operation")
            }
        }
    }
}
