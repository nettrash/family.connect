/*
 * ChatItemsReuseTest.kt
 * Family Connect (Android)
 *
 * reuseUnchangedItems (ChatItems.kt): a rebuilt window hands the rows that
 * did not change back as the INSTANCES the list already drew — so Compose,
 * which compares these unstable rows by identity, skips their bubbles — and
 * never swaps a row for one that is not equal to it.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.dto.ReactionDto
import me.nettrash.familyconnect.data.net.dto.ReactionsCodec
import org.junit.Test
import java.time.ZoneId

class ChatItemsReuseTest {

    private companion object {
        val ZONE: ZoneId = ZoneId.of("UTC")
        const val NOON = 1_755_000_000_000L
        const val ME = 7L
        const val PEER = 9L
    }

    private fun row(serverId: Long, minutesAgo: Long, reactionsJson: String? = null) = MessageEntity(
        clientMsgId = "s$serverId",
        serverId = serverId,
        chatId = 42,
        senderId = if (serverId % 2 == 0L) ME else PEER,
        body = "message $serverId",
        createdAt = NOON - minutesAgo * 60_000,
        status = MessageStatus.SENT,
        reactionsJson = reactionsJson,
    )

    private fun build(rows: List<MessageEntity>) = buildChatItems(
        messagesNewestFirst = rows,
        isFamilyChat = true,
        myUserId = ME,
        memberNames = mapOf(PEER to "Ann"),
        nowMillis = NOON,
        zone = ZONE,
    )

    /** The same rows as Room hands them over on the next emission: equal, but new objects. */
    private fun reread(rows: List<MessageEntity>) = rows.map { it.copy() }

    @Test
    fun anUnchangedWindowIsHandedBackAsTheListAlreadyDrawn() {
        val rows = listOf(row(3, 1), row(2, 2), row(1, 3))
        val first = build(rows)
        val second = reuseUnchangedItems(first, build(reread(rows)))
        assertThat(second).isSameInstanceAs(first)
    }

    @Test
    fun onlyTheRowThatChangedIsANewInstance() {
        val rows = listOf(row(3, 1), row(2, 2), row(1, 3))
        val first = build(rows)
        val reacted = reread(rows).toMutableList()
        reacted[1] = reacted[1].copy(reactionsJson = ReactionsCodec.encode(listOf(ReactionDto(PEER, "👍"))))
        val second = reuseUnchangedItems(first, build(reacted))

        assertThat(second).isNotSameInstanceAs(first)
        assertThat(second).isEqualTo(build(reacted))
        val firstByKey = first.associateBy { it.key }
        for (item in second) {
            val held = firstByKey.getValue(item.key)
            if (item.key == "s2") {
                assertThat(item).isNotSameInstanceAs(held)
                assertThat((item as ChatListItem.MessageItem).reactionChips).hasSize(1)
            } else {
                assertThat(item).isSameInstanceAs(held)
            }
        }
    }

    @Test
    fun aNewMessageArrivingKeepsEveryOlderRowsInstance() {
        val rows = listOf(row(2, 2), row(1, 3))
        val first = build(rows)
        val grown = listOf(row(4, 0)) + reread(rows)
        val second = reuseUnchangedItems(first, build(grown))

        assertThat(second).isEqualTo(build(grown))
        val firstByKey = first.associateBy { it.key }
        // The row that used to be newest changes (it no longer ends the
        // run, so it loses its timestamp) and is rebuilt; anything equal
        // to what was drawn is the drawn instance.
        for (item in second) {
            val held = firstByKey[item.key] ?: continue
            if (held == item) assertThat(item).isSameInstanceAs(held)
        }
        assertThat(second.first { it.key == "s1" }).isSameInstanceAs(first.first { it.key == "s1" })
    }

    @Test
    fun nothingToReuseFromHandsTheNewListBack() {
        val built = build(listOf(row(1, 1)))
        assertThat(reuseUnchangedItems(emptyList(), built)).isSameInstanceAs(built)
        assertThat(reuseUnchangedItems(built, emptyList())).isEmpty()
    }
}
