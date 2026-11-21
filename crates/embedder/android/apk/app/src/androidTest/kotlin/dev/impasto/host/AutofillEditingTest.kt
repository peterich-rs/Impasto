package dev.impasto.host

import android.app.UiAutomation
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.util.SparseArray
import android.view.ViewGroup
import android.view.autofill.AutofillManager
import android.view.autofill.AutofillValue
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AutofillEditingTest {
    @Test
    fun androidFillReachesTheSceneEditor() {
        assumeTrue(Build.VERSION.SDK_INT >= 26)
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val automation = instrumentation.uiAutomation
        val service = ComponentName(instrumentation.context, TestAutofillService::class.java)
        val setting = "secure autofill_service"
        val previous = automation.runShell("settings get $setting").trim()
        try {
            automation.runShell("settings put $setting ${service.flattenToString()}")
            fillEditor(instrumentation.targetContext)
        } finally {
            if (previous == "null") {
                automation.runShell("settings delete $setting")
            } else {
                automation.runShell("settings put $setting $previous")
            }
        }
    }

    private fun fillEditor(context: Context) {
        val intent = Intent(context, ImpastoActivity::class.java)
            .putExtra("script", "host-test/autofill.js")
        ActivityScenario.launch<ImpastoActivity>(intent).use { scenario ->
            val dataset = SparseArray<AutofillValue>().apply {
                put(1, AutofillValue.forText("Android fill 😀"))
            }
            val deadline = SystemClock.uptimeMillis() + 10_000
            while (SystemClock.uptimeMillis() < deadline) {
                var filled = false
                scenario.onActivity { activity ->
                    if (activity.getSystemService(AutofillManager::class.java)?.isEnabled != true) {
                        return@onActivity
                    }
                    val root = activity.findViewById<ViewGroup>(android.R.id.content).getChildAt(0) as ImpastoView
                    val snapshot = NativeBridge.nativeSavedState(root.handle)
                    filled = snapshot != null && JSONObject(snapshot.substringAfter('\n')).optString("filled") == "yes"
                    if (!filled) root.autofill(dataset)
                }
                if (filled) return
                SystemClock.sleep(50)
            }
            throw AssertionError("The Android autofill dataset did not reach the scene editor")
        }
    }

    private fun UiAutomation.runShell(command: String): String =
        ParcelFileDescriptor.AutoCloseInputStream(executeShellCommand(command))
            .bufferedReader().use { it.readText() }
}
