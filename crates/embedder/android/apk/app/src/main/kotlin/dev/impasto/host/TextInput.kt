package dev.impasto.host

import android.graphics.Matrix
import android.icu.text.BreakIterator
import android.os.Build
import android.text.Editable
import android.text.Selection
import android.text.SpannableStringBuilder
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CursorAnchorInfo
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.ExtractedText
import android.view.inputmethod.ExtractedTextRequest
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.inputmethod.EditorInfoCompat

/** Mirrors the focused session field and owns the currently authorized input connection. */
internal class TextInput(private val view: ImpastoView) {
    private val imm = view.context.getSystemService(InputMethodManager::class.java)
    private val editable = SpannableStringBuilder().apply { Selection.setSelection(this, 0) }
    private var inputType = EditorInfo.TYPE_CLASS_TEXT
    private var imeAction = EditorInfo.IME_ACTION_DONE
    private var connection: Connection? = null
    private var lastSent: EditingState? = null
    private var caret = Caret(0f, 0f, 0f, 0f)
    private var deadKey = 0
    var active = false
        private set

    fun createConnection(info: EditorInfo): InputConnection? {
        if (!active || view.handle == 0L) return null
        info.inputType = inputType
        info.imeOptions = imeAction or EditorInfo.IME_FLAG_NO_FULLSCREEN or EditorInfo.IME_FLAG_NO_EXTRACT_UI
        info.initialSelStart = Selection.getSelectionStart(editable)
        info.initialSelEnd = Selection.getSelectionEnd(editable)
        EditorInfoCompat.setInitialSurroundingText(info, editable)
        return Connection().also { connection = it }
    }

    fun apply(
        op: Int, value: String, selStart: Int, selEnd: Int, compStart: Int, compEnd: Int,
        caretX: Float, caretY: Float, caretW: Float, caretH: Float, inputType: Int, imeAction: Int,
    ) {
        if (!NativeBridge.nativeAccept(view.handle, op, value, selStart, selEnd, compStart, compEnd)) return
        if (op == HIDE) {
            release()
            return
        }
        val opening = op == SHOW || !active
        val restart = opening || this.inputType != inputType || this.imeAction != imeAction
        active = true
        this.inputType = inputType
        this.imeAction = imeAction
        caret = Caret(caretX, caretY, caretW, caretH)
        val start = selStart.coerceIn(0, value.length)
        val end = selEnd.coerceIn(0, value.length)
        val hasComposition = compStart >= 0 && compEnd > compStart && compEnd <= value.length
        val state = EditingState(value, start, end, if (hasComposition) compStart else -1, if (hasComposition) compEnd else -1)
        if (snapshot() != state) {
            BaseInputConnection.removeComposingSpans(editable)
            if (editable.toString() != value) editable.replace(0, editable.length, value)
            Selection.setSelection(editable, start, end)
            if (hasComposition) {
                // BaseInputConnection uses the same composing marker as the real IME connection.
                object : BaseInputConnection(view, true) {
                    override fun getEditable(): Editable = this@TextInput.editable
                }.setComposingRegion(compStart, compEnd)
            }
        }
        lastSent = state
        if (restart) {
            connection = null
            deadKey = 0
            view.requestFocus()
            imm?.restartInput(view)
            if (op == SHOW) view.insetsController.show(WindowInsetsCompat.Type.ime())
        } else {
            reportEditingState(state)
            connection?.updateCursor()
        }
    }

    fun release(keepKeyboard: Boolean = false) {
        val hadClient = active
        active = false
        connection = null
        lastSent = null
        deadKey = 0
        if (hadClient) {
            BaseInputConnection.removeComposingSpans(editable)
            editable.clear()
            Selection.setSelection(editable, 0)
            if (!keepKeyboard) {
                view.insetsController.hide(WindowInsetsCompat.Type.ime())
                imm?.restartInput(view)
            }
        }
    }

