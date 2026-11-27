package dev.impasto.plugins.permissions

import android.Manifest.permission as AndroidPermission
import android.app.Activity
import android.app.AlarmManager
import android.app.NotificationManager
import android.bluetooth.BluetoothManager
import android.content.Intent
import android.content.pm.PackageManager
import android.location.LocationManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.PowerManager
import android.provider.Settings
import androidx.core.app.ActivityCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.location.LocationManagerCompat

internal class PermissionFailure(val code: String, message: String) : RuntimeException(message)

internal class PermissionCapabilities(private val activity: Activity, private val enabled: Set<String>?) {
    private val sdk = Build.VERSION.SDK_INT
    private val packageUri = Uri.fromParts("package", activity.packageName, null)
    private val notifications = NotificationManagerCompat.from(activity)

    @Suppress("DEPRECATION")
    private val declared = activity.packageManager
        .getPackageInfo(activity.packageName, PackageManager.GET_PERMISSIONS)
        .requestedPermissions.orEmpty().toSet()
    private val history = activity.getSharedPreferences("dev.impasto.permissions", Activity.MODE_PRIVATE)

    fun validateEnabled(name: String) {
        if (enabled != null && isSupported(name) && name !in enabled) {
            throw PermissionFailure("not-enabled", "Enable $name in [plugins.\"@impasto/permissions\"].enabled")
        }
    }

    fun validateRequest(name: String) {
        validateEnabled(name)
        val missing = (runtime(name) + foreground(name) + listOfNotNull(specialPermission(name)))
            .distinct().filterNot(declared::contains)
        if (missing.isNotEmpty()) {
            throw PermissionFailure("missing-config", "Missing Android manifest permissions for $name: ${missing.joinToString()}")
        }
    }

    private fun isSupported(name: String): Boolean = name in supported

    private fun media(permission: String) = when {
        sdk >= 34 && permission != AndroidPermission.READ_MEDIA_AUDIO ->
            listOf(permission, AndroidPermission.READ_MEDIA_VISUAL_USER_SELECTED)
        sdk >= 33 -> listOf(permission)
        else -> listOf(AndroidPermission.READ_EXTERNAL_STORAGE)
    }

    fun runtime(name: String): List<String> = when (name) {
        "camera" -> listOf(AndroidPermission.CAMERA)
        "microphone", "speech" -> listOf(AndroidPermission.RECORD_AUDIO)
        "location", "locationWhenInUse" -> locationPermissions
        "locationAlways" -> if (sdk >= 29) listOf(AndroidPermission.ACCESS_BACKGROUND_LOCATION) else locationPermissions
        "contacts" -> listOf(AndroidPermission.READ_CONTACTS)
        "calendarWriteOnly" -> listOf(AndroidPermission.WRITE_CALENDAR)
        "calendarFullAccess" -> listOf(AndroidPermission.READ_CALENDAR, AndroidPermission.WRITE_CALENDAR)
        "storage" -> when {
            sdk <= 28 -> listOf(AndroidPermission.READ_EXTERNAL_STORAGE, AndroidPermission.WRITE_EXTERNAL_STORAGE)
            sdk <= 32 -> listOf(AndroidPermission.READ_EXTERNAL_STORAGE)
            else -> emptyList()
        }
        "manageExternalStorage" -> if (sdk <= 29) runtime("storage") else emptyList()
        "photos" -> media(AndroidPermission.READ_MEDIA_IMAGES)
        "videos" -> media(AndroidPermission.READ_MEDIA_VIDEO)
        "audio", "mediaLibrary" -> media(AndroidPermission.READ_MEDIA_AUDIO)
        "photosAddOnly" -> if (sdk <= 28) listOf(AndroidPermission.WRITE_EXTERNAL_STORAGE) else emptyList()
        "accessMediaLocation" -> if (sdk >= 29) listOf(AndroidPermission.ACCESS_MEDIA_LOCATION) else emptyList()
        "notification" -> if (sdk >= 33) listOf(AndroidPermission.POST_NOTIFICATIONS) else emptyList()
        "bluetoothConnect" -> if (sdk >= 31) listOf(AndroidPermission.BLUETOOTH_CONNECT) else listOf(AndroidPermission.BLUETOOTH)
        "bluetoothScan" -> if (sdk >= 31) listOf(AndroidPermission.BLUETOOTH_SCAN) else listOf(AndroidPermission.BLUETOOTH) + locationPermissions
        "bluetoothAdvertise" -> if (sdk >= 31) listOf(AndroidPermission.BLUETOOTH_ADVERTISE) else listOf(AndroidPermission.BLUETOOTH)
        "bluetooth" -> runtime("bluetoothConnect") + runtime("bluetoothScan")
        "nearbyWifiDevices" -> if (sdk >= 33) listOf(AndroidPermission.NEARBY_WIFI_DEVICES) else locationPermissions
        "sensors" -> listOf(AndroidPermission.BODY_SENSORS)
        "sensorsAlways" -> if (sdk >= 33) listOf(AndroidPermission.BODY_SENSORS_BACKGROUND) else runtime("sensors")
        "activityRecognition" -> if (sdk >= 29) listOf(AndroidPermission.ACTIVITY_RECOGNITION) else emptyList()
        "phone" -> listOf(AndroidPermission.READ_PHONE_STATE)
        "sms" -> listOf(AndroidPermission.SEND_SMS, AndroidPermission.READ_SMS)
        else -> emptyList()
    }

