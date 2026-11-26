package dev.impasto.host

import android.content.Intent
import android.os.Bundle
import androidx.activity.result.ActivityResultRegistry
import androidx.activity.result.contract.ActivityResultContract
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.app.ActivityOptionsCompat
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.impasto.plugins.permissions.Permissions
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class PermissionRecreationTest {
    @Test
    fun resultReturnsToTheOriginalRequestThroughARestoredRegistry() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val intent = Intent(context, ImpastoActivity::class.java)
            .putExtra("script", "host-test/editor.js")
        val original = Registry()
        val state = Bundle()
        val key = "impasto.permissions.test"
        val requestId = 81L
        val completed = mutableListOf<Long>()
        ActivityScenario.launch<ImpastoActivity>(intent).use { scenario ->
            scenario.onActivity {
                val launch = original.register(key, ActivityResultContracts.RequestMultiplePermissions()) {
                    error("old registry received the result")
                }
                launch.launch(arrayOf(android.Manifest.permission.CAMERA))
                original.onSaveInstanceState(state)
                launch.unregister()
            }
            scenario.recreate()
            scenario.onActivity { activity ->
                val replacement = Registry()
                replacement.onRestoreInstanceState(state)
                val pending = Permissions.Pending(key, requestId, arrayOf("camera"))
                Permissions(activity, replacement, pending) { id, statuses, error ->
                    completed.add(id)
                    assertEquals(1, statuses?.size)
                    assertEquals(null, error)
                }.use { permission ->
                    replacement.dispatchResult(
                        original.launched,
                        mapOf(android.Manifest.permission.CAMERA to false),
                    )
                    assertFalse(permission.hasPendingRequest)
                }
            }
        }
        assertEquals(listOf(requestId), completed)
    }

    private class Registry : ActivityResultRegistry() {
        var launched: Int = 0
            private set

        override fun <I, O> onLaunch(
            requestCode: Int,
            contract: ActivityResultContract<I, O>,
            input: I,
            options: ActivityOptionsCompat?,
        ) {
            launched = requestCode
        }
    }
}