    fun handleKey(event: KeyEvent): Boolean {
        if (!active || event.action != KeyEvent.ACTION_DOWN || event.isCtrlPressed ||
            event.isAltPressed || event.isMetaPressed
        ) return false
        when (event.keyCode) {
            KeyEvent.KEYCODE_DEL, KeyEvent.KEYCODE_FORWARD_DEL -> {
                deleteHardwareText(editable, backwards = event.keyCode == KeyEvent.KEYCODE_DEL)
                sendEditingState()
                return true
            }
        }
        val unicode = event.unicodeChar
        if (unicode and KeyCharacterMap.COMBINING_ACCENT != 0) {
            deadKey = unicode and KeyCharacterMap.COMBINING_ACCENT_MASK
            return true
        }
        if (unicode <= 0 || !Character.isValidCodePoint(unicode) || Character.isISOControl(unicode)) return false
        val combined = if (deadKey != 0) KeyCharacterMap.getDeadChar(deadKey, unicode) else 0
        val text = if (combined != 0) {
            String(Character.toChars(combined))
        } else {
            (if (deadKey == 0) "" else String(Character.toChars(deadKey))) + String(Character.toChars(unicode))
        }
        deadKey = 0
        insertHardwareText(editable, text)
        sendEditingState()
        return true
    }

    private fun snapshot() = EditingState(
        editable.toString(),
        Selection.getSelectionStart(editable).takeIf { it >= 0 } ?: editable.length,
        Selection.getSelectionEnd(editable).takeIf { it >= 0 } ?: editable.length,
        BaseInputConnection.getComposingSpanStart(editable),
        BaseInputConnection.getComposingSpanEnd(editable),
    )

    private fun sendEditingState() {
        if (!active || view.handle == 0L) return
        val state = snapshot()
        if (state == lastSent) return
        lastSent = state
        NativeBridge.nativeSync(view.handle, state.text, state.start, state.end, state.composeStart, state.composeEnd)
        reportEditingState(state)
        connection?.updateCursor()
    }

    private fun reportEditingState(state: EditingState) {
        imm?.updateSelection(view, state.start, state.end, state.composeStart, state.composeEnd)
        connection?.extractedRequest?.let {
            imm?.updateExtractedText(view, it.token, extracted(it))
        }
    }

    private fun extracted(request: ExtractedTextRequest?) = ExtractedText().apply {
        partialStartOffset = -1
        partialEndOffset = -1
        selectionStart = Selection.getSelectionStart(editable)
        selectionEnd = Selection.getSelectionEnd(editable)
        text = if (request != null && request.flags and InputConnection.GET_TEXT_WITH_STYLES != 0) {
            android.text.SpannedString(editable)
        } else {
            editable.toString()
        }
    }

