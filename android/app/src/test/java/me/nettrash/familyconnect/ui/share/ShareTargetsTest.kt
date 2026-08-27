/*
 * ShareTargetsTest.kt
 * Family Connect (Android)
 *
 * The share picker's one rule: family or direct, NEVER the assistant's
 * private thread (`kind == "ai"`) — the first "ai" comparison this app
 * makes, so it gets its own pin. Order is the DAO's and must survive the
 * filter: family first, then recency.
 */

package me.nettrash.familyconnect.ui.share

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.db.ChatEntity
import org.junit.Test

class ShareTargetsTest {

    private fun chat(id: Long, kind: String) = ChatEntity(
        id = id,
        kind = kind,
        peerUserId = if (kind == "direct") id * 10 else null,
        title = "chat $id",
        unreadCount = 0,
        myLastReadId = null,
        peerLastReadId = null,
        lastMessageBody = null,
        lastMessageAt = null,
        lastMessageSenderId = null,
    )

    @Test
    fun `the assistant's thread is never a share target`() {
        val chats = listOf(chat(1, "family"), chat(2, "ai"), chat(3, "direct"))
        assertThat(shareTargets(chats).map { it.id }).containsExactly(1L, 3L).inOrder()
    }

    @Test
    fun `family and direct chats pass through in the order the DAO gave`() {
        val chats = listOf(chat(1, "family"), chat(5, "direct"), chat(3, "direct"))
        assertThat(shareTargets(chats)).isEqualTo(chats)
    }

    @Test
    fun `a roster of only the assistant is empty`() {
        assertThat(shareTargets(listOf(chat(2, "ai")))).isEmpty()
    }
}
