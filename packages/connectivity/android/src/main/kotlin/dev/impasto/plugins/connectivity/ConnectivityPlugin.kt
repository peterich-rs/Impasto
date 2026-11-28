package dev.impasto.plugins.connectivity

import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.os.Handler
import android.os.Looper
import dev.impasto.host.ImpastoHost
import org.json.JSONArray
import org.json.JSONObject

object ConnectivityPlugin {
    private const val name = "@impasto/connectivity"
    private const val topic = "$name/status"

    fun register(host: ImpastoHost) {
        val manager = host.activity.getSystemService(ConnectivityManager::class.java)
        val monitor = Monitor(host, manager)
        val retained = host.takeRetainedPluginState(name) == true
        host.registerNativePlugin(name, hasPendingState = { monitor.listening }, detach = { retain ->
            val listening = monitor.listening
            monitor.stop()
            (retain && listening).takeIf { it }
        }) { request ->
            when (request.operation) {
                topic -> request.resolveJson(monitor.status())
                "@impasto/events/start" -> {
                    if (JSONObject(request.payload).optString("topic") != topic) {
                        request.fail("Unknown network topic")
                    } else {
                        monitor.start()
                        request.resolve()
                    }
                }
                "@impasto/events/stop" -> { monitor.stop(); request.resolve() }
                else -> request.fail("Unknown connectivity operation")
            }
        }
        if (retained) monitor.start()
    }

    private class Monitor(private val host: ImpastoHost, private val manager: ConnectivityManager?) {
        private val main = Handler(Looper.getMainLooper())
        private var callback: ConnectivityManager.NetworkCallback? = null
        private var prior: String? = null
        val listening: Boolean get() = callback != null

        fun status(): JSONObject {
            val capabilities = manager?.activeNetwork?.let(manager::getNetworkCapabilities)
            val connected = capabilities?.let {
                it.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
                    it.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
            } == true
            val transports = if (!connected) listOf("none") else listOf(
                NetworkCapabilities.TRANSPORT_WIFI to "wifi",
                NetworkCapabilities.TRANSPORT_CELLULAR to "cellular",
                NetworkCapabilities.TRANSPORT_ETHERNET to "ethernet",
                NetworkCapabilities.TRANSPORT_VPN to "vpn",
                NetworkCapabilities.TRANSPORT_BLUETOOTH to "bluetooth",
            ).filter { capabilities?.hasTransport(it.first) == true }.map { it.second }.ifEmpty { listOf("other") }
            return JSONObject().put("types", JSONArray(transports)).put("isConnected", connected)
                .put("isExpensive", connected && capabilities?.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED) == false)
                .put("isConstrained", manager?.restrictBackgroundStatus == ConnectivityManager.RESTRICT_BACKGROUND_STATUS_ENABLED)
        }

        fun start() {
            if (callback != null) return
            val manager = manager ?: return
            val observer = object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) = refresh()
                override fun onCapabilitiesChanged(network: Network, networkCapabilities: NetworkCapabilities) = refresh()
                override fun onLost(network: Network) = refresh()
                override fun onUnavailable() = refresh()
                private fun refresh() { main.post { if (callback === this) emit() } }
            }
            manager.registerDefaultNetworkCallback(observer)
            callback = observer
            emit()
        }

        fun stop() {
            val observer = callback ?: return
            callback = null
            prior = null
            manager?.unregisterNetworkCallback(observer)
        }

        private fun emit() {
            val next = status().toString()
            if (next != prior && host.emitPluginEvent(topic, next)) prior = next
        }
    }
}
