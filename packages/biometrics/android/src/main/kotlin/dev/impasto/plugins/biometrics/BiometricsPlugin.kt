package dev.impasto.plugins.biometrics

import android.app.KeyguardManager
import android.content.pm.PackageManager
import android.os.Build
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_WEAK
import androidx.biometric.BiometricManager.Authenticators.DEVICE_CREDENTIAL
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import dev.impasto.host.ImpastoHost
import dev.impasto.host.ImpastoNativeRequest
import org.json.JSONArray
import org.json.JSONObject

object BiometricsPlugin {
    private const val name = "@impasto/biometrics"

    fun register(host: ImpastoHost) {
        val activity = host.activity as? FragmentActivity
        val runner = activity?.let { Runner(it) }
        host.registerNativePlugin(name, detach = { runner?.cancel("system-cancelled"); null }) { request ->
            when {
                runner == null -> request.fail("host-unavailable", "BiometricPrompt requires a FragmentActivity")
                request.operation == "$name/capabilities" -> request.resolveJson(runner.capabilities())
                request.operation == "$name/authenticate" -> runner.authenticate(request)
                request.operation == "$name/cancel" -> {
                    if (runner.id == request.payload.toLongOrNull()) runner.cancel("cancelled")
                    request.resolve()
                }
                else -> request.fail("platform", "Unknown biometric operation")
            }
        }
    }

    private class Runner(private val activity: FragmentActivity) {
        private val manager = BiometricManager.from(activity)
        private val keyguard = activity.getSystemService(KeyguardManager::class.java)
        private var prompt: BiometricPrompt? = null
        private var pending: ImpastoNativeRequest? = null
        var id: Long? = null
            private set

        fun capabilities(): JSONObject {
            val strong = manager.canAuthenticate(BIOMETRIC_STRONG)
            val weak = manager.canAuthenticate(BIOMETRIC_WEAK)
            val enrolled = strong == BiometricManager.BIOMETRIC_SUCCESS || weak == BiometricManager.BIOMETRIC_SUCCESS
            val hardware = weak != BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE && weak != BiometricManager.BIOMETRIC_ERROR_UNSUPPORTED
            val credential = if (Build.VERSION.SDK_INT >= 30) manager.canAuthenticate(DEVICE_CREDENTIAL) == BiometricManager.BIOMETRIC_SUCCESS
                else keyguard?.isDeviceSecure == true
            val types = listOf(
                PackageManager.FEATURE_FINGERPRINT to "fingerprint", "android.hardware.biometrics.face" to "face",
                "android.hardware.biometrics.iris" to "iris",
            ).filter { activity.packageManager.hasSystemFeature(it.first) }.map { it.second }
            return JSONObject().put("isAvailable", credential || hardware).put("isEnrolled", enrolled)
                .put("hasDeviceCredential", credential).put("biometryTypes", JSONArray(types.ifEmpty { if (hardware) listOf("unknown") else emptyList() }))
                .put("securityLevel", when { strong == BiometricManager.BIOMETRIC_SUCCESS -> "strong"; enrolled -> "weak"; else -> "none" })
        }

        fun authenticate(request: ImpastoNativeRequest) {
            if (pending != null) { request.fail("busy", "Authentication is already in progress"); return }
            val config = JSONObject(request.payload)
            val credential = config.optBoolean("allowDeviceCredential")
            val authenticators = if (!credential) BIOMETRIC_STRONG
                else if (Build.VERSION.SDK_INT >= 30) BIOMETRIC_STRONG or DEVICE_CREDENTIAL
                else BIOMETRIC_WEAK or DEVICE_CREDENTIAL
            val current = config.getLong("id")
            pending = request
            id = current
            try {
                val info = BiometricPrompt.PromptInfo.Builder()
                    .setTitle(config.optional("title") ?: config.getString("reason"))
                    .setAllowedAuthenticators(authenticators)
                    .setConfirmationRequired(config.optBoolean("confirmationRequired", true))
                config.optional("subtitle")?.let(info::setSubtitle)
                config.optional("description")?.let(info::setDescription)
                if (!credential) info.setNegativeButtonText(config.optional("cancelLabel")?.ifBlank { "Cancel" } ?: "Cancel")
                val callback = object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        if (id == current) complete(null)
                    }
                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        if (id == current) complete(when (errorCode) {
                            BiometricPrompt.ERROR_USER_CANCELED, BiometricPrompt.ERROR_NEGATIVE_BUTTON -> "cancelled"
                            BiometricPrompt.ERROR_CANCELED -> "system-cancelled"
                            BiometricPrompt.ERROR_LOCKOUT -> "lockout"
                            BiometricPrompt.ERROR_LOCKOUT_PERMANENT -> "permanent-lockout"
                            BiometricPrompt.ERROR_NO_BIOMETRICS -> "not-enrolled"
                            BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL -> "passcode-not-set"
                            BiometricPrompt.ERROR_HW_NOT_PRESENT, BiometricPrompt.ERROR_HW_UNAVAILABLE -> "not-available"
                            else -> "failed"
                        })
                    }
                }
                prompt = BiometricPrompt(activity, ContextCompat.getMainExecutor(activity), callback).also { it.authenticate(info.build()) }
            } catch (_: IllegalArgumentException) {
                pending = null
                id = null
                prompt = null
                request.fail("invalid-argument", "Invalid biometric prompt configuration")
            } catch (_: IllegalStateException) {
                cancel("system-cancelled")
            }
        }

        private fun JSONObject.optional(key: String): String? = if (isNull(key)) null else getString(key)

        private fun complete(reason: String?) {
            val request = pending ?: return
            pending = null
            prompt = null
            id = null
            val outcome = JSONObject().put("authenticated", reason == null)
            if (reason != null) outcome.put("reason", reason).put("message", "Authentication did not complete")
            request.resolveJson(outcome)
        }

        fun cancel(reason: String) {
            val closing = prompt
            complete(reason)
            closing?.cancelAuthentication()
        }
    }
}
