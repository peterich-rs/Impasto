package dev.impasto.host

import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Color
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.KeyEvent
import android.view.PixelCopy
import android.view.SurfaceView
import android.view.View
import android.view.ViewGroup
import android.widget.EditText
import android.widget.FrameLayout
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.Parameterized
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

@RunWith(Parameterized::class)
class PlatformContentTest(private val renderer: ImpastoHost.Renderer) {
    companion object {
        @JvmStatic
        @Parameterized.Parameters(name = "{0}")
        fun renderers() = listOf(ImpastoHost.Renderer.VULKAN, ImpastoHost.Renderer.GLES)
    }

    @Test
    fun sceneMountsANativeEditorAndReplacesProducerFrames() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val intent = Intent(context, ImpastoActivity::class.java)
            .putExtra("script", "host-test/platform-views.js")
            .putExtra("impasto.renderer", "gles")
        ActivityScenario.launch<ImpastoActivity>(intent).use { scenario ->
            lateinit var host: ImpastoHost
            lateinit var stream: ExternalTexture
            val presented = CountDownLatch(1)
            val destroyed = CountDownLatch(1)
            scenario.onActivity { activity ->
                val root = activity.findViewById<ViewGroup>(android.R.id.content)
                root.removeAllViews()
                val editor = object : PlatformViewFactory {
                    override fun create(context: android.content.Context) = EditText(context)
                    override fun dispose(view: View) { destroyed.countDown() }
                }
                host = requireNotNull(ImpastoHost.attach(
                    activity, activity, script = "host-test/platform-views.js", renderer = renderer,
                    platformViewFactories = mapOf("edit" to editor),
                    onFirstFrameSubmitted = presented::countDown,
                ))
                activity.setContentView(host.view)
                stream = host.view.openExternalTexture("camera", 16, 16)
            }
            assertTrue(presented.await(10, TimeUnit.SECONDS))
            await {
                var ready = false
                scenario.onActivity { ready = editor(host.view)?.width?.let { it > 0 } ?: false }
                ready
            }
            scenario.onActivity {
                val field = requireNotNull(editor(host.view))
                field.requestFocus()
                val key = KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_A)
                assertTrue(host.view.dispatchKeyEvent(key))
                assertEquals("a", field.text.toString())
            }
            try {
                for (color in listOf(Color.RED, Color.BLUE)) {
                    val canvas = stream.surface.lockCanvas(null)
                    canvas.drawColor(color)
                    stream.surface.unlockCanvasAndPost(canvas)
                    await { pixel(scenario, host.view, color) }
                }
            } finally {
                scenario.onActivity { host.close() }
            }
            assertTrue(destroyed.await(2, TimeUnit.SECONDS))
        }
    }

    private fun pixel(scenario: ActivityScenario<ImpastoActivity>, view: ImpastoView, color: Int): Boolean {
        val done = CountDownLatch(1)
        var matches = false
        scenario.onActivity {
            val surface = view.getChildAt(0) as SurfaceView
            val bitmap = Bitmap.createBitmap(surface.width, surface.height, Bitmap.Config.ARGB_8888)
            PixelCopy.request(surface, bitmap, { status ->
                // An empty script has no other solid red or blue pixels.
                if (status == PixelCopy.SUCCESS) {
                    val x = (32 * view.density).toInt()
                    for (y in 0 until bitmap.height) {
                        if (bitmap.getPixel(x, y) == color) {
                            matches = true
                            break
                        }
                    }
                }
                bitmap.recycle()
                done.countDown()
            }, Handler(Looper.getMainLooper()))
        }
        return done.await(2, TimeUnit.SECONDS) && matches
    }

    private fun editor(view: View): EditText? {
        if (view is EditText) return view
        if (view is ViewGroup) {
            for (index in 0 until view.childCount) {
                editor(view.getChildAt(index))?.let { return it }
            }
        }
        return null
    }

    private fun await(condition: () -> Boolean) {
        val deadline = SystemClock.uptimeMillis() + 10_000
        while (SystemClock.uptimeMillis() < deadline) {
            if (condition()) return
            SystemClock.sleep(30)
        }
        throw AssertionError("The scene did not expose the expected Android content")
    }
}
