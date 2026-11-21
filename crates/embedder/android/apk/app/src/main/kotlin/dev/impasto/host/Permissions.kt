package dev.impasto.host

import android.Manifest.permission as AndroidPermission
import android.app.Activity
import android.app.NotificationManager
import android.bluetooth.BluetoothManager
import android.content.Intent
import android.content.pm.PackageManager
import android.location.LocationManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.activity.result.ActivityResultRegistry
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.app.ActivityCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.location.LocationManagerCompat
import java.util.UUID

/** Binds session permission requests to the Activity Result registry for this window. */
internal class Permissions(
    private val activity: Activity,
    registry: ActivityResultRegistry?,
    pending: Pending?,
    private val complete: (Long, Array<String>?, String?) -> Unit,
) : AutoCloseable {
    internal data class Pending(val key: String, val id: Long, val names: Array<String>)

    @Suppress("DEPRECATION")
    private val declared = activity.packageManager
        .getPackageInfo(activity.packageName, PackageManager.GET_PERMISSIONS)
        .requestedPermissions.orEmpty().toSet()
    private val history = activity.getSharedPreferences("dev.impasto.permissions", Activity.MODE_PRIVATE)
    private val key = pending?.key ?: "impasto.permissions.${UUID.randomUUID()}"
    private var active = pending
    val hasPendingRequest: Boolean get() = active != null
    private val launcher = registry?.register(
        key,
        ActivityResultContracts.RequestMultiplePermissions(),
        ::receive,
    )

    init {
        if (launcher == null && pending != null) {
            active = null
            complete(pending.id, null, "replacement host does not provide Activity Results")
        }
    }

    private fun permissions(name: String): List<String> = when (name) {
        "camera" -> listOf(AndroidPermission.CAMERA)
        "microphone", "speech" -> listOf(AndroidPermission.RECORD_AUDIO)
        "location", "locationWhenInUse" -> listOf(
            AndroidPermission.ACCESS_COARSE_LOCATION, AndroidPermission.ACCESS_FINE_LOCATION,
        )
        "locationAlways" -> if (Build.VERSION.SDK_INT >= 29) {
            listOf(AndroidPermission.ACCESS_BACKGROUND_LOCATION)
        } else permissions("location")
        "contacts" -> listOf(AndroidPermission.READ_CONTACTS)
        "calendar" -> listOf(AndroidPermission.READ_CALENDAR)
        "storage" -> if (Build.VERSION.SDK_INT < 33) listOf(AndroidPermission.READ_EXTERNAL_STORAGE) else emptyList()
        "photos" -> when {
            Build.VERSION.SDK_INT >= 34 -> listOf(
                AndroidPermission.READ_MEDIA_IMAGES, AndroidPermission.READ_MEDIA_VISUAL_USER_SELECTED,
            )
            Build.VERSION.SDK_INT >= 33 -> listOf(AndroidPermission.READ_MEDIA_IMAGES)
            else -> listOf(AndroidPermission.READ_EXTERNAL_STORAGE)
        }
        "notification" -> if (Build.VERSION.SDK_INT >= 33) {
            listOf(AndroidPermission.POST_NOTIFICATIONS)
        } else emptyList()
        "bluetoothConnect" -> if (Build.VERSION.SDK_INT >= 31) {
            listOf(AndroidPermission.BLUETOOTH_CONNECT)
        } else listOf(AndroidPermission.BLUETOOTH)
        "bluetoothScan" -> if (Build.VERSION.SDK_INT >= 31) {
            listOf(AndroidPermission.BLUETOOTH_SCAN)
        } else listOf(AndroidPermission.ACCESS_FINE_LOCATION)
        "bluetooth" -> if (Build.VERSION.SDK_INT >= 31) {
            listOf(AndroidPermission.BLUETOOTH_CONNECT, AndroidPermission.BLUETOOTH_SCAN)
        } else listOf(AndroidPermission.BLUETOOTH)
        else -> emptyList()
    }

    private fun granted(permission: String) =
        activity.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    private fun rationaleFor(permission: String) =
        ActivityCompat.shouldShowRequestPermissionRationale(activity, permission)

    fun rationale(name: String): Boolean = permissions(name).any(::rationaleFor)

    fun status(name: String): String {
        if (name == "notification" && Build.VERSION.SDK_INT < 33) {
            return if (NotificationManagerCompat.from(activity).areNotificationsEnabled()) "granted" else "denied"
        }
        val mapped = permissions(name)
        if (mapped.isEmpty()) return "denied"
        if (name == "photos" && Build.VERSION.SDK_INT >= 34) {
            if (granted(AndroidPermission.READ_MEDIA_IMAGES)) return "granted"
            if (granted(AndroidPermission.READ_MEDIA_VISUAL_USER_SELECTED)) return "limited"
        } else if (mapped.all(::granted)) {
            return "granted"
        }
        if ((name == "location" || name == "locationWhenInUse") &&
            granted(AndroidPermission.ACCESS_COARSE_LOCATION)
        ) return "limited"
        val missing = mapped.filter { it in declared && !granted(it) }
        if (missing.isEmpty()) return "denied"
        return if (missing.all { history.getBoolean(it, false) && !rationaleFor(it) }) {
            "permanentlyDenied"
        } else {
            "denied"
        }
    }

    fun service(name: String): String = when (name) {
        "location", "locationWhenInUse", "locationAlways" -> {
            val manager = activity.getSystemService(LocationManager::class.java)
            if (manager != null && LocationManagerCompat.isLocationEnabled(manager)) "enabled" else "disabled"
        }
        "notification" -> {
            val manager = activity.getSystemService(NotificationManager::class.java)
            if (manager?.areNotificationsEnabled() == true) "enabled" else "disabled"
        }
        "bluetooth", "bluetoothConnect", "bluetoothScan" -> {
            val enabled = try {
                activity.getSystemService(BluetoothManager::class.java)?.adapter?.isEnabled
            } catch (_: SecurityException) {
                null
            }
            when (enabled) {
                true -> "enabled"
                false -> "disabled"
                null -> "notApplicable"
            }
        }
        else -> "notApplicable"
    }

    fun openSettings(): Boolean = try {
        activity.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", activity.packageName, null)))
        true
    } catch (_: RuntimeException) {
        false
    }

    fun request(id: Long, names: Array<String>) {
        if (active != null) {
            complete(id, null, "a permission request is already open")
            return
        }
        val requested = names.flatMap(::permissions).distinct().filter { it in declared }.toSet()
        // On Android 11+ background location is changed through app settings.
        // On Android 10 ask for foreground access first in a separate request.
        val candidates = requested.filter {
            it != AndroidPermission.ACCESS_BACKGROUND_LOCATION ||
                (Build.VERSION.SDK_INT == 29 && (
                    granted(AndroidPermission.ACCESS_COARSE_LOCATION) ||
                        granted(AndroidPermission.ACCESS_FINE_LOCATION)
                    ))
        }
        // Keep fine and coarse together for approximate location on Android 12+.
        val missing = candidates.filter { !granted(it) }.toMutableList()
        if (Build.VERSION.SDK_INT >= 31 && AndroidPermission.ACCESS_FINE_LOCATION in missing &&
            AndroidPermission.ACCESS_COARSE_LOCATION in declared && AndroidPermission.ACCESS_COARSE_LOCATION !in missing
        ) missing.add(AndroidPermission.ACCESS_COARSE_LOCATION)
        if (missing.isEmpty()) {
            complete(id, names.map(::status).toTypedArray(), null)
            return
        }
        val resultLauncher = launcher
        if (resultLauncher == null) {
            complete(id, null, "host does not provide Activity Results")
            return
        }
        active = Pending(key, id, names)
        try {
            resultLauncher.launch(missing.toTypedArray())
        } catch (error: RuntimeException) {
            active = null
            complete(id, null, "could not request permissions (${error.javaClass.simpleName})")
        }
    }

    private fun receive(result: Map<String, Boolean>) {
        val request = active ?: return
        active = null
        if (result.isNotEmpty()) {
            val edit = history.edit()
            for ((permission, granted) in result) edit.putBoolean(permission, !granted)
            edit.apply()
        }
        complete(request.id, request.names.map(::status).toTypedArray(), null)
    }

    fun unbind(): Pending? {
        val request = active
        active = null
        launcher?.unregister()
        return request
    }

    override fun close() {
        val request = unbind()
        if (request != null) complete(request.id, null, "permission request cancelled with host")
    }
}
