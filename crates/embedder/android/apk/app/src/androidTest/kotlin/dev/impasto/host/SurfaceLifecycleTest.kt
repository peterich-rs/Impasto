package dev.impasto.host

import android.app.Dialog
import android.content.Intent
import android.content.pm.ActivityInfo
import android.graphics.Bitmap
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.os.Process
import android.os.SystemClock
import android.view.PixelCopy
import android.view.SurfaceView
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.lifecycle.Lifecycle
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.google.protobuf.CodedInputStream
import com.google.protobuf.WireFormat
import org.junit.Assert.assertFalse
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertSame
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.Parameterized
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit

@RunWith(Parameterized::class)
class SurfaceLifecycleTest(private val renderer: ImpastoHost.Renderer) {
    companion object {
        @JvmStatic
        @Parameterized.Parameters(name = "{0}")
        fun renderers() = testRenderers()
    }

    @Test
    fun frameReplayStartsWithoutLeavingABufferConversionException() {
        launchFixture().use { scenario ->
            awaitPresentedBuffer(scenario)

            val fault = shell("logcat -d --pid=${Process.myPid()}").lineSequence()
                .firstOrNull { it.contains("ArrayBuffer object expected") }
            assertFalse("Frame replay startup: $fault", fault != null)
        }
    }

    @Test
    fun rotationPresentsBuffersAtTheCurrentViewSize() {
        launchFixture().use { scenario ->
            awaitPresentedBuffer(scenario)
            repeat(3) {
                rotateAndCheckBuffer(scenario, ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE)
                rotateAndCheckBuffer(scenario, ActivityInfo.SCREEN_ORIENTATION_PORTRAIT)
            }
        }
    }

    @Test
    fun stoppedActivityRecreatesItsSurfaceAndPresentsAgain() {
        launchFixture().use { scenario ->
            lateinit var initialView: ImpastoView
            scenario.onActivity { initialView = hostView(it) }
            awaitPresentedBuffer(scenario)

            repeat(3) {
                scenario.moveToState(Lifecycle.State.CREATED)
                scenario.onActivity { assertFalse(surface(hostView(it)).holder.surface.isValid) }
                scenario.moveToState(Lifecycle.State.RESUMED)
                scenario.onActivity { assertSame(initialView, hostView(it)) }
                awaitPresentedBuffer(scenario)
            }
        }
    }

    @Test
    fun aSecondHostCanPresentAndCloseInAnExistingActivity() {
        launchFixture().use { scenario ->
            lateinit var embedded: ImpastoHost
            scenario.onActivity { activity ->
                embedded = requireNotNull(ImpastoHost.attach(
                    activity, activity,
                    renderer = renderer,
                ))
                assertNotEquals(hostView(activity).handle, embedded.view.handle)
                activity.addContentView(embedded.view, FrameLayout.LayoutParams(640, 640))
            }
            awaitPresentedBuffer(scenario) { embedded.view }
            scenario.onActivity {
                (embedded.view.parent as ViewGroup).removeView(embedded.view)
                embedded.close()
            }
            awaitPresentedBuffer(scenario)
        }
    }

    @Test
    fun aLiveSessionCanMoveBetweenWindows() {
        launchFixture().use { scenario ->
            lateinit var original: ImpastoHost
            lateinit var moved: ImpastoHost
            lateinit var dialog: Dialog
            scenario.onActivity { activity ->
                original = requireNotNull(ImpastoHost.attach(
                    activity, activity,
                    renderer = renderer,
                ))
                activity.addContentView(original.view, FrameLayout.LayoutParams(480, 560))
            }
            awaitPresentedBuffer(scenario) { original.view }
            scenario.onActivity { activity ->
                val handle = original.view.handle
                val session = original.retainSession()
                assertEquals(0L, original.view.handle)
                assertFalse(original.view.sessionReady)
                (original.view.parent as ViewGroup).removeView(original.view)
                dialog = Dialog(activity)
                moved = requireNotNull(ImpastoHost.attach(activity, activity, session = session))
                assertEquals(handle, moved.view.handle)
                dialog.setContentView(moved.view)
                dialog.show()
                dialog.window?.setLayout(600, 480)
            }
            try {
                awaitPresentedBuffer(scenario) { moved.view }
            } finally {
                scenario.onActivity {
                    dialog.dismiss()
                    moved.close()
                }
            }
            awaitPresentedBuffer(scenario)
        }
    }

    private fun launchFixture(): ActivityScenario<ImpastoActivity> {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val intent = Intent(context, ImpastoActivity::class.java)
            .putExtra("impasto.renderer", renderer.wireName)
        return ActivityScenario.launch(intent)
    }