    private inner class Connection : BaseInputConnection(view, true) {
        private var batchDepth = 0
        private var monitorCursor = false
        var extractedRequest: ExtractedTextRequest? = null
            private set

        private fun current() = active && connection === this && view.handle != 0L

        override fun getEditable(): Editable? = if (current()) this@TextInput.editable else null

        override fun beginBatchEdit(): Boolean {
            if (!current()) return false
            batchDepth++
            return true
        }

        override fun endBatchEdit(): Boolean {
            if (!current() || batchDepth == 0) return false
            batchDepth--
            if (batchDepth == 0) sendEditingState()
            return batchDepth > 0
        }

        override fun closeConnection() {
            super.closeConnection()
            if (current()) {
                batchDepth = 0
                sendEditingState()
                connection = null
            }
        }

        private inline fun edit(action: () -> Boolean): Boolean {
            if (!current()) return false
            val result = action()
            if (batchDepth == 0) sendEditingState()
            return result
        }

        override fun commitText(text: CharSequence, newCursorPosition: Int) =
            edit { super.commitText(text, newCursorPosition) }

        override fun setComposingText(text: CharSequence, newCursorPosition: Int) =
            edit { super.setComposingText(text, newCursorPosition) }

        override fun setComposingRegion(start: Int, end: Int) =
            edit { super.setComposingRegion(start, end) }

        override fun finishComposingText() = edit { super.finishComposingText() }

        override fun deleteSurroundingText(beforeLength: Int, afterLength: Int) =
            edit { super.deleteSurroundingText(beforeLength, afterLength) }

        override fun deleteSurroundingTextInCodePoints(beforeLength: Int, afterLength: Int) =
            edit { super.deleteSurroundingTextInCodePoints(beforeLength, afterLength) }

        override fun setSelection(start: Int, end: Int) = edit { super.setSelection(start, end) }

        override fun performEditorAction(editorAction: Int): Boolean {
            if (!current()) return false
            NativeBridge.nativeAction(view.handle)
            return true
        }

        override fun performContextMenuAction(id: Int): Boolean {
            if (!current()) return false
            val action = when (id) {
                android.R.id.copy -> 1
                android.R.id.cut -> 2
                android.R.id.paste -> 3
                android.R.id.selectAll -> 4
                else -> return false
            }
            NativeBridge.nativeContext(view.handle, action)
            return true
        }

        override fun sendKeyEvent(event: KeyEvent): Boolean = current() && view.dispatchKeyEvent(event)

        override fun getExtractedText(request: ExtractedTextRequest?, flags: Int): ExtractedText? {
            if (!current()) return null
            extractedRequest = request.takeIf { flags and GET_EXTRACTED_TEXT_MONITOR != 0 }
            return extracted(request)
        }

        override fun requestCursorUpdates(cursorUpdateMode: Int): Boolean {
            if (!current()) return false
            monitorCursor = cursorUpdateMode and CURSOR_UPDATE_MONITOR != 0
            if (cursorUpdateMode and CURSOR_UPDATE_IMMEDIATE != 0) reportCursor()
            return true
        }

        fun updateCursor() {
            if (current() && monitorCursor) reportCursor()
        }

        private fun reportCursor() {
            val state = snapshot()
            val matrix = Matrix()
            if (Build.VERSION.SDK_INT >= 29) {
                view.transformMatrixToGlobal(matrix)
            } else {
                val origin = IntArray(2)
                view.getLocationOnScreen(origin)
                matrix.setTranslate(origin[0].toFloat(), origin[1].toFloat())
            }
            val density = view.density
            val left = caret.x * density
            val top = caret.y * density
            val bottom = (caret.y + caret.height.coerceAtLeast(1f)) * density
            val builder = CursorAnchorInfo.Builder()
                .setMatrix(matrix)
                .setSelectionRange(state.start, state.end)
                .setInsertionMarkerLocation(left, top, bottom, bottom, CursorAnchorInfo.FLAG_HAS_VISIBLE_REGION)
            if (state.composeStart >= 0 && state.composeEnd > state.composeStart) {
                builder.setComposingText(state.composeStart, state.text.substring(state.composeStart, state.composeEnd))
            }
            if (state.end < state.text.length) {
                builder.addCharacterBounds(
                    state.end, left, top, (caret.x + caret.width.coerceAtLeast(1f)) * density, bottom,
                    CursorAnchorInfo.FLAG_HAS_VISIBLE_REGION,
                )
            }
            imm?.updateCursorAnchorInfo(view, builder.build())
        }
    }

    private data class EditingState(
        val text: String, val start: Int, val end: Int, val composeStart: Int, val composeEnd: Int,
    )

    private data class Caret(val x: Float, val y: Float, val width: Float, val height: Float)

    companion object {
        private const val SHOW = 1
        private const val HIDE = 3
    }
}

/** Hardware events insert at the selection. An IME commit can replace its composing region. */
internal fun insertHardwareText(editable: Editable, text: String) {
    val start = Selection.getSelectionStart(editable).takeIf { it >= 0 } ?: editable.length
    val end = Selection.getSelectionEnd(editable).takeIf { it >= 0 } ?: start
    val cursor = minOf(start, end)
    BaseInputConnection.removeComposingSpans(editable)
    editable.replace(cursor, maxOf(start, end), text)
    Selection.setSelection(editable, cursor + text.length)
}

/** Physical delete walks user-perceived characters in the UTF-16 editor buffer. */
internal fun deleteHardwareText(editable: Editable, backwards: Boolean) {
    val start = Selection.getSelectionStart(editable).takeIf { it >= 0 } ?: editable.length
    val end = Selection.getSelectionEnd(editable).takeIf { it >= 0 } ?: start
    BaseInputConnection.removeComposingSpans(editable)
    if (start != end) {
        editable.delete(minOf(start, end), maxOf(start, end))
        return
    }
    val graphemes = BreakIterator.getCharacterInstance().apply { setText(editable.toString()) }
    val inside = !graphemes.isBoundary(start)
    val before = if (inside || backwards) graphemes.preceding(start) else start
    val after = if (inside || !backwards) graphemes.following(start) else start
    if (before != BreakIterator.DONE && after != BreakIterator.DONE) editable.delete(before, after)
}
