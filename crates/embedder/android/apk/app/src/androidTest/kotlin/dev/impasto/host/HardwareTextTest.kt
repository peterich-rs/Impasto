package dev.impasto.host

import android.text.Editable
import android.text.Selection
import android.text.SpannableStringBuilder
import android.view.View
import android.view.inputmethod.BaseInputConnection
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class HardwareTextTest {
    @Test
    fun physicalKeyPreservesTheTextTheImeJustMarked() {
        val device = InstrumentationRegistry.getInstrumentation()
        device.runOnMainSync {
            val editable = SpannableStringBuilder("native")
            Selection.setSelection(editable, editable.length)
            val ime = object : BaseInputConnection(View(device.targetContext), true) {
                override fun getEditable(): Editable = editable
            }

            for (character in "123456789") {
                ime.setComposingRegion(0, editable.length)
                insertHardwareText(editable, character.toString())
            }

            assertEquals("native123456789", editable.toString())
            assertEquals(editable.length, Selection.getSelectionStart(editable))
            assertEquals(editable.length, Selection.getSelectionEnd(editable))
            assertEquals(-1, BaseInputConnection.getComposingSpanStart(editable))
        }
    }

    @Test
    fun physicalKeyReplacesSelectionAndCountsUtf16Units() {
        val editable = SpannableStringBuilder("abcd")
        Selection.setSelection(editable, 3, 1)

        insertHardwareText(editable, "\uD83D\uDE00")

        assertEquals("a\uD83D\uDE00d", editable.toString())
        assertEquals(3, Selection.getSelectionStart(editable))
        assertEquals(3, Selection.getSelectionEnd(editable))
    }

    @Test
    fun physicalDeleteKeepsCombiningMarksAndEmojiSequencesTogether() {
        val editable = SpannableStringBuilder("Ae\u0301👩‍💻Z")
        Selection.setSelection(editable, 3)
        deleteHardwareText(editable, backwards = true)
        assertEquals("A👩‍💻Z", editable.toString())
        assertEquals(1, Selection.getSelectionStart(editable))
        deleteHardwareText(editable, backwards = false)
        assertEquals("AZ", editable.toString())
        assertEquals(1, Selection.getSelectionStart(editable))
    }
}
