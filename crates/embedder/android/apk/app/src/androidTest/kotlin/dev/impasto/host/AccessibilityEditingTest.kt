package dev.impasto.host

import android.content.Intent
import android.os.Bundle
import android.os.SystemClock
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AccessibilityEditingTest {
    @Test
    fun virtualEditorPublishesTextAndUtf16Selection() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val automation = instrumentation.uiAutomation
        val context = instrumentation.targetContext
        val intent = Intent(context, ImpastoActivity::class.java)
            .putExtra("script", "host-test/editor.js")
        ActivityScenario.launch<ImpastoActivity>(intent).use {
            val focused = awaitEditor({ automation.rootInActiveWindow }) { it.isFocused }
            assertTrue(focused.performAction(AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS))

            val replacement = "中😀e\u0301"
            val setText = Bundle().apply {
                putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, replacement)
            }
            val textEvent = automation.executeAndWaitForEvent({
                assertTrue(focused.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, setText))
            }, {
                it.eventType == AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED &&
                    it.packageName == context.packageName && it.text.any { value -> value.toString() == replacement }
            }, 10_000)
            assertEquals(replacement.length, textEvent.addedCount)

            val selection = Bundle().apply {
                putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, 1)
                putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, 3)
            }
            val updated = awaitEditor({ automation.rootInActiveWindow }) { it.text?.toString() == replacement }
            val selectionEvent = automation.executeAndWaitForEvent({
                assertTrue(updated.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, selection))
            }, {
                it.eventType == AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED &&
                    it.packageName == context.packageName && it.fromIndex == 1 && it.toIndex == 3
            }, 10_000)
            assertEquals(replacement.length, selectionEvent.itemCount)
            awaitEditor({ automation.rootInActiveWindow }) { it.textSelectionStart == 1 && it.textSelectionEnd == 3 }
        }
    }

    private fun awaitEditor(
        root: () -> AccessibilityNodeInfo?,
        predicate: (AccessibilityNodeInfo) -> Boolean = { true },
    ): AccessibilityNodeInfo {
        val deadline = SystemClock.uptimeMillis() + 10_000
        while (SystemClock.uptimeMillis() < deadline) {
            editor(root(), predicate)?.let { return it }
            SystemClock.sleep(50)
        }
        throw AssertionError("The virtual editor did not expose the expected state")
    }

    private fun editor(
        node: AccessibilityNodeInfo?,
        predicate: (AccessibilityNodeInfo) -> Boolean,
    ): AccessibilityNodeInfo? {
        if (node == null) return null
        if (node.isEditable && node.className == "android.widget.EditText" && predicate(node)) return node
        for (index in 0 until node.childCount) {
            editor(node.getChild(index), predicate)?.let { return it }
        }
        return null
    }
}