    private fun rotateAndCheckBuffer(scenario: ActivityScenario<ImpastoActivity>, orientation: Int) {
        scenario.onActivity { it.requestedOrientation = orientation }
        val landscape = orientation == ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE
        val deadline = SystemClock.uptimeMillis() + 15_000
        var size = 0 to 0
        var buffers = emptyList<Pair<Int, Int>>()
        while (SystemClock.uptimeMillis() < deadline) {
            scenario.onActivity {
                val view = surface(hostView(it))
                size = view.width to view.height
            }
            if (size.first > 0 && size.second > 0 && (size.first > size.second) == landscape) {
                buffers = activeBufferSizes()
                if (size in buffers) {
                    awaitPresentedBuffer(scenario)
                    return
                }
            }
            SystemClock.sleep(100)
        }
        throw AssertionError("orientation $orientation: View $size, SurfaceFlinger buffers $buffers")
    }

    private fun activeBufferSizes(): List<Pair<Int, Int>> {
        val packageName = InstrumentationRegistry.getInstrumentation().targetContext.packageName
        val layerPrefix = "SurfaceView[$packageName/"
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        val dump = ParcelFileDescriptor.AutoCloseInputStream(
            automation.executeShellCommand("dumpsys SurfaceFlinger --proto"),
        ).use { it.readBytes() }
        // LayersTraceFileProto.entry -> LayersTraceProto.layers -> LayersProto.layers.
        // https://android.googlesource.com/platform/frameworks/native/+/04d24da36e/services/surfaceflinger/layerproto/layers.proto
        return CodedInputStream.newInstance(dump).messages(2).flatMap { it.messages(3) }
            .flatMap { it.messages(1) }.mapNotNull { layer ->
                var name = ""
                var size: Pair<Int, Int>? = null
                var transform = 0
                while (!layer.isAtEnd) {
                    val tag = layer.readTag()
                    when (WireFormat.getTagFieldNumber(tag)) {
                        2 -> name = layer.readString()
                        27 -> size = bufferSize(CodedInputStream.newInstance(layer.readByteArray()))
                        39 -> transform = bufferTransform(CodedInputStream.newInstance(layer.readByteArray()))
                        else -> layer.skipField(tag)
                    }
                }
                size?.takeIf { layerPrefix in name && "(BLAST)" in name }?.let { (width, height) ->
                    // The producer can pre-rotate its allocation. SurfaceFlinger encodes the
                    // HAL orientation in the high byte of TransformProto.type.
                    val swapsAxes = transform and (4 shl 8) != 0
                    if (swapsAxes) height to width else width to height
                }
            }.toList()
    }

    private fun CodedInputStream.messages(field: Int): Sequence<CodedInputStream> = sequence {
        while (!isAtEnd) {
            val tag = readTag()
            if (WireFormat.getTagFieldNumber(tag) == field &&
                WireFormat.getTagWireType(tag) == WireFormat.WIRETYPE_LENGTH_DELIMITED
            ) {
                yield(CodedInputStream.newInstance(readByteArray()))
            } else {
                skipField(tag)
            }
        }
    }

    private fun bufferSize(buffer: CodedInputStream): Pair<Int, Int>? {
        var width = 0
        var height = 0
        while (!buffer.isAtEnd) {
            val tag = buffer.readTag()
            when (WireFormat.getTagFieldNumber(tag)) {
                1 -> width = buffer.readUInt32()
                2 -> height = buffer.readUInt32()
                else -> buffer.skipField(tag)
            }
        }
        return (width to height).takeIf { width > 0 && height > 0 }
    }

    private fun bufferTransform(transform: CodedInputStream): Int {
        while (!transform.isAtEnd) {
            val tag = transform.readTag()
            if (WireFormat.getTagFieldNumber(tag) == 5) return transform.readUInt32()
            transform.skipField(tag)
        }
        return 0
    }

    private fun shell(command: String): String {
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        return ParcelFileDescriptor.AutoCloseInputStream(automation.executeShellCommand(command))
            .bufferedReader().use { it.readText() }
    }

    private fun awaitPresentedBuffer(
        scenario: ActivityScenario<ImpastoActivity>,
        select: (ImpastoActivity) -> ImpastoView = ::hostView,
    ) {
        val image = Bitmap.createBitmap(32, 32, Bitmap.Config.ARGB_8888)
        val handler = Handler(Looper.getMainLooper())
        val deadline = SystemClock.uptimeMillis() + 10_000
        var lastResult = PixelCopy.ERROR_SOURCE_INVALID
        while (SystemClock.uptimeMillis() < deadline) {
            val result = ArrayBlockingQueue<Int>(1)
            scenario.onActivity {
                val view = surface(select(it))
                if (view.holder.surface.isValid) {
                    PixelCopy.request(view, image, { code -> result.offer(code) }, handler)
                } else {
                    result.offer(PixelCopy.ERROR_SOURCE_INVALID)
                }
            }
            lastResult = result.poll(2, TimeUnit.SECONDS) ?: PixelCopy.ERROR_TIMEOUT
            if (lastResult == PixelCopy.SUCCESS) return
            SystemClock.sleep(50)
        }
        throw AssertionError("Surface did not present a buffer, PixelCopy result $lastResult")
    }

    private fun hostView(activity: ImpastoActivity): ImpastoView =
        activity.findViewById<ViewGroup>(android.R.id.content).getChildAt(0) as ImpastoView

    private fun surface(view: ImpastoView): SurfaceView = view.getChildAt(0) as SurfaceView
}
