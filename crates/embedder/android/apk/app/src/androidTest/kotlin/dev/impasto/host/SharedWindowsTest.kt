package dev.impasto.host

import android.app.Dialog
import android.content.Intent
import android.os.SystemClock
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class SharedWindowsTest {
    @Test
    fun twoWindowsShareApplicationValuesAndKeepIndependentViewports() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val intent = Intent(context, ImpastoActivity::class.java).putExtra("script", "host-test/editor.js")
        ActivityScenario.launch<ImpastoActivity>(intent).use { scenario ->
            lateinit var session: ImpastoWindowSession
            lateinit var first: ImpastoHost
            lateinit var second: ImpastoHost
            lateinit var dialog: Dialog
            scenario.onActivity { activity ->
                session = ImpastoWindowSession()
                fun attach() = requireNotNull(ImpastoHost.attach(
                    activity, activity, script = "host-test/window-state.js",
                    renderer = ImpastoHost.Renderer.GLES, windowSession = session,
                ))
                first = attach()
                second = attach()
                activity.addContentView(first.view, FrameLayout.LayoutParams(480, 420))
                dialog = Dialog(activity).apply {
                    setContentView(second.view)
                    show()
                    window?.setLayout(640, 500)
                }
            }
            try {
                val firstWidth = await(first) { it.has("width") }.getString("width")
                val secondWidth = await(second) { it.has("width") }.getString("width")
                assertNotEquals(firstWidth, secondWidth)
                scenario.onActivity { NativeBridge.nativeBackGesture(first.view.handle, 3, 0f) }
                assertEquals(firstWidth, await(first) { it.optString("shared") == "1" }.getString("width"))
                assertEquals(secondWidth, await(second) { it.optString("shared") == "1" }.getString("width"))
                scenario.onActivity {
                    (first.view.parent as ViewGroup).removeView(first.view)
                    first.close()
                    dialog.window?.setLayout(560, 500)
                    NativeBridge.nativeBackGesture(second.view.handle, 3, 0f)
                }
                val resized = await(second) {
                    it.optString("shared") == "2" && it.optString("width") != secondWidth
                }
                assertTrue(resized.getString("width").toFloat() < secondWidth.toFloat())
            } finally {
                scenario.onActivity {
                    dialog.dismiss()
                    session.close()
                }
            }
        }
    }

    private fun await(host: ImpastoHost, condition: (JSONObject) -> Boolean): JSONObject {
        val deadline = SystemClock.uptimeMillis() + 10_000
        var last = JSONObject()
        while (SystemClock.uptimeMillis() < deadline) {
            NativeBridge.nativeSavedState(host.view.handle)?.let {
                last = JSONObject(it.substringAfter('\n'))
            }
            if (condition(last)) return last
            SystemClock.sleep(25)
        }
        throw AssertionError("Unexpected window snapshot: $last")
    }
}
