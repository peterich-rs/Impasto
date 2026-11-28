package dev.impasto.plugins.notifications

import android.annotation.SuppressLint
import android.app.AlarmManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.AtomicFile
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.RemoteInput
import dev.impasto.host.ImpastoHost
import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener
import java.io.File
import java.io.IOException
import java.security.MessageDigest
import java.util.Calendar
import java.util.TimeZone
import java.util.UUID
import java.util.concurrent.Executors

private const val name = "@impasto/notifications"
private const val channelId = "impasto_default"
private const val noticeId = "impasto.notification.id"
private const val actionId = "impasto.notification.actionId"
private const val dataJson = "impasto.notification.dataJson"
private const val userText = "impasto.notification.userText"
private const val responseToken = "impasto.notification.responseToken"
private val background = Executors.newSingleThreadExecutor()
private val main = Handler(Looper.getMainLooper())

object NotificationsPlugin {
    fun register(host: ImpastoHost, enabled: Set<String> = emptySet()) {
        val app = host.activity.applicationContext
        val state = WindowNotifications(host)
        state.initial = response(app, host.activity.intent)
        if (state.initial != null) clearClick(host.activity.intent)
        val listener = host.addNewIntentListener { intent ->
            val response = response(app, intent) ?: return@addNewIntentListener false
            clearClick(intent)
            state.receive(response)
            true
        }
        Notices.windows.add(state)
        Notices.channels(app)
        host.registerNativePlugin(name, detach = {
            listener.close()
            Notices.windows.remove(state)
            null
        }) { request ->
            if (request.operation == "@impasto/events/start" || request.operation == "@impasto/events/stop") {
                val topic = JSONObject(request.payload).getString("topic").substringAfterLast('/')
                if (request.operation.endsWith("/start")) {
                    state.topics.add(topic)
                    if (topic == "response") { state.pending.forEach { state.emit(topic, it) }; state.pending.clear() }
                } else state.topics.remove(topic)
                request.resolve()
                return@registerNativePlugin
            }
            val action = request.operation.substringAfterLast('/')
            if (action == "getInitialResponse") {
                request.resolveJson(state.initial)
                state.initial = null
                return@registerNativePlugin
            }
            val foregroundNow = host.activity.hasWindowFocus()
            background.execute {
                try {
                    val value = JSONTokener(request.payload).nextValue()
                    val result = Notices.invoke(app, action, value, "exactAlarm" in enabled, foregroundNow)
                    main.post { request.resolveJson(result) }
                } catch (error: ExactAlarmDenied) {
                    main.post { request.fail("exact-alarm-denied", "Exact alarm authorization is required") }
                } catch (error: SecurityException) {
                    main.post { request.fail("permission-denied", "Notification authorization is required") }
                } catch (error: IllegalArgumentException) {
                    main.post { request.fail("invalid-argument", "Invalid notification options") }
                } catch (error: Exception) {
                    main.post { request.fail("platform", "The notification operation failed") }
                }
            }
        }
    }

    private fun clearClick(intent: Intent?) {
        listOf(noticeId, actionId, dataJson, userText, responseToken).forEach { intent?.removeExtra(it) }
    }
}

private class WindowNotifications(private val host: ImpastoHost) {
    var initial: JSONObject? = null
    val topics = mutableSetOf<String>()
    val pending = ArrayDeque<JSONObject>()
    val active: Boolean get() = host.activity.hasWindowFocus()
    fun receive(value: JSONObject) {
        if ("response" in topics) emit("response", value)
        else { if (pending.size == 16) pending.removeFirst(); pending.addLast(value) }
    }
    fun emit(event: String, value: JSONObject) {
        if (event in topics) host.emitPluginEvent("$name/$event", JSONObject.quote(value.toString()))
    }
}

class AlarmReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val id = intent.getStringExtra(noticeId) ?: return
        val result = goAsync()
        background.execute {
            try { Notices.fire(context.applicationContext, id) }
            finally { result.finish() }
        }
    }
}
class ActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val value = response(context, intent) ?: return
        val id = value.getString("notificationId")
        NotificationManagerCompat.from(context).cancel(id, stableId(id))
        Notices.windows.toList().forEach { it.receive(value) }
    }
}
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action !in listOf(Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_MY_PACKAGE_REPLACED, Intent.ACTION_TIMEZONE_CHANGED)) return
        val result = goAsync()
        background.execute {
            try { Notices.restore(context.applicationContext, intent.action == Intent.ACTION_TIMEZONE_CHANGED) }
            finally { result.finish() }
        }
    }
}

