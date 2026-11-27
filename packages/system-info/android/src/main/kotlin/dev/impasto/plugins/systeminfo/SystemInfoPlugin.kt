package dev.impasto.plugins.systeminfo

import android.app.Activity
import android.app.ActivityManager
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.os.Build
import android.os.PowerManager
import android.os.StatFs
import dev.impasto.host.ImpastoHost
import org.json.JSONObject
import java.util.Locale
import java.util.TimeZone

object SystemInfoPlugin {
    private const val name = "@impasto/system-info"

    fun register(host: ImpastoHost) {
        val activity = host.activity
        host.registerNativePlugin(name) { request ->
            when (request.operation) {
                "$name/appInfo" -> request.resolveJson(app(activity))
                "$name/deviceInfo" -> request.resolveJson(device(activity))
                "$name/runtimeInfo" -> request.resolveJson(runtime(activity))
                else -> request.fail("Unknown system information operation")
            }
        }
    }

    @Suppress("DEPRECATION")
    private fun app(activity: Activity): JSONObject {
        val manager = activity.packageManager
        val name = activity.packageName
        val info = if (Build.VERSION.SDK_INT >= 33) manager.getPackageInfo(name, PackageManager.PackageInfoFlags.of(0))
            else manager.getPackageInfo(name, 0)
        val build = if (Build.VERSION.SDK_INT >= 28) info.longVersionCode else info.versionCode.toLong()
        val installer = try {
            if (Build.VERSION.SDK_INT >= 30) manager.getInstallSourceInfo(name).installingPackageName
            else manager.getInstallerPackageName(name)
        } catch (_: PackageManager.NameNotFoundException) { null }
        val store = if (emulator()) "simulator" else when (installer) {
            "com.android.vending" -> "googlePlay"
            "com.amazon.venezia" -> "amazon"
            "com.huawei.appmarket" -> "huawei"
            "com.xiaomi.market" -> "xiaomi"
            null, "com.android.packageinstaller", "com.google.android.packageinstaller" -> "sideload"
            else -> "unknown"
        }
        return JSONObject().put("appName", activity.applicationInfo.loadLabel(manager).toString())
            .put("packageName", name).put("version", info.versionName ?: "").put("buildNumber", build.toString())
            .put("installerStore", store).put("isDebug", activity.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0)
    }

    private fun emulator(): Boolean = Build.FINGERPRINT.startsWith("generic") ||
        Build.HARDWARE.contains("ranchu") || Build.HARDWARE.contains("goldfish") ||
        Build.PRODUCT.startsWith("sdk") || Build.MODEL.contains("sdk_gphone", ignoreCase = true)

    private fun device(activity: Activity): JSONObject {
        val memory = ActivityManager.MemoryInfo()
        activity.getSystemService(ActivityManager::class.java)?.getMemoryInfo(memory)
        return JSONObject().put("platform", "android").put("osVersion", Build.VERSION.RELEASE).put("osBuild", Build.DISPLAY)
            .put("sdkInt", Build.VERSION.SDK_INT).put("model", Build.MODEL).put("brand", Build.BRAND)
            .put("manufacturer", Build.MANUFACTURER).put("architecture", Build.SUPPORTED_ABIS.firstOrNull().orEmpty())
            .put("isPhysicalDevice", !emulator()).put("processorCount", Runtime.getRuntime().availableProcessors())
            .put("totalMemoryBytes", memory.totalMem).put("locale", Locale.getDefault().toLanguageTag())
            .put("timeZone", TimeZone.getDefault().id)
    }

    private fun runtime(activity: Activity): JSONObject {
        val disk = StatFs(activity.filesDir.absolutePath)
        return JSONObject().put("freeDiskBytes", disk.availableBytes).put("totalDiskBytes", disk.totalBytes)
            .put("isLowPowerMode", activity.getSystemService(PowerManager::class.java)?.isPowerSaveMode == true)
    }
}
