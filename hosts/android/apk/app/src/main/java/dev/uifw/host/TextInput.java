package dev.uifw.host;

import android.graphics.Matrix;
import android.os.Build;
import android.text.Selection;
import android.view.KeyEvent;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.CompletionInfo;
import android.view.inputmethod.CursorAnchorInfo;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.ExtractedText;
import android.view.inputmethod.ExtractedTextRequest;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;

/**
 * The input connection for one focused field. The session remains the source
 * of the string. Each connection call is forwarded as an edit, and
 * {@link #apply} writes the session's state back.
 */
final class TextInput {
    private final HostView view;
    private final android.text.SpannableStringBuilder editable = new android.text.SpannableStringBuilder();
    private final InputMethodManager imm;
    private boolean client;
    private int batch;
    private int inputType = EditorInfo.TYPE_CLASS_TEXT;
    private int imeAction = EditorInfo.IME_ACTION_DONE;
    private float caretX;
    private float caretY;
    private float caretW;
    private float caretH;
    private boolean monitorCursor;
    private ExtractedTextRequest extractRequest;
    private final Matrix cursorMatrix = new Matrix();

    TextInput(HostView view) {
        this.view = view;
        this.imm = (InputMethodManager) view.getContext().getSystemService(android.content.Context.INPUT_METHOD_SERVICE);
    }

    boolean clientActive() {
        return client;
    }

    InputConnection createConnection(EditorInfo outAttrs) {
        if (!client) {
            return null;
        }
        outAttrs.inputType = inputType;
        outAttrs.imeOptions = imeAction
                | EditorInfo.IME_FLAG_NO_FULLSCREEN
                | EditorInfo.IME_FLAG_NO_EXTRACT_UI;
        outAttrs.initialSelStart = Selection.getSelectionStart(editable);
        outAttrs.initialSelEnd = Selection.getSelectionEnd(editable);
        return new Connection(outAttrs);
    }

    void apply(
            int op,
            String text,
            int selStart,
            int selEnd,
            int compStart,
            int compEnd,
            float caretX,
            float caretY,
            float caretW,
            float caretH,
            int inputType,
            int imeAction) {
        if (!nativeAccept(op, text, selStart, selEnd, compStart, compEnd)) {
            return;
        }
        if (op == 3) {
            client = false;
            this.caretW = 0f;
            this.caretH = 0f;
            imm.hideSoftInputFromWindow(view.getWindowToken(), 0);
            imm.restartInput(view);
            return;
        }
        boolean opening = !client;
        client = true;
        this.inputType = inputType;
        this.imeAction = imeAction;
        this.caretX = caretX;
        this.caretY = caretY;
        this.caretW = caretW;
        this.caretH = caretH;
        boolean textChanged = text != null && !text.contentEquals(editable);
        editable.clear();
        if (text != null) {
            editable.append(text);
        }
        int end = editable.length();
        int start = clamp(selStart, end);
        int stop = clamp(selEnd, end);
        if (stop < start) {
            int swap = start;
            start = stop;
            stop = swap;
        }
        Selection.setSelection(editable, start, stop);
        if (compStart >= 0 && compEnd > compStart && compEnd <= end) {
            markComposing(compStart, compEnd);
        } else {
            markComposing(0, 0);
        }
        if (opening || textChanged) {
            view.requestFocus();
            imm.restartInput(view);
            if (op == 1) {
                imm.showSoftInput(view, 0);
            }
        } else {
            imm.updateSelection(
                    view,
                    Selection.getSelectionStart(editable),
                    Selection.getSelectionEnd(editable),
                    compStart,
                    compEnd);
        }
        updateCursor();
        if (extractRequest != null) {
            imm.updateExtractedText(view, extractRequest.token, extracted(extractRequest));
        }
    }

    void deleteSurrounding(int before, int after) {
        nativeDelete(before, after);
    }

    private void updateCursor() {
        if (!monitorCursor || imm == null) {
            return;
        }
        float density = view.density();
        float left = caretX * density;
        float top = caretY * density;
        float right = left + Math.max(caretW, 1f) * density;
        float bottom = top + Math.max(caretH, 1f) * density;
        // Bounds are view-local pixels. Builder.build throws if a position is
        // set and this local-to-screen matrix is missing.
        cursorMatrix.reset();
        if (Build.VERSION.SDK_INT >= 29) {
            view.transformMatrixToGlobal(cursorMatrix);
        } else {
            int[] location = new int[2];
            view.getLocationOnScreen(location);
            cursorMatrix.setTranslate(location[0], location[1]);
        }
        CursorAnchorInfo.Builder builder = new CursorAnchorInfo.Builder();
        builder.setMatrix(cursorMatrix);
        builder.setSelectionRange(
                Selection.getSelectionStart(editable), Selection.getSelectionEnd(editable));
        int compStart = BaseInputConnection.getComposingSpanStart(editable);
        int compEnd = BaseInputConnection.getComposingSpanEnd(editable);
        if (compStart >= 0 && compEnd > compStart) {
            builder.setComposingText(compStart, editable.subSequence(compStart, compEnd));
        }
        int caret = Selection.getSelectionEnd(editable);
        if (caret >= 0) {
            builder.setInsertionMarkerLocation(
                    left, top, bottom, bottom, CursorAnchorInfo.FLAG_HAS_VISIBLE_REGION);
            builder.addCharacterBounds(
                    caret, left, top, right, bottom, CursorAnchorInfo.FLAG_HAS_VISIBLE_REGION);
        }
        imm.updateCursorAnchorInfo(view, builder.build());
    }