private class ExactAlarmDenied : IOException()

private object Notices {
    val windows = mutableSetOf<WindowNotifications>()
    private fun preferences(app: Context) = app.getSharedPreferences("impasto.notifications", Context.MODE_PRIVATE)
    @Synchronized
    fun responseToken(app: Context): String {
        val preferences = preferences(app)
        return preferences.getString("response_token", null) ?: UUID.randomUUID().toString().also {
            preferences.edit().putString("response_token", it).commit()
        }
    }

    private fun alarms(app: Context) = checkNotNull(app.getSystemService(AlarmManager::class.java))
    private fun manager(app: Context) = checkNotNull(app.getSystemService(NotificationManager::class.java))

    fun channels(app: Context) {
        if (Build.VERSION.SDK_INT >= 26) manager(app).createNotificationChannel(
            NotificationChannel(channelId, "General", NotificationManager.IMPORTANCE_DEFAULT),
        )
    }

    fun invoke(app: Context, action: String, value: Any?, exactFeature: Boolean, foregroundNow: Boolean): Any? {
        when (action) {
            "submit" -> {
                val data = value as JSONObject
                val request = data.getJSONObject("request")
                val trigger = data.optJSONObject("trigger")
                if (trigger == null) display(app, request, foregroundNow)
                else schedule(app, request, trigger, exactFeature)
            }
            "cancel" -> cancel(app, value as String)
            "cancelAll" -> {
                val scheduled = read(app)
                scheduled.keys().asSequence().toList().forEach { clearAlarm(app, it); removeMedia(app, it) }
                write(app, JSONObject())
            }
            "getPending" -> {
                val scheduled = read(app)
                return JSONArray(scheduled.keys().asSequence().map { id ->
                    val item = scheduled.getJSONObject(id)
                    val request = item.getJSONObject("request")
                    JSONObject().put("id", id).put("title", request.getString("title"))
                        .put("body", request.getString("body")).put("data", request.optJSONObject("data") ?: JSONObject())
                        .put("trigger", item.getJSONObject("trigger"))
                }.toList())
            }
            "dismissDelivered" -> if (value is String) manager(app).cancel(value, stableId(value)) else manager(app).cancelAll()
            "createChannel" -> createChannel(app, value as JSONObject)
            "deleteChannel" -> if (Build.VERSION.SDK_INT >= 26) manager(app).deleteNotificationChannel(value as String)
            "listChannels" -> return listChannels(app)
            "registerCategories" -> preferences(app).edit().putString("categories", (value as JSONArray).toString()).apply()
            "getBadgeCount" -> return preferences(app).getInt("badge", 0)
            "setBadgeCount" -> {
                val count = (value as Number).toInt()
                require(count >= 0)
                preferences(app).edit().putInt("badge", count).apply()
            }
            "setForegroundPresentation" -> preferences(app).edit().putString("policy", (value as JSONObject).toString()).apply()
            else -> throw IllegalArgumentException("Unknown notification operation")
        }
        return null
    }

    private fun read(app: Context): JSONObject {
        val file = AtomicFile(File(app.noBackupFilesDir, "impasto_notifications.json"))
        return try { JSONObject(file.readFully().decodeToString()) }
        catch (_: java.io.FileNotFoundException) { JSONObject() }
    }
    private fun write(app: Context, value: JSONObject) {
        val file = AtomicFile(File(app.noBackupFilesDir, "impasto_notifications.json"))
        val out = file.startWrite()
        try { out.write(value.toString().toByteArray()); file.finishWrite(out) }
        catch (error: IOException) { file.failWrite(out); throw error }
    }

    private fun schedule(app: Context, request: JSONObject, trigger: JSONObject, exactFeature: Boolean) {
        val id = request.getString("id")
        val now = System.currentTimeMillis()
        val instant = next(trigger, now) ?: throw IllegalArgumentException("No future notification time")
        val exact = trigger.optBoolean("exact")
        if (exact && Build.VERSION.SDK_INT >= 31 && (!exactFeature || !alarms(app).canScheduleExactAlarms())) throw ExactAlarmDenied()
        val set = read(app)
        val existing = set.optJSONObject(id)
        val copy = copyMedia(app, id, request)
        try {
            set.put(id, JSONObject().put("request", copy).put("trigger", trigger).put("at", instant))
            write(app, set)
            try { arm(app, id, instant, exact) }
            catch (error: Exception) {
                if (existing == null) set.remove(id) else set.put(id, existing)
                write(app, set)
                throw error
            }
            reapMedia(app, id, copy)
        } catch (error: Exception) {
            reapMedia(app, id, existing?.optJSONObject("request"))
            throw error
        }
    }

