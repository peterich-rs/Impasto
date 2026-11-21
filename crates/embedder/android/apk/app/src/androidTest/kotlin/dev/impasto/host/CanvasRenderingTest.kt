package dev.impasto.host

import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.Rect
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.PixelCopy
import android.view.SurfaceView
import android.view.ViewGroup
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit

@RunWith(AndroidJUnit4::class)
class CanvasRenderingTest {
    @Test
    fun curvedStrokeDoesNotFillItsBoundingBoxOnGles() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val intent = Intent(context, ImpastoActivity::class.java)
            .putExtra("script", "host-test/canvas-path.js")
            .putExtra("impasto.renderer", "gles")
        ActivityScenario.launch<ImpastoActivity>(intent).use { scenario ->
            val image = Bitmap.createBitmap(360, 640, Bitmap.Config.ARGB_8888)
            try {
                awaitPixels(scenario, image)
                val pixels = IntArray(image.width * image.height)
                image.getPixels(pixels, 0, image.width, 0, 0, image.width, image.height)
                val bounds = Rect()
                var colored = 0
                for ((index, pixel) in pixels.withIndex()) {
                    if (Color.red(pixel) < 80 && Color.green(pixel) > 180 && Color.blue(pixel) > 100) {
                        val x = index % image.width
                        val y = index / image.width
                        bounds.union(x, y, x + 1, y + 1)
                        colored++
                    }
                }
                assertTrue("Expected visible stroke, got $colored pixels in $bounds", bounds.width() > 30 && bounds.height() > 24)
                val coverage = colored.toFloat() / (bounds.width() * bounds.height())
                assertTrue("Stroke filled $coverage of its bounding box", coverage < 0.35f)
            } finally {
                image.recycle()
            }
        }
    }

    private fun awaitPixels(scenario: ActivityScenario<ImpastoActivity>, image: Bitmap) {
        val handler = Handler(Looper.getMainLooper())
        val deadline = SystemClock.uptimeMillis() + 5_000
        while (SystemClock.uptimeMillis() < deadline) {
            val result = ArrayBlockingQueue<Int>(1)
            scenario.onActivity { activity ->
                val root = activity.findViewById<ViewGroup>(android.R.id.content)
                val host = root.getChildAt(0) as ImpastoView
                val surface = host.getChildAt(0) as SurfaceView
                if (surface.holder.surface.isValid) {
                    PixelCopy.request(surface, image, { result.offer(it) }, handler)
                } else {
                    result.offer(PixelCopy.ERROR_SOURCE_INVALID)
                }
            }
            if (result.poll(2, TimeUnit.SECONDS) == PixelCopy.SUCCESS) return
            SystemClock.sleep(30)
        }
        throw AssertionError("GLES surface did not present a buffer")
    }
}
