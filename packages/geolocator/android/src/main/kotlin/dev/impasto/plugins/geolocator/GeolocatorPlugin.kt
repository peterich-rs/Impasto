package dev.impasto.plugins.geolocator

import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.location.Location
import android.location.LocationManager
import android.os.SystemClock
import android.os.Build
import android.os.Handler
import android.os.Looper
import androidx.core.content.ContextCompat
import androidx.core.location.LocationCompat
import androidx.core.location.LocationListenerCompat
import androidx.core.location.LocationManagerCompat
import androidx.core.location.LocationRequestCompat
import android.os.CancellationSignal
import dev.impasto.host.ImpastoHost
import dev.impasto.host.ImpastoNativeRequest
import org.json.JSONObject

object GeolocatorPlugin {
    private const val name = "@impasto/geolocator"
    private const val positions = "$name/position"
    private const val service = "$name/service"

    fun register(host: ImpastoHost, enabled: Set<String> = emptySet()) {
        val session = Session(host, "background" in enabled)
        host.registerNativePlugin(name, detach = { session.close(); null }) { session.handle(it) }
    }

    @SuppressLint("MissingPermission")
    private class Session(private val host: ImpastoHost, private val background: Boolean) {
        private val context = host.activity.applicationContext
        private val manager = checkNotNull(context.getSystemService(LocationManager::class.java))
        private val main = Handler(Looper.getMainLooper())
        private val executor = ContextCompat.getMainExecutor(context)
        private val active = mutableMapOf<Long, Current>()
        private val cancelled = LinkedHashSet<Long>()
        private var listener: LocationListenerCompat? = null
        private var receiver: BroadcastReceiver? = null
        private var newest: Location? = null

        private class Current(val request: ImpastoNativeRequest, val timer: Runnable) {
            val signals = mutableListOf<CancellationSignal>()
            val listeners = mutableListOf<LocationListenerCompat>()
            var candidates = 0
        }

        fun handle(request: ImpastoNativeRequest) {
            try {
                when (request.operation) {
                    service -> request.resolveJson(LocationManagerCompat.isLocationEnabled(manager))
                    "$name/last" -> request.resolveJson(last(providers(true))?.let(::json))
                    "$name/current" -> current(request, JSONObject(request.payload))
                    "$name/cancel" -> {
                        request.payload.toLongOrNull()?.let { id ->
                            if (id in active) finish(id, null, "cancelled")
                            else {
                                cancelled.add(id)
                                if (cancelled.size > 128) cancelled.remove(cancelled.first())
                            }
                        }
                        request.resolve()
                    }
                    "@impasto/events/start" -> {
                        val control = JSONObject(request.payload)
                        when (control.getString("topic")) {
                            positions -> watch(JSONObject(control.optString("params", "{}")))
                            service -> watchService()
                        }
                        request.resolve()
                    }
                    "@impasto/events/stop" -> {
                        when (JSONObject(request.payload).getString("topic")) {
                            positions -> stopWatch()
                            service -> stopService()
                        }
                        request.resolve()
                    }
                    else -> request.fail("platform", "Unknown location operation")
                }
            } catch (_: SecurityException) {
                request.fail("permission-denied", "Location permission is unavailable")
            } catch (_: Exception) {
                request.fail("platform", "Could not configure location")
            }
        }

        private fun providers(high: Boolean): List<String> {
            val enabled = manager.getProviders(true)
            if (Build.VERSION.SDK_INT >= 31 && LocationManager.FUSED_PROVIDER in enabled) return listOf(LocationManager.FUSED_PROVIDER)
            val priority = if (high) listOf(LocationManager.GPS_PROVIDER, LocationManager.NETWORK_PROVIDER, LocationManager.PASSIVE_PROVIDER)
                else listOf(LocationManager.NETWORK_PROVIDER, LocationManager.PASSIVE_PROVIDER, LocationManager.GPS_PROVIDER)
            return priority.filter(enabled::contains)
        }

        private fun last(providers: List<String>): Location? =
            (providers.mapNotNull { runCatching { manager.getLastKnownLocation(it) }.getOrNull() } + listOfNotNull(newest))
                .filter { it.hasAccuracy() }.maxByOrNull { it.elapsedRealtimeNanos }