    private ExtractedText extracted(ExtractedTextRequest request) {
        ExtractedText extracted = new ExtractedText();
        extracted.startOffset = 0;
        extracted.partialStartOffset = -1;
        extracted.partialEndOffset = -1;
        extracted.selectionStart = Selection.getSelectionStart(editable);
        extracted.selectionEnd = Selection.getSelectionEnd(editable);
        extracted.text = request != null && (request.flags & InputConnection.GET_TEXT_WITH_STYLES) != 0
                ? editable
                : editable.toString();
        return extracted;
    }

    private void markComposing(int start, int end) {
        new BaseInputConnection(view, false) {
            @Override
            public android.text.Editable getEditable() {
                return editable;
            }
        }.setComposingRegion(start, end);
    }

    private static int clamp(int value, int length) {
        if (value < 0) {
            return 0;
        }
        return Math.min(value, length);
    }

    private final class Connection extends BaseInputConnection {
        Connection(EditorInfo info) {
            super(view, true);
        }

        @Override
        public android.text.Editable getEditable() {
            return editable;
        }

        @Override
        public boolean beginBatchEdit() {
            batch += 1;
            if (batch == 1) {
                nativeBegin();
            }
            return super.beginBatchEdit();
        }

        @Override
        public boolean endBatchEdit() {
            boolean result = super.endBatchEdit();
            if (batch > 0) {
                batch -= 1;
                if (batch == 0) {
                    nativeEnd();
                }
            }
            return result;
        }

        @Override
        public boolean commitText(CharSequence text, int newCursorPosition) {
            boolean result = super.commitText(text, newCursorPosition);
            nativeCommit(text == null ? "" : text.toString());
            updateCursor();
            return result;
        }

        @Override
        public boolean setComposingText(CharSequence text, int newCursorPosition) {
            boolean result = super.setComposingText(text, newCursorPosition);
            nativeComposing(text == null ? "" : text.toString());
            updateCursor();
            return result;
        }

        @Override
        public boolean setComposingRegion(int start, int end) {
            boolean result = super.setComposingRegion(start, end);
            int from = Math.min(start, end);
            int to = Math.max(start, end);
            if (from < 0 || to > editable.length() || from >= to) {
                nativeComposing("");
            } else {
                nativeComposing(editable.subSequence(from, to).toString());
            }
            return result;
        }

        @Override
        public boolean finishComposingText() {
            boolean result = super.finishComposingText();
            nativeFinish();
            return result;
        }

        @Override
        public boolean deleteSurroundingText(int beforeLength, int afterLength) {
            boolean result = super.deleteSurroundingText(beforeLength, afterLength);
            nativeDelete(Math.max(0, beforeLength), Math.max(0, afterLength));
            return result;
        }

        @Override
        public boolean deleteSurroundingTextInCodePoints(int beforeLength, int afterLength) {
            boolean result = super.deleteSurroundingTextInCodePoints(beforeLength, afterLength);
            nativeDelete(Math.max(0, beforeLength), Math.max(0, afterLength));
            return result;
        }

        @Override
        public boolean setSelection(int start, int end) {
            return super.setSelection(start, end);
        }

        @Override
        public boolean performEditorAction(int editorAction) {
            nativeAction();
            return true;
        }

        @Override
        public boolean performContextMenuAction(int id) {
            if (id == android.R.id.copy) {
                nativeContext(1);
                return true;
            }
            if (id == android.R.id.cut) {
                nativeContext(2);
                return true;
            }
            if (id == android.R.id.paste) {
                nativeContext(3);
                return true;
            }
            if (id == android.R.id.selectAll) {
                nativeContext(4);
                return true;
            }
            return false;
        }

        @Override
        public boolean sendKeyEvent(KeyEvent event) {
            return view.dispatchKeyEvent(event);
        }

        @Override
        public ExtractedText getExtractedText(ExtractedTextRequest request, int flags) {
            extractRequest = (flags & GET_EXTRACTED_TEXT_MONITOR) != 0 ? request : null;
            return extracted(request);
        }

        @Override
        public boolean requestCursorUpdates(int cursorUpdateMode) {
            monitorCursor = (cursorUpdateMode & CURSOR_UPDATE_IMMEDIATE) != 0
                    || (cursorUpdateMode & CURSOR_UPDATE_MONITOR) != 0;
            if (monitorCursor) {
                updateCursor();
            }
            return true;
        }

        @Override
        public boolean commitCompletion(CompletionInfo text) {
            return false;
        }
    }

    private static native boolean nativeAccept(
            int op,
            String text,
            int selStart,
            int selEnd,
            int compStart,
            int compEnd);

    private static native void nativeBegin();

    private static native void nativeEnd();

    private static native void nativeCommit(String text);

    private static native void nativeComposing(String text);

    private static native void nativeFinish();

    private static native void nativeDelete(int before, int after);

    private static native void nativeAction();

    private static native void nativeContext(int kind);
}
