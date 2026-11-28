package dev.impasto.plugins.haptics

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import dev.impasto.host.ImpastoHost
import org.json.JSONObject

object HapticsPlugin {
    private const val name = "@impasto/haptics"

    @Suppress("DEPRECATION")
    fun register(host: ImpastoHost) {
        val activity = host.activity
        val permission = activity.checkSelfPermission(Manifest.permission.VIBRATE) == PackageManager.PERMISSION_GRANTED
        val vibrator = when {
            !permission -> null
            Build.VERSION.SDK_INT >= 31 -> activity.getSystemService(VibratorManager::class.java)?.defaultVibrator
            else -> activity.getSystemService(Vibrator::class.java)
        }
        host.registerNativePlugin(name, detach = { vibrator?.cancel(); null }) { request ->
            val action = request.operation.substringAfterLast('/')
            val supported = vibrator?.hasVibrator() == true
            if (action == "isSupported") {
                request.resolveJson(supported)
                return@registerNativePlugin
            }
            if (!supported) {
                request.resolve()
                return@registerNativePlugin
            }
            when (action) {
                "impact" -> {
                    val style = request.payload.trim('"')
                    when (style) {
                        "light", "soft" -> feedback(vibrator, VibrationEffect.EFFECT_TICK, 15, 64)
                        "heavy", "rigid" -> feedback(vibrator, VibrationEffect.EFFECT_HEAVY_CLICK, 35, 220)
                        else -> feedback(vibrator, VibrationEffect.EFFECT_CLICK, 25, 135)
                    }
                }
                "selection" -> feedback(vibrator, VibrationEffect.EFFECT_TICK, 10, 65)
                "notification" -> {
                    val kind = request.payload.trim('"')
                    val effect = when (kind) {
                        "error" -> VibrationEffect.EFFECT_HEAVY_CLICK
                        "warning" -> VibrationEffect.EFFECT_DOUBLE_CLICK
                        else -> VibrationEffect.EFFECT_CLICK
                    }
                    feedback(vibrator, effect, 30, 170)
                }
                "vibrate" -> oneShot(vibrator, request.payload.toLong(), VibrationEffect.DEFAULT_AMPLITUDE)
                "vibratePattern" -> {
                    val options = JSONObject(request.payload)
                    val timings = options.getJSONArray("timingsMs")
                    val times = LongArray(timings.length()) { timings.getLong(it) }
                    val amplitudes = options.optJSONArray("amplitudes")
                    if (Build.VERSION.SDK_INT >= 26) {
                        val effect = if (amplitudes != null) VibrationEffect.createWaveform(times,
                            IntArray(amplitudes.length()) { amplitudes.getInt(it) }, -1)
                        else if (times.size == 1) VibrationEffect.createOneShot(times[0], VibrationEffect.DEFAULT_AMPLITUDE)
                        else VibrationEffect.createWaveform(times, -1)
                        vibrator.vibrate(effect)
                    } else if (times.size == 1) vibrator.vibrate(times[0]) else vibrator.vibrate(times, -1)
                }
                "cancel" -> vibrator.cancel()
                else -> { request.fail("Unknown haptics operation"); return@registerNativePlugin }
            }
            request.resolve()
        }
    }

    private fun feedback(vibrator: Vibrator, effect: Int, ms: Long, amplitude: Int) {
        if (Build.VERSION.SDK_INT >= 29) vibrator.vibrate(VibrationEffect.createPredefined(effect))
        else oneShot(vibrator, ms, amplitude)
    }

    @Suppress("DEPRECATION")
    private fun oneShot(vibrator: Vibrator, ms: Long, amplitude: Int) {
        if (Build.VERSION.SDK_INT >= 26) vibrator.vibrate(VibrationEffect.createOneShot(ms, amplitude)) else vibrator.vibrate(ms)
    }
}