    fun foreground(name: String): List<String> = when {
        name == "locationAlways" && sdk >= 29 -> locationPermissions
        name == "sensorsAlways" && sdk >= 33 -> runtime("sensors")
        else -> emptyList()
    }

    fun hasForeground(name: String): Boolean = when (name) {
        "locationAlways" -> locationPermissions.any(::granted)
        "sensorsAlways" -> granted(AndroidPermission.BODY_SENSORS)
        else -> true
    }

    fun requiresFollowUp(name: String): Boolean = foreground(name).isNotEmpty()

    fun granted(permission: String): Boolean {
        val granted = activity.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED
        if (granted && history.getBoolean(permission, false)) history.edit().remove(permission).apply()
        return granted
    }

    private fun rationaleFor(permission: String) =
        ActivityCompat.shouldShowRequestPermissionRationale(activity, permission)

    fun rationale(name: String): Boolean {
        validateEnabled(name)
        return (runtime(name) + foreground(name)).any(::rationaleFor)
    }

    fun recordResult(result: Map<String, Boolean>) {
        if (result.isEmpty()) return
        val edit = history.edit()
        for ((permission, granted) in result) {
            if (granted) edit.remove(permission)
            else if (rationaleFor(permission)) edit.putBoolean(permission, true)
        }
        edit.apply()
    }

    fun promptPermissions(permissions: List<String>): Array<String> {
        val missing = permissions.distinct().filter { !granted(it) }.toMutableList()
        // Android 12+ only allows a user to choose precision when both are requested together.
        if (sdk >= 31 && AndroidPermission.ACCESS_FINE_LOCATION in missing &&
            AndroidPermission.ACCESS_COARSE_LOCATION in permissions &&
            AndroidPermission.ACCESS_COARSE_LOCATION !in missing
        ) missing.add(AndroidPermission.ACCESS_COARSE_LOCATION)
        if (sdk >= 34 && AndroidPermission.READ_MEDIA_VISUAL_USER_SELECTED in permissions &&
            AndroidPermission.READ_MEDIA_VISUAL_USER_SELECTED !in missing &&
            missing.any { it == AndroidPermission.READ_MEDIA_IMAGES || it == AndroidPermission.READ_MEDIA_VIDEO }
        ) missing.add(AndroidPermission.READ_MEDIA_VISUAL_USER_SELECTED)
        return missing.toTypedArray()
    }

    fun status(name: String): String {
        validateEnabled(name)
        if (!isSupported(name)) return "restricted"
        specialStatus(name)?.let { return if (it) "granted" else "denied" }
        if (name == "notification" && sdk < 33) {
            return if (notifications.areNotificationsEnabled()) "granted" else "denied"
        }
        if (name == "locationAlways" && sdk >= 29 && !hasForeground(name)) return "denied"
        if (name == "sensorsAlways" && sdk >= 33 && !hasForeground(name)) return "denied"
        val mapped = runtime(name)
        if ((name == "photos" || name == "videos") && sdk >= 34) {
            if (granted(mapped.first())) return "granted"
            if (granted(AndroidPermission.READ_MEDIA_VISUAL_USER_SELECTED)) return "limited"
        } else if (mapped.all(::granted)) {
            return "granted"
        }
        val foregroundLocation = name == "location" || name == "locationWhenInUse" || (name == "locationAlways" && sdk < 29)
        if (foregroundLocation && granted(AndroidPermission.ACCESS_COARSE_LOCATION)) {
            return "limited"
        }
        val missing = mapped.filter { it != AndroidPermission.READ_MEDIA_VISUAL_USER_SELECTED && !granted(it) }
        return if (missing.isNotEmpty() && missing.all { it in declared && history.getBoolean(it, false) && !rationaleFor(it) }) {
            "permanentlyDenied"
        } else {
            "denied"
        }
    }

