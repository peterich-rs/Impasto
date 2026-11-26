package dev.impasto.fieldnotes.embedded

import android.content.Context
import android.content.ActivityNotFoundException
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.os.Bundle
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import android.widget.Button
import android.widget.EditText
import android.widget.ScrollView
import android.widget.LinearLayout
import android.widget.Toast
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.fragment.app.Fragment
import androidx.fragment.app.FragmentActivity
import androidx.fragment.app.FragmentContainerView
import dev.impasto.host.ExternalTexture
import dev.impasto.host.ImpastoHost
import dev.impasto.host.PlatformViewFactory
import dev.impasto.plugins.filesystem.FileSystemPlugin
import dev.impasto.plugins.linking.LinkingPlugin
import dev.impasto.plugins.permissions.PermissionsPlugin

class NativeActivity : FragmentActivity() {
    private val capturePhoto = registerForActivityResult(ActivityResultContracts.TakePicturePreview()) { image ->
        if (image != null) {
            (supportFragmentManager.findFragmentById(CONTAINER_ID) as? NotesFragment)
                ?.showPhoto(image)
        }
    }

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        enableEdgeToEdge()

        val root = LinearLayout(this).also { it.orientation = LinearLayout.VERTICAL }
        ViewCompat.setOnApplyWindowInsetsListener(root) { view, insets ->
            val bars = WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
            val safe = insets.getInsets(bars)
            view.setPadding(safe.left, safe.top, safe.right, safe.bottom)
            WindowInsetsCompat.Builder(insets).setInsets(bars, Insets.NONE).build()
        }
        root.addView(heading(this, "原生宿主 · 现场检查", 18f))
        val capture = Button(this).apply {
            text = "拍摄现场照片"
            isAllCaps = false
            setOnClickListener {
                try {
                    capturePhoto.launch(null)
                } catch (_: ActivityNotFoundException) {
                    Toast.makeText(context, "没有可用的相机应用", Toast.LENGTH_SHORT).show()
                }
            }
        }
        root.addView(capture)
        val container = FragmentContainerView(this).apply { id = CONTAINER_ID }
        root.addView(container, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        setContentView(root)

        if (state == null) {
            supportFragmentManager.beginTransaction().replace(container.id, NotesFragment()).commit()
        }
    }

    private companion object {
        const val CONTAINER_ID = 0x12340
    }
}

class NotesFragment : Fragment() {
    private var photo: ExternalTexture? = null

    fun showPhoto(image: Bitmap) {
        val surface = photo?.surface ?: return
        val canvas = surface.lockCanvas(null)
        try {
            canvas.drawColor(Color.BLACK)
            val scale = minOf(canvas.width.toFloat() / image.width, canvas.height.toFloat() / image.height)
            val width = (image.width * scale).toInt()
            val height = (image.height * scale).toInt()
            val left = (canvas.width - width) / 2
            val top = (canvas.height - height) / 2
            canvas.drawBitmap(image, null, Rect(left, top, left + width, top + height), Paint(Paint.FILTER_BITMAP_FLAG))
        } finally {
            surface.unlockCanvasAndPost(canvas)
        }
    }

    override fun onDestroyView() {
        photo?.close()
        photo = null
        super.onDestroyView()
    }

    override fun onCreateView(inflater: LayoutInflater, container: ViewGroup?, state: Bundle?): View =
        FrameLayout(requireContext())

    override fun onViewCreated(view: View, state: Bundle?) {
        val container = view as FrameLayout
        fun showFailure() {
            container.removeAllViews()
            container.addView(heading(requireContext(), "无法启动现场记录", 16f))
        }
        val host = ImpastoHost.attach(
            requireActivity(), viewLifecycleOwner,
            initialUrl = if (state == null) "impasto-demo://record/42?native=1" else null,
            restorationId = "notes",
            backDispatcher = requireActivity().onBackPressedDispatcher,
            onStartupFailure = { showFailure() },
            platformViewFactories = mapOf(
                "site-editor" to PlatformViewFactory { context ->
                    EditText(context).apply {
                        hint = "原生补充说明"
                        setSingleLine()
                    }
                },
                "site-checklist" to PlatformViewFactory { context ->
                    ScrollView(context).apply {
                        addView(LinearLayout(context).also { list ->
                            list.orientation = LinearLayout.VERTICAL
                            for (label in listOf("设备外观", "操作面板", "紧固件", "清洁记录")) {
                                list.addView(heading(context, label, 15f))
                            }
                        })
                    }
                },
            ),
        )
        if (host == null) {
            showFailure()
        } else {
            FileSystemPlugin.register(host)
            LinkingPlugin.register(host)
            PermissionsPlugin.register(host)
        container.addView(host.view, FrameLayout.LayoutParams(-1, -1))
            photo = host.view.openExternalTexture("site-photo", 320, 180)
        }
    }
}

private fun heading(context: Context, label: String, size: Float) = android.widget.TextView(context).apply {
    text = label
    textSize = size
    val pad = (16 * resources.displayMetrics.density).toInt()
    setPadding(pad, pad, pad, pad)
}
