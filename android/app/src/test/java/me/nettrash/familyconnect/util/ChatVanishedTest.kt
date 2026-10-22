/*
 * ChatVanishedTest.kt
 * Family Connect (Android)
 *
 * The signal a thread leaves on (ThreadViewModel.chatGone): the chat row it
 * belongs to was seen and then went — a group left, removed from or deleted
 * (docs/protocol.md, "Groups") — while a first null, Room not having
 * answered yet, must never close the screen.
 */

package me.nettrash.familyconnect.util

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.test.runTest
import org.junit.Test

class ChatVanishedTest {

    @Test
    fun `a row that was seen and then went is gone`() = runTest {
        assertThat(flowOf(null, "group", "group renamed", null).goneAfterSeen().toList())
            .containsExactly(false, true).inOrder()
    }

    @Test
    fun `a first null is Room not having answered, never a departure`() = runTest {
        assertThat(flowOf<String?>(null, null).goneAfterSeen().toList()).containsExactly(false)
        assertThat(flowOf(null, "group").goneAfterSeen().toList()).containsExactly(false)
    }

    /** One-way: a screen already leaving is not called back by a row reappearing. */
    @Test
    fun `gone stays gone`() = runTest {
        assertThat(flowOf("group", null, "group").goneAfterSeen().toList())
            .containsExactly(false, true).inOrder()
    }
}