    private fun specialPermission(name: String): String? = when {
        name == "manageExternalStorage" && sdk >= 30 -> AndroidPermission.MANAGE_EXTERNAL_STORAGE
        name == "systemAlertWindow" -> AndroidPermission.SYSTEM_ALERT_WINDOW
        name == "requestInstallPackages" && sdk >= 26 -> AndroidPermission.REQUEST_INSTALL_PACKAGES
        name == "ignoreBatteryOptimizations" -> AndroidPermission.REQUEST_IGNORE_BATTERY_OPTIMIZATIONS
        name == "scheduleExactAlarm" && sdk >= 31 -> AndroidPermission.SCHEDULE_EXACT_ALARM
        name == "accessNotificationPolicy" || name == "criticalAlerts" -> AndroidPermission.ACCESS_NOTIFICATION_POLICY
        else -> null
    }

    private fun specialStatus(name: String): Boolean? = when (name) {
        "manageExternalStorage" -> if (sdk >= 30) Environment.isExternalStorageManager() else null
        "systemAlertWindow" -> Settings.canDrawOverlays(activity)
        "requestInstallPackages" -> sdk < 26 || activity.packageManager.canRequestPackageInstalls()
        "ignoreBatteryOptimizations", "backgroundRefresh" ->
            activity.getSystemService(PowerManager::class.java)?.isIgnoringBatteryOptimizations(activity.packageName) == true
        "scheduleExactAlarm" -> sdk < 31 || activity.getSystemService(AlarmManager::class.java)?.canScheduleExactAlarms() == true
        "accessNotificationPolicy", "criticalAlerts" ->
            activity.getSystemService(NotificationManager::class.java)?.isNotificationPolicyAccessGranted == true
        else -> null
    }

    fun service(name: String): String {
        validateEnabled(name)
        val enabled = when (name) {
            "location", "locationWhenInUse", "locationAlways" ->
                activity.getSystemService(LocationManager::class.java)?.let(LocationManagerCompat::isLocationEnabled)
            "notification" -> notifications.areNotificationsEnabled()
            "bluetooth", "bluetoothConnect", "bluetoothScan", "bluetoothAdvertise" -> try {
                activity.getSystemService(BluetoothManager::class.java)?.adapter?.isEnabled
            } catch (_: SecurityException) {
                null
            }
            "phone" -> activity.packageManager.hasSystemFeature(PackageManager.FEATURE_TELEPHONY)
            else -> null
        }
        return when (enabled) {
            true -> "enabled"
            false -> "disabled"
            null -> "notApplicable"
        }
    }

    fun requestSettings(name: String): Intent? = when (name) {
        "locationAlways" -> if (sdk >= 30) appSettings() else null
        "manageExternalStorage" -> if (sdk >= 30) Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION, packageUri) else null
        "systemAlertWindow" -> Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, packageUri)
        "requestInstallPackages" -> if (sdk >= 26) Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, packageUri) else null
        "ignoreBatteryOptimizations" -> Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, packageUri)
        "scheduleExactAlarm" -> if (sdk >= 31) Intent(Settings.ACTION_REQUEST_SCHEDULE_EXACT_ALARM, packageUri) else null
        "accessNotificationPolicy", "criticalAlerts" -> Intent(Settings.ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS)
        else -> null
    }

    fun openSettings(name: String): Boolean {
        val intent = when (name) {
            "notification" -> if (sdk >= 26) Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                .putExtra(Settings.EXTRA_APP_PACKAGE, activity.packageName) else null
            "backgroundRefresh" -> Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)
            "ignoreBatteryOptimizations" -> Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)
            else -> requestSettings(name)
        }
        return (intent != null && open(intent)) || open(appSettings())
    }

    private fun appSettings() = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, packageUri)

    private fun open(intent: Intent): Boolean = try {
        activity.startActivity(intent)
        true
    } catch (_: RuntimeException) {
        false
    }

    companion object {
        private val locationPermissions = listOf(AndroidPermission.ACCESS_COARSE_LOCATION, AndroidPermission.ACCESS_FINE_LOCATION)
        private val supported = setOf(
            "camera", "microphone", "speech", "location", "locationWhenInUse", "locationAlways", "contacts",
            "calendarWriteOnly", "calendarFullAccess", "storage", "manageExternalStorage", "accessMediaLocation",
            "photos", "photosAddOnly", "videos", "audio", "mediaLibrary", "notification", "criticalAlerts",
            "bluetooth", "bluetoothConnect", "bluetoothScan", "bluetoothAdvertise", "nearbyWifiDevices", "sensors",
            "sensorsAlways", "activityRecognition", "phone", "sms", "scheduleExactAlarm", "accessNotificationPolicy",
            "systemAlertWindow", "requestInstallPackages", "ignoreBatteryOptimizations",
            "appTrackingTransparency", "backgroundRefresh",
        )
    }
}
