package dev.impasto.plugins.camera

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.ImageFormat
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.media.Image
import android.media.ImageReader
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.view.View
import dev.impasto.host.ExternalTexture
import dev.impasto.host.ImpastoView
import java.util.concurrent.atomic.AtomicBoolean

object CameraBridge {
    // This map and the view attachment hooks are owned by the platform thread.
    private val active = mutableMapOf<Long, Capture>()

    @JvmStatic private external fun nativeFrame(id: Long, width: Int, height: Int, pixels: ByteArray, nativePreview: Boolean)
    @JvmStatic private external fun nativeFailure(id: Long)

    @JvmStatic
    fun start(view: View, id: Long, front: Boolean, texture: String) {
        if (id in active) return
        val context = view.context
        if (context.checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            nativeFailure(id)
            return
        }
        val manager = context.getSystemService(Context.CAMERA_SERVICE) as? CameraManager
        if (manager == null) {
            nativeFailure(id)
            return
        }
        val capture = Capture(view, id, manager, texture)
        active[id] = capture
        view.addOnAttachStateChangeListener(capture)
        capture.start(front)
    }

    @JvmStatic
    fun stop(id: Long) {
        val capture = active.remove(id) ?: return
        capture.view.removeOnAttachStateChangeListener(capture)
        capture.close()
    }

    private class Capture(
        val view: View,
        private val id: Long,
        private val manager: CameraManager,
        private val texture: String,
    ) : View.OnAttachStateChangeListener {
        private val thread = HandlerThread("ImpastoCamera").apply { start() }
        private val handler = Handler(thread.looper)
        private val stopped = AtomicBoolean(false)
        private var reader: ImageReader? = null
        private var device: CameraDevice? = null
        private var session: CameraCaptureSession? = null
        private var platformPreview: ExternalTexture? = null // Main thread only.
        private val nativePreview = AtomicBoolean(false)

        fun start(front: Boolean) {
            handler.post {
                if (stopped.get()) return@post
                try {
                    val facing = if (front) CameraCharacteristics.LENS_FACING_FRONT else CameraCharacteristics.LENS_FACING_BACK
                    val camera = manager.cameraIdList.firstOrNull { camera ->
                        manager.getCameraCharacteristics(camera)[CameraCharacteristics.LENS_FACING] == facing
                    } ?: return@post fail()
                    val formats = manager.getCameraCharacteristics(camera)[CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP]
                    val size = formats?.getOutputSizes(ImageFormat.YUV_420_888)
                        ?.filter { it.width.toLong() * it.height <= 640L * 480L }
                        ?.maxByOrNull { it.width.toLong() * it.height }
                        ?: return@post fail()
                    val frames = ImageReader.newInstance(size.width, size.height, ImageFormat.YUV_420_888, 2)
                    reader = frames
                    frames.setOnImageAvailableListener({ source ->
                        source.acquireLatestImage()?.use { image ->
                            if (!stopped.get()) {
                                nativeFrame(id, image.width, image.height, rgba(image), nativePreview.get())
                            }
                        }
                    }, handler)
                    view.post mount@ {
                        if (stopped.get()) return@mount
                        val host = view as? ImpastoView
                        platformPreview = if (Build.VERSION.SDK_INT >= 33 && host != null) {
                            runCatching {
                                host.openExternalTexture(texture, size.width, size.height, ExternalTexture.Format.CAMERA_YUV)
                            }.getOrNull()
                        } else null
                        val preview = platformPreview
                        handler.post { open(camera, frames, preview) }
                    }
                } catch (_: Exception) {
                    fail()
                }
            }
        }

        private fun open(id: String, frames: ImageReader, preview: ExternalTexture?) {
            if (stopped.get()) return
            try {
                manager.openCamera(id, object : CameraDevice.StateCallback() {
                    override fun onOpened(camera: CameraDevice) {
                        if (stopped.get()) {
                            camera.close()
                            return
                        }
                        device = camera
                        capture(camera, frames, preview)
                    }

                    override fun onDisconnected(camera: CameraDevice) {
                        camera.close()
                        fail()
                    }

                    override fun onError(camera: CameraDevice, error: Int) {
                        camera.close()
                        fail()
                    }
                }, handler)
            } catch (_: Exception) {
                fail()
            }
        }

        @Suppress("DEPRECATION") // The callback API also works on Android 24.
        private fun capture(camera: CameraDevice, frames: ImageReader, preview: ExternalTexture?) {
            val surfaces = listOfNotNull(frames.surface, preview?.surface)
            try {
                camera.createCaptureSession(surfaces, object : CameraCaptureSession.StateCallback() {
                    override fun onConfigured(configured: CameraCaptureSession) {
                        if (stopped.get()) {
                            configured.close()
                            return
                        }
                        session = configured
                        try {
                            val request = camera.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW)
                            surfaces.forEach { request.addTarget(it) }
                            nativePreview.set(preview != null)
                            configured.setRepeatingRequest(request.build(), null, handler)
                        } catch (_: Exception) {
                            configured.close()
                            fallback(camera, frames, preview)
                        }
                    }

                    override fun onConfigureFailed(configured: CameraCaptureSession) {
                        configured.close()
                        fallback(camera, frames, preview)
                    }
                }, handler)
            } catch (_: Exception) {
                fallback(camera, frames, preview)
            }
        }

        private fun fallback(camera: CameraDevice, frames: ImageReader, preview: ExternalTexture?) {
            if (preview == null) {
                fail()
                return
            }
            view.post {
                closePreview()
                if (!stopped.get()) handler.post { capture(camera, frames, null) }
            }
        }

        private fun closePreview() {
            nativePreview.set(false)
            platformPreview?.close()
            platformPreview = null
        }

        private fun fail() {
            if (!stopped.get()) nativeFailure(id)
            view.post { if (active[id] === this) stop(id) }
            close()
        }

        fun close() {
            if (!stopped.compareAndSet(false, true)) return
            view.post { closePreview() }
            handler.post {
                session?.close()
                device?.close()
                reader?.close()
                thread.quitSafely()
            }
        }

        override fun onViewAttachedToWindow(view: View) = Unit
        override fun onViewDetachedFromWindow(view: View) = stop(id)
    }

    private fun rgba(image: Image): ByteArray {
        val width = image.width
        val height = image.height
        val data = ByteArray(width * height * 4)
        val (luma, blue, red) = image.planes
        val yBytes = luma.buffer
        val uBytes = blue.buffer
        val vBytes = red.buffer
        val yStart = yBytes.position()
        val uStart = uBytes.position()
        val vStart = vBytes.position()
        var output = 0
        for (row in 0 until height) {
            for (column in 0 until width) {
                val y = (yBytes.get(yStart + row * luma.rowStride + column * luma.pixelStride).toInt() and 255) - 16
                val u = (uBytes.get(uStart + (row / 2) * blue.rowStride + (column / 2) * blue.pixelStride).toInt() and 255) - 128
                val v = (vBytes.get(vStart + (row / 2) * red.rowStride + (column / 2) * red.pixelStride).toInt() and 255) - 128
                data[output++] = ((298 * y + 409 * v + 128) shr 8).coerceIn(0, 255).toByte()
                data[output++] = ((298 * y - 100 * u - 208 * v + 128) shr 8).coerceIn(0, 255).toByte()
                data[output++] = ((298 * y + 516 * u + 128) shr 8).coerceIn(0, 255).toByte()
                data[output++] = 0xff.toByte()
            }
        }
        return data
    }
}
