package dev.impasto.host

import android.content.Intent
import android.net.Uri
import android.os.SystemClock
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.activity.BackEventCompat
import androidx.activity.OnBackPressedDispatcher
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

@RunWith(AndroidJUnit4::class)
class TaskRestorationTest {
    @Test
    fun declinedGestureReturnsToTheNativeHostAndDetachingDisablesTheCallback() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val intent = Intent(context, ImpastoActivity::class.java)
        val fallback = CountDownLatch(1)
        ActivityScenario.launch<ImpastoActivity>(intent).use { scenario ->
            lateinit var host: ImpastoHost
            lateinit var dispatcher: OnBackPressedDispatcher
            scenario.onActivity { activity ->
                dispatcher = OnBackPressedDispatcher(fallback::countDown)
                host = requireNotNull(ImpastoHost.attach(
                    activity, activity,
                    script = "host-test/back-fallback.js",
                    backDispatcher = dispatcher,
                ))
                activity.addContentView(host.view, FrameLayout.LayoutParams(480, 640))
            }
            awaitBackOwner(scenario, true, { dispatcher })
            scenario.onActivity {
                dispatcher.dispatchOnBackStarted(BackEventCompat(10f, 50f, 0f, BackEventCompat.EDGE_LEFT))
                dispatcher.onBackPressed()
            }
            assertTrue("native return fallback was not called", fallback.await(10, TimeUnit.SECONDS))
            scenario.onActivity {
                (host.view.parent as ViewGroup).removeView(host.view)
                assertFalse(dispatcher.hasEnabledCallbacks())
                host.close()
            }
        }
    }

    @Test
    fun recreatedTaskRestoresSnapshotBeforeEvaluationAndDoesNotReplayIntent() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val url = "impasto-test://record/12"
        val intent = Intent(context, ImpastoActivity::class.java)
            .setData(Uri.parse(url))
            .putExtra("script", "host-test/restoration.js")
        ActivityScenario.launch<ImpastoActivity>(intent).use { scenario ->
            val original = awaitState(scenario) { it.has("refreshRate") }
            assertEquals("1", original.getString("starts"))
            assertEquals(url, original.getString("initialUrl"))
            assertTrue(original.getString("refreshRate").toFloat() > 0f)

            scenario.recreate()
            val recreated = awaitState(scenario) { it.optString("starts") == "2" }
            assertFalse(recreated.has("initialUrl"))
            awaitBackOwner(scenario, true)
            val start = BackEventCompat(24f, 64f, 0f, BackEventCompat.EDGE_LEFT)
            val halfway = BackEventCompat(128f, 64f, 0.5f, BackEventCompat.EDGE_LEFT)
            scenario.onActivity { activity ->
                val dispatcher = activity.onBackPressedDispatcher
                assertTrue(dispatcher.hasEnabledCallbacks())
                dispatcher.dispatchOnBackStarted(start)
                dispatcher.dispatchOnBackProgressed(halfway)
            }
            awaitState(scenario) { it.optString("gesture") == "progress:0.5" }
            scenario.onActivity { it.onBackPressedDispatcher.dispatchOnBackCancelled() }
            awaitState(scenario) { it.optString("gesture") == "cancel" }
            scenario.onActivity {
                it.onBackPressedDispatcher.dispatchOnBackStarted(start)
                it.onBackPressedDispatcher.onBackPressed()
            }
            awaitState(scenario) { it.optString("gesture") == "commit" }
            awaitBackOwner(scenario, false)
        }
    }

    private fun awaitBackOwner(
        scenario: ActivityScenario<ImpastoActivity>,
        expected: Boolean,
        dispatcher: (ImpastoActivity) -> OnBackPressedDispatcher = { it.onBackPressedDispatcher },
    ) {
        val deadline = SystemClock.uptimeMillis() + 10_000
        while (SystemClock.uptimeMillis() < deadline) {
            var ready = false
            scenario.onActivity { ready = dispatcher(it).hasEnabledCallbacks() == expected }
            if (ready) return
            SystemClock.sleep(25)
        }
        throw AssertionError("Expected framework back callback enabled=$expected")
    }

    private fun awaitState(
        scenario: ActivityScenario<ImpastoActivity>,
        condition: (JSONObject) -> Boolean,
    ): JSONObject {
        val deadline = SystemClock.uptimeMillis() + 10_000
        var last = JSONObject()
        while (SystemClock.uptimeMillis() < deadline) {
            scenario.onActivity { activity ->
                val view = activity.findViewById<ViewGroup>(android.R.id.content).getChildAt(0) as ImpastoView
                NativeBridge.nativeSavedState(view.handle)?.let { last = JSONObject(it.substringAfter('\n')) }
            }
            if (condition(last)) return last
            SystemClock.sleep(25)
        }
        throw AssertionError("Unexpected task snapshot: $last")
    }
}