    fun fire(app: Context, id: String) {
        val set = read(app)
        val item = set.optJSONObject(id) ?: return
        val now = System.currentTimeMillis()
        val scheduled = item.getLong("at")
        if (scheduled > now + 1_000) return
        val request = item.getJSONObject("request")
        val trigger = item.getJSONObject("trigger")
        try { display(app, request, foregroundNow = false) } catch (_: SecurityException) {}
        val next = if (trigger.optBoolean("repeats")) next(trigger, maxOf(scheduled, now)) else null
        if (next == null) {
            set.remove(id)
            write(app, set)
            removeMedia(app, id)
        } else {
            item.put("at", next)
            write(app, set)
            arm(app, id, next, false)
        }
    }

    fun restore(app: Context, timeZoneChanged: Boolean) {
        val set = read(app)
        val now = System.currentTimeMillis()
        for (id in set.keys().asSequence().toList()) {
            val item = set.getJSONObject(id)
            val trigger = item.getJSONObject("trigger")
            var at: Long? = item.getLong("at")
            if (timeZoneChanged && trigger.getString("kind") == "calendar" || at!! <= now && trigger.optBoolean("repeats")) at = next(trigger, now)
            if (at == null || at <= now) { set.remove(id); removeMedia(app, id); continue }
            item.put("at", at)
            try { arm(app, id, at, trigger.optBoolean("exact")) } catch (_: ExactAlarmDenied) {}
        }
        write(app, set)
    }

    private fun cancel(app: Context, id: String) {
        clearAlarm(app, id)
        val set = read(app)
        set.remove(id)
        write(app, set)
        removeMedia(app, id)
    }

