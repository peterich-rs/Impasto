package dev.impasto.host

import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.impasto.host.WindowLifecycle.State
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class WindowLifecycleTest {
    @Test
    fun onlyAnAttachedWindowCanBeActive() {
        val states = mutableListOf<State>()
        val lifecycle = WindowLifecycle(true, initiallyAttached = false) { states.add(it) }

        lifecycle.activityChanged(State.RESUMED)
        lifecycle.attachmentChanged(true)
        lifecycle.attachmentChanged(false)
        lifecycle.focusChanged(false)
        lifecycle.attachmentChanged(true)

        assertEquals(listOf(State.HIDDEN, State.RESUMED, State.HIDDEN, State.INACTIVE), states)
    }

    @Test
    fun resumedActivityBecomesActiveWhenItsWindowGainsFocus() {
        val states = mutableListOf<State>()
        val lifecycle = WindowLifecycle(false) { states.add(it) }

        lifecycle.activityChanged(State.RESUMED)
        lifecycle.focusChanged(true)
        lifecycle.focusChanged(false)
        lifecycle.activityChanged(State.INACTIVE)
        lifecycle.focusChanged(true)

        assertEquals(listOf(State.INACTIVE, State.RESUMED, State.INACTIVE), states)
    }

    @Test
    fun focusBeforeResumeOrWhileStoppedDoesNotPublishAnActiveWindow() {
        val states = mutableListOf<State>()
        val lifecycle = WindowLifecycle(false) { states.add(it) }

        lifecycle.focusChanged(true)
        lifecycle.activityChanged(State.RESUMED)
        lifecycle.activityChanged(State.INACTIVE)
        lifecycle.activityChanged(State.HIDDEN)
        lifecycle.focusChanged(false)
        lifecycle.focusChanged(true)
        lifecycle.focusChanged(false)
        lifecycle.activityChanged(State.RESUMED)

        assertEquals(listOf(State.RESUMED, State.INACTIVE, State.HIDDEN, State.INACTIVE), states)
    }
}