        private fun current(request: ImpastoNativeRequest, options: JSONObject) {
            val id = options.getLong("id")
            if (cancelled.remove(id)) {
                request.fail("cancelled", "Location request cancelled")
                return
            }
            val high = options.optString("accuracy") !in listOf("lowest", "low")
            val providers = providers(high)
            if (providers.isEmpty()) { request.fail("service-disabled", "There are no enabled location providers"); return }
            val maximumAge = options.optLong("maximumAgeMs", 0)
            val started = SystemClock.elapsedRealtimeNanos()
            val earliest = started - java.util.concurrent.TimeUnit.MILLISECONDS.toNanos(maximumAge)
            val cached = last(providers)
            if (cached != null && maximumAge > 0 && cached.elapsedRealtimeNanos >= earliest) {
                request.resolveJson(json(cached))
                return
            }
            val timer = Runnable { finish(id, null, "timeout") }
            val operation = Current(request, timer)
            active[id] = operation
            main.postDelayed(timer, options.optLong("timeoutMs", 15_000))
            for (provider in providers) {
                val signal = CancellationSignal()
                operation.signals.add(signal)
                operation.candidates++
                try {
                    LocationManagerCompat.getCurrentLocation(manager, provider, signal, executor) { location ->
                        if (active[id] !== operation) return@getCurrentLocation
                        if (location != null && location.hasAccuracy() && location.elapsedRealtimeNanos >= earliest) {
                            newest = location
                            finish(id, location, null)
                        } else if (location != null) {
                            // Some providers answer the one-shot API with a recent cached fix.
                            // Continue listening until a sample satisfies this caller's age limit.
                            val listener = object : LocationListenerCompat {
                                override fun onLocationChanged(fix: Location) {
                                    if (active[id] === operation && fix.hasAccuracy() && fix.elapsedRealtimeNanos >= earliest) {
                                        newest = fix
                                        finish(id, fix, null)
                                    }
                                }
                            }
                            try {
                                val configuration = LocationRequestCompat.Builder(0).build()
                                LocationManagerCompat.requestLocationUpdates(manager, provider, configuration, executor, listener)
                                operation.listeners.add(listener)
                            } catch (_: SecurityException) {
                                if (--operation.candidates <= 0) finish(id, null, "permission-denied")
                            }
                        } else if (--operation.candidates <= 0) finish(id, null, "position-unavailable")
                    }
                } catch (_: SecurityException) { operation.candidates-- }
            }
            if (operation.candidates == 0) finish(id, null, "permission-denied")
        }

        private fun finish(id: Long, location: Location?, error: String?) {
            val operation = active.remove(id) ?: return
            main.removeCallbacks(operation.timer)
            operation.signals.forEach { it.cancel() }
            operation.listeners.forEach(manager::removeUpdates)
            if (location != null) operation.request.resolveJson(json(location))
            else operation.request.fail(error ?: "position-unavailable", "Location request did not complete")
        }

        private fun watch(options: JSONObject) {
            if (options.optBoolean("allowBackground") && !background) throw IllegalArgumentException("Enable background location in app.toml")
            stopWatch()
            val accuracy = options.optString("accuracy", "high")
            val high = accuracy !in listOf("lowest", "low", "medium")
            val request = LocationRequestCompat.Builder(options.optLong("intervalMs", 2000))
                .setMinUpdateDistanceMeters(options.optDouble("distanceFilterMeters", 10.0).toFloat())
                .setQuality(if (high) LocationRequestCompat.QUALITY_HIGH_ACCURACY else LocationRequestCompat.QUALITY_BALANCED_POWER_ACCURACY).build()
            val callback = LocationListenerCompat { point ->
                val prior = newest
                if (point.hasAccuracy() && (prior == null || point.elapsedRealtimeNanos > prior.elapsedRealtimeNanos &&
                    (point.accuracy <= prior.accuracy * 2 || point.elapsedRealtimeNanos - prior.elapsedRealtimeNanos > 2_000_000_000L))) {
                    newest = point
                    host.emitPluginEvent(positions, json(point).toString())
                }
            }
            var registered = false
            for (provider in providers(high)) {
                try { LocationManagerCompat.requestLocationUpdates(manager, provider, request, executor, callback); registered = true }
                catch (_: SecurityException) { }
            }
            if (!registered) throw SecurityException()
            listener = callback
        }

        private fun watchService() {
            if (receiver == null) {
                val updates = object : BroadcastReceiver() {
                    override fun onReceive(context: Context, intent: Intent) { publishService() }
                }
                ContextCompat.registerReceiver(context, updates, IntentFilter(LocationManager.PROVIDERS_CHANGED_ACTION), ContextCompat.RECEIVER_EXPORTED)
                receiver = updates
            }
            publishService()
        }
        private fun publishService() {
            host.emitPluginEvent(service, if (LocationManagerCompat.isLocationEnabled(manager)) "\"enabled\"" else "\"disabled\"")
        }
        private fun stopWatch() { listener?.let(manager::removeUpdates); listener = null }
        private fun stopService() { receiver?.let(context::unregisterReceiver); receiver = null }

        fun close() {
            active.keys.toList().forEach { finish(it, null, "cancelled") }
            stopWatch()
            stopService()
        }
    }

    private fun json(value: Location): JSONObject = JSONObject()
        .put("latitude", value.latitude).put("longitude", value.longitude).put("accuracy", value.accuracy)
        .put("altitude", if (value.hasAltitude()) value.altitude else JSONObject.NULL)
        .put("altitudeAccuracy", if (LocationCompat.hasVerticalAccuracy(value)) LocationCompat.getVerticalAccuracyMeters(value) else JSONObject.NULL)
        .put("heading", if (value.hasBearing()) value.bearing else JSONObject.NULL)
        .put("headingAccuracy", if (LocationCompat.hasBearingAccuracy(value)) LocationCompat.getBearingAccuracyDegrees(value) else JSONObject.NULL)
        .put("speed", if (value.hasSpeed()) value.speed else JSONObject.NULL)
        .put("speedAccuracy", if (LocationCompat.hasSpeedAccuracy(value)) LocationCompat.getSpeedAccuracyMetersPerSecond(value) else JSONObject.NULL)
        .put("timestampMs", value.time).put("isMocked", LocationCompat.isMock(value))
}