    private fun alarmIntent(app: Context, id: String) = Intent(app, AlarmReceiver::class.java)
        .setAction("$name.alarm").setData(Uri.Builder().scheme("impasto-notification").authority("alarm").appendPath(id).build())
        .putExtra(noticeId, id)
    private fun clearAlarm(app: Context, id: String) {
        PendingIntent.getBroadcast(app, stableId(id), alarmIntent(app, id), PendingIntent.FLAG_NO_CREATE or PendingIntent.FLAG_IMMUTABLE)
            ?.let { alarms(app).cancel(it); it.cancel() }
    }
    @SuppressLint("ScheduleExactAlarm")
    private fun arm(app: Context, id: String, at: Long, exact: Boolean) {
        val alarms = alarms(app)
        if (exact && Build.VERSION.SDK_INT >= 31 && !alarms.canScheduleExactAlarms()) throw ExactAlarmDenied()
        val pending = PendingIntent.getBroadcast(app, stableId(id), alarmIntent(app, id),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        if (exact) alarms.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pending)
        else alarms.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pending)
    }

    private fun next(trigger: JSONObject, now: Long): Long? {
        return when (trigger.getString("kind")) {
            "date" -> trigger.getLong("atMs").takeIf { it > now }
            "interval" -> now + (trigger.getDouble("everySeconds") * 1000).toLong()
            "calendar" -> calendar(trigger, now)
            else -> null
        }
    }
    private fun calendar(trigger: JSONObject, now: Long): Long? {
        val zone = if (trigger.has("timeZone")) {
            val name = trigger.getString("timeZone")
            require(name in TimeZone.getAvailableIDs())
            TimeZone.getTimeZone(name)
        } else TimeZone.getDefault()
        val day = Calendar.getInstance(zone).apply {
            timeInMillis = now
            set(Calendar.HOUR_OF_DAY, 0); set(Calendar.MINUTE, 0); set(Calendar.SECOND, 0); set(Calendar.MILLISECOND, 0)
        }
        val targetYear = trigger.optInt("year", day.get(Calendar.YEAR))
        if (targetYear < day.get(Calendar.YEAR)) return null
        if (targetYear > day.get(Calendar.YEAR)) {
            day.set(Calendar.DAY_OF_MONTH, 1); day.set(Calendar.MONTH, Calendar.JANUARY); day.set(Calendar.YEAR, targetYear)
        }
        fun matches(key: String, actual: Int) = !trigger.has(key) || trigger.getInt(key) == actual
        fun range(key: String, max: Int): IntRange = if (trigger.has(key)) trigger.getInt(key).let { it..it } else 0..max
        repeat(366 * 8) {
            if (matches("year", day.get(Calendar.YEAR)) && matches("month", day.get(Calendar.MONTH) + 1)
                && matches("day", day.get(Calendar.DAY_OF_MONTH)) && matches("weekday", day.get(Calendar.DAY_OF_WEEK))) {
                val candidate = day.clone() as Calendar
                for (hour in range("hour", 23)) for (minute in range("minute", 59)) for (second in range("second", 59)) {
                    candidate.set(Calendar.HOUR_OF_DAY, hour); candidate.set(Calendar.MINUTE, minute); candidate.set(Calendar.SECOND, second)
                    val instant = candidate.timeInMillis
                    if (instant > now && candidate.get(Calendar.HOUR_OF_DAY) == hour) return instant
                }
            }
            day.add(Calendar.DATE, 1)
        }
        return null
    }

    @SuppressLint("MissingPermission")
    private fun display(app: Context, request: JSONObject, foregroundNow: Boolean) {
        channels(app)
        val id = request.getString("id")
        val data = request.optJSONObject("data") ?: JSONObject()
        val settings = preferences(app)
        val policy = JSONObject(settings.getString("policy", "{}") ?: "{}")
        main.post { windows.filter { it.active }.forEach { it.emit("foreground", request) } }
        if (foregroundNow && !policy.optBoolean("alert", true)) return
        val builder = NotificationCompat.Builder(app, request.text("channelId") ?: channelId)
            .setSmallIcon(app.applicationInfo.icon.takeIf { it != 0 } ?: android.R.drawable.ic_dialog_info)
            .setContentTitle(request.getString("title")).setContentText(request.getString("body"))
            .setStyle(NotificationCompat.BigTextStyle().bigText(request.getString("body"))).setAutoCancel(true)
            .setSubText(request.text("subtitle")).setGroup(request.text("threadId"))
            .setNumber(if (!foregroundNow || policy.optBoolean("badge", true)) request.optInt("badge", settings.getInt("badge", 0)) else 0)
        if (request.text("sound") == "none" || foregroundNow && !policy.optBoolean("sound", true)) builder.setSilent(true)
        app.packageManager.getLaunchIntentForPackage(app.packageName)?.let { launch ->
            launch.action = "$name.click.$id"
            launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            launch.putExtra(noticeId, id).putExtra(actionId, "default").putExtra(dataJson, data.toString())
                .putExtra(responseToken, responseToken(app))
            builder.setContentIntent(PendingIntent.getActivity(app, stableId(id), launch, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
        }
        val attachments = request.optJSONArray("attachments")
        if (attachments != null && attachments.length() > 0) {
            val path = attachments.getJSONObject(0).optString("path")
            val options = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeFile(path, options)
            if (options.outWidth > 0 && options.outHeight > 0) {
                val actual = BitmapFactory.Options().apply { inSampleSize = maxOf(1, maxOf(options.outWidth, options.outHeight) / 1024) }
                BitmapFactory.decodeFile(path, actual)?.let { builder.setStyle(NotificationCompat.BigPictureStyle().bigPicture(it)) }
            }
        }
        val categories = JSONArray(settings.getString("categories", "[]") ?: "[]")
        for (index in 0 until categories.length()) {
            val category = categories.getJSONObject(index)
            if (category.getString("id") != request.text("categoryId")) continue
            val actions = category.optJSONArray("actions") ?: continue
            for (position in 0 until actions.length()) {
                val action = actions.getJSONObject(position)
                val key = action.getString("id")
                val text = action.optJSONObject("textInput")
                val opensApp = action.optBoolean("foreground", true)
                val intent = if (opensApp) app.packageManager.getLaunchIntentForPackage(app.packageName)
                    ?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP) ?: continue
                    else Intent(app, ActionReceiver::class.java)
                intent.action = "$name.action.$id.$key"
                if (!opensApp) {
                    intent.data = Uri.Builder().scheme("impasto-notification").authority("action").appendPath(id).appendPath(key).build()
                }
                intent.putExtra(noticeId, id).putExtra(actionId, key).putExtra(dataJson, data.toString())
                    .putExtra(responseToken, responseToken(app))
                val flag = if (text != null && Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else if (text == null) PendingIntent.FLAG_IMMUTABLE else 0
                val requestCode = stableId(id + key)
                val flags = PendingIntent.FLAG_UPDATE_CURRENT or flag
                val pending = if (opensApp) PendingIntent.getActivity(app, requestCode, intent, flags)
                    else PendingIntent.getBroadcast(app, requestCode, intent, flags)
                val item = NotificationCompat.Action.Builder(0, action.getString("title"), pending)
                if (text != null) item.addRemoteInput(RemoteInput.Builder(userText).setLabel(text.optString("placeholder")).build())
                if (Build.VERSION.SDK_INT >= 31) item.setAuthenticationRequired(action.optBoolean("authenticationRequired"))
                builder.addAction(item.build())
            }
        }
        NotificationManagerCompat.from(app).notify(id, stableId(id), builder.build())
    }

    private fun createChannel(app: Context, data: JSONObject) {
        if (Build.VERSION.SDK_INT >= 26) {
            val importance = when (data.getString("importance")) {
                "min" -> NotificationManager.IMPORTANCE_MIN
                "low" -> NotificationManager.IMPORTANCE_LOW
                "high" -> NotificationManager.IMPORTANCE_HIGH
                "default" -> NotificationManager.IMPORTANCE_DEFAULT
                else -> throw IllegalArgumentException("Invalid channel importance")
            }
            val channel = NotificationChannel(data.getString("id"), data.getString("name"), importance)
            channel.description = data.text("description")
            channel.enableVibration(data.optBoolean("vibration"))
            channel.enableLights(data.optBoolean("lights"))
            channel.setShowBadge(data.optBoolean("showBadge", true))
            when (val sound = data.text("sound")) {
                "none" -> channel.setSound(null, null)
                null, "default" -> Unit
                else -> channel.setSound(Uri.parse(sound), null)
            }
            manager(app).createNotificationChannel(channel)
        }
    }

    private fun listChannels(app: Context): JSONArray {
        if (Build.VERSION.SDK_INT < 26) return JSONArray()
        return JSONArray(manager(app).notificationChannels.map { channel ->
            val importance = when (channel.importance) {
                NotificationManager.IMPORTANCE_MIN -> "min"
                NotificationManager.IMPORTANCE_LOW -> "low"
                NotificationManager.IMPORTANCE_HIGH, NotificationManager.IMPORTANCE_MAX -> "high"
                else -> "default"
            }
            JSONObject().put("id", channel.id).put("name", channel.name.toString())
                .put("description", channel.description).put("importance", importance)
                .put("sound", channel.sound?.toString() ?: "none").put("vibration", channel.shouldVibrate())
                .put("lights", channel.shouldShowLights()).put("showBadge", channel.canShowBadge())
        })
    }

    private fun media(app: Context, id: String): File {
        val key = MessageDigest.getInstance("SHA-256").digest(id.toByteArray()).joinToString("") { "%02x".format(it) }
        return File(app.noBackupFilesDir, "impasto-notification-media/$key")
    }
    private fun copyMedia(app: Context, id: String, original: JSONObject): JSONObject {
        val files = original.optJSONArray("attachments") ?: return original
        if (files.length() == 0) return original
        val root = File(media(app, id), UUID.randomUUID().toString())
        if (!root.mkdirs()) throw IOException("Could not prepare notification media")
        try {
            val output = JSONArray()
            for (index in 0 until files.length()) {
                val input = files.getJSONObject(index)
                val source = File(input.getString("path"))
                val file = File(root, index.toString())
                source.copyTo(file)
                output.put(JSONObject(input.toString()).put("path", file.path))
            }
            return JSONObject(original.toString()).put("attachments", output)
        } catch (failure: Exception) { root.deleteRecursively(); throw failure }
    }
    private fun reapMedia(app: Context, id: String, retain: JSONObject?) {
        val current = retain?.optJSONArray("attachments")?.optJSONObject(0)?.optString("path")?.let { File(it).parentFile }
        media(app, id).listFiles()?.filter { it != current }?.forEach { it.deleteRecursively() }
    }
    private fun removeMedia(app: Context, id: String) { media(app, id).deleteRecursively() }
}

private fun JSONObject.text(key: String): String? = if (isNull(key)) null else getString(key)
private fun stableId(value: String): Int = value.toByteArray().fold(0x811c9dc5.toInt()) { hash, byte ->
    (hash xor (byte.toInt() and 255)) * 16777619
}
private fun response(context: Context, intent: Intent?): JSONObject? {
    val id = intent?.getStringExtra(noticeId) ?: return null
    if (intent.getStringExtra(responseToken) != Notices.responseToken(context)) return null
    val data = runCatching { JSONObject(intent.getStringExtra(dataJson) ?: "{}") }.getOrElse { JSONObject() }
    val reply = RemoteInput.getResultsFromIntent(intent)?.getCharSequence(userText)?.toString()
    return JSONObject().put("notificationId", id).put("actionId", intent.getStringExtra(actionId) ?: "default")
        .put("userText", reply).put("data", data).put("receivedAtMs", System.currentTimeMillis())
}
