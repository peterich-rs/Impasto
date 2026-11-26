package dev.impasto.host

import android.graphics.Rect
import android.os.Build
import android.text.InputType
import android.util.SparseArray
import android.view.View
import android.view.ViewStructure
import android.view.autofill.AutofillManager
import android.view.autofill.AutofillValue
import androidx.annotation.RequiresApi
import kotlin.math.roundToInt

@RequiresApi(Build.VERSION_CODES.O)
internal class AutofillBridge(private val view: ImpastoView) {
    private class Field(
        val nodeId: Long,
        val virtualId: Int,
        val hint: String,
        val value: String,
        val bounds: Rect,
    )

    private val manager = view.context.getSystemService(AutofillManager::class.java)
    private var fields = emptyMap<Long, Field>()
    private var nextId = 1
    private var focusedField: Field? = null
    private var entered: Field? = null
    val enabled: Boolean get() = manager?.isEnabled == true

    fun update(
        ids: LongArray, labels: Array<String>, values: Array<String>,
        flags: IntArray, bounds: FloatArray,
    ) {
        val nextFields = LinkedHashMap<Long, Field>()
        val visible = Rect()
        val hasVisibleRect = view.getLocalVisibleRect(visible)
        var nextFocus: Field? = null
        for (index in ids.indices) {
            val label = labels[index]
            if (label.isEmpty()) continue
            val base = index * 4
            val rect = Rect(
                pixels(bounds[base]), pixels(bounds[base + 1]),
                pixels(bounds[base] + bounds[base + 2]), pixels(bounds[base + 1] + bounds[base + 3]),
            )
            if (!hasVisibleRect || !rect.intersect(visible)) continue
            val id = ids[index]
            val previous = fields[id]
            val field = Field(id, previous?.virtualId ?: nextId++, label, values[index], rect)
            nextFields[id] = field
            if (flags[index] and FOCUSED != 0) nextFocus = field
        }
        val previous = fields
        fields = nextFields
        focusedField = nextFocus
        windowFocusChanged(view.hasWindowFocus())
        for ((id, field) in nextFields) {
            if (previous[id]?.value != field.value) {
                manager?.notifyValueChanged(view, field.virtualId, AutofillValue.forText(field.value))
            }
        }
    }

    fun provide(structure: ViewStructure) {
        val parent = structure.autofillId ?: return
        val start = structure.addChildCount(fields.size)
        for ((index, field) in fields.values.withIndex()) {
            val child = structure.newChild(start + index)
            child.setAutofillId(parent, field.virtualId)
            child.setClassName("android.widget.EditText")
            child.setAutofillType(View.AUTOFILL_TYPE_TEXT)
            child.setAutofillHints(arrayOf(field.hint))
            child.setInputType(InputType.TYPE_CLASS_TEXT or when (field.hint) {
                "password", "newPassword" -> InputType.TYPE_TEXT_VARIATION_PASSWORD
                "emailAddress" -> InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS
                else -> InputType.TYPE_TEXT_VARIATION_NORMAL
            })
            child.setDataIsSensitive(true)
            child.setFocused(entered?.virtualId == field.virtualId)
            val rect = field.bounds
            child.setDimens(rect.left, rect.top, 0, 0, rect.width(), rect.height())
            child.setAutofillValue(AutofillValue.forText(field.value))
        }
    }

    fun fill(values: SparseArray<AutofillValue>) {
        if (view.handle == 0L) return
        for (field in fields.values) {
            val value = values[field.virtualId] ?: continue
            if (value.isText) NativeBridge.nativeAutofill(view.handle, field.nodeId, value.textValue.toString())
        }
    }

    fun windowFocusChanged(focused: Boolean) {
        val next = focusedField.takeIf { focused && view.isShown }
        if (entered?.virtualId == next?.virtualId && entered?.bounds == next?.bounds) return
        entered?.let { manager?.notifyViewExited(view, it.virtualId) }
        entered = next
        if (next != null) {
            val origin = IntArray(2)
            view.getLocationOnScreen(origin)
            val screen = Rect(next.bounds).apply { offset(origin[0], origin[1]) }
            manager?.notifyViewEntered(view, next.virtualId, screen)
        }
    }

    fun clear() {
        focusedField = null
        windowFocusChanged(false)
        fields = emptyMap()
    }

    private fun pixels(value: Float): Int = (value * view.density).roundToInt()

    private companion object {
        const val FOCUSED = 32
    }
}
