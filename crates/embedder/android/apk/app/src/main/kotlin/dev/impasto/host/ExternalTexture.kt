package dev.impasto.host

import android.graphics.ImageFormat
import android.graphics.PixelFormat
import android.media.Image
import android.media.ImageReader
import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import android.util.Log
import android.view.Surface

/** One bounded producer surface for a scene image named external:<name>. */
class ExternalTexture internal constructor(
    private val owner: Long,
    private val name: String,
    width: Int,
    height: Int,
    format: Format,
    private val removed: () -> Unit,
) : AutoCloseable {
    enum class Format(internal val pixelFormat: Int) {
        RGBA(PixelFormat.RGBA_8888),
        CAMERA_YUV(ImageFormat.YUV_420_888),
    }

    private val lock = Any()
    private var closed = false
    private var reported = false
    private val reader = ImageReader.newInstance(width, height, format.pixelFormat, 2)
    val surface: Surface get() = reader.surface

    init {
        reader.setOnImageAvailableListener(::frameAvailable, frames)
    }

    private fun frameAvailable(reader: ImageReader) {
        try {
            reader.acquireLatestImage()?.use { image ->
                val area = image.cropRect
                val pixels = when (image.format) {
                    PixelFormat.RGBA_8888 -> readRgba(image)
                    ImageFormat.YUV_420_888 -> readYuv(image)
                    else -> return
                }
                synchronized(lock) {
                    if (!closed) {
                        NativeBridge.nativeExternalTexture(owner, name, area.width(), area.height(), pixels)
                    }
                }
            }
        } catch (error: RuntimeException) {
            if (!reported) {
                reported = true
                Log.e("Impasto", "Could not read an external texture frame", error)
            }
        }
    }

    override fun close() {
        check(Looper.myLooper() == Looper.getMainLooper())
        synchronized(lock) {
            if (closed) return
            closed = true
            NativeBridge.nativeExternalTexture(owner, name, 0, 0, null)
        }
        reader.setOnImageAvailableListener(null, null)
        frames.post { reader.close() }
        removed()
    }

    private fun readRgba(image: Image): ByteArray {
        val crop = image.cropRect
        val plane = image.planes[0]
        require(plane.pixelStride == 4)
        val buffer = plane.buffer
        val first = buffer.position()
        val stride = crop.width() * 4
        return ByteArray(stride * crop.height()).also { pixels ->
            for (row in 0 until crop.height()) {
                buffer.position(first + (row + crop.top) * plane.rowStride + crop.left * 4)
                buffer.get(pixels, row * stride, stride)
            }
        }
    }

    @Suppress("MagicNumber")
    private fun readYuv(image: Image): ByteArray {
        val crop = image.cropRect
        val planes = image.planes.map(::Samples)
        val pixels = ByteArray(crop.width() * crop.height() * 4)
        var out = 0
        // BT.601 limited-range YUV 4:2:0.
        for (y in crop.top until crop.bottom) {
            for (x in crop.left until crop.right) {
                val luma = (planes[0].at(x, y) - 16).coerceAtLeast(0) * 298
                val u = planes[1].at(x / 2, y / 2) - 128
                val v = planes[2].at(x / 2, y / 2) - 128
                pixels[out++] = channel(luma + 409 * v)
                pixels[out++] = channel(luma - 100 * u - 208 * v)
                pixels[out++] = channel(luma + 516 * u)
                pixels[out++] = 0xff.toByte()
            }
        }
        return pixels
    }

    private class Samples(private val plane: Image.Plane) {
        private val buffer = plane.buffer
        private val first = buffer.position()
        fun at(x: Int, y: Int): Int =
            buffer.get(first + y * plane.rowStride + x * plane.pixelStride).toInt() and 0xff
    }

    private fun channel(value: Int) = ((value + 128) shr 8).coerceIn(0, 255).toByte()

    private companion object {
        val frames: Handler by lazy {
            Handler(HandlerThread("Impasto media frames").apply { start() }.looper)
        }
    }
}
