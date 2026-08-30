/*
 * BlockedMessageRuleTest.kt
 * Family Connect (Android)
 *
 * What a blocked member's content looks like, and — mostly — what does NOT
 * change when somebody is blocked. A cross-platform contract:
 * BlockPresentationTests.swift and MessageGroupingTests.swift pin the same
 * vectors on iOS, and two apps that disagree about this disagree in public.
 *
 * The load-bearing negative is run GROUPING: hiding a row must not merge
 * the runs around it, or the next visible message from somebody else
 * silently loses its sender caption.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import org.junit.Test

class BlockedMessageRuleTest {

    private val me = 7L
    private val blocked = 11L
    private val other = 12L

    private fun message(
        id: Long,
        senderId: Long,
        at: Long = 1_700_000_000_000,
        replySenderId: Long? = null,
        replyParentSenderId: Long? = null,
    ) = MessageEntity(
        clientMsgId = "m$id",
        serverId = id,
        chatId = 42,
        senderId = senderId,
        body = "hello",
        createdAt = at,
        status = MessageStatus.SENT,
        replyToMessageId = replySenderId?.let { 900 + id },
        replySenderId = replySenderId,
        replyExcerpt = replySenderId?.let { "quoted text" },
        replyParentSenderId = replyParentSenderId,
        replyParentExcerpt = replyParentSenderId?.let { "grandparent text" },
    )

    // MARK: - The predicate

    @Test
    fun aBlockedSendersRowIsHidden() {
        assertThat(BlockedMessageRule.isHidden(blocked, me, setOf(blocked))).isTrue()
    }

    /** Keyed on the AUTHOR, not on the set being non-empty. */
    @Test
    fun somebodyElsesRowIsNotHidden() {
        assertThat(BlockedMessageRule.isHidden(other, me, setOf(blocked))).isFalse()
        assertThat(BlockedMessageRule.isHidden(other, me, emptySet())).isFalse()
    }

    /**
     * Blocking yourself is refused by the server, so this guards a corrupt
     * store rather than a real case — but a chat that hid your own
     * messages would be the most alarming possible bug.
     */
    @Test
    fun myOwnRowIsNeverHidden() {
        assertThat(BlockedMessageRule.isHidden(me, me, setOf(me))).isFalse()
    }

    @Test
    fun aQuoteWithNoAuthorIsNotHidden() {
        assertThat(BlockedMessageRule.isQuoteHidden(null, me, setOf(blocked))).isFalse()
    }

    /** Integers are not presence: only the identities go. */
    @Test
    fun onlyBlockedVotersAreUndrawable() {
        val voters = listOf(me, blocked, other)
        assertThat(BlockedMessageRule.drawableVoters(voters, me, setOf(blocked)))
            .containsExactly(me, other).inOrder()
    }

    // MARK: - Through buildChatItems

    private fun items(messagesNewestFirst: List<MessageEntity>, blockedIds: Set<Long>) =
        buildChatItems(
            messagesNewestFirst = messagesNewestFirst,
            isFamilyChat = true,
            myUserId = me,
            memberNames = mapOf(blocked to "Blocked One", other to "Someone"),
            nowMillis = 1_700_000_100_000,
            blockedUserIds = blockedIds,
        ).filterIsInstance<ChatListItem.MessageItem>()

    @Test
    fun theRowStaysInTheListAndIsFlagged() {
        // Newest first, so this is [other, blocked].
        val built = items(
            listOf(message(2, other), message(1, blocked)),
            setOf(blocked),
        )
        // TWO items: the row is flagged, never dropped. A client that
        // filtered history would freeze its own read marker at the id
        // before the hidden one.
        assertThat(built).hasSize(2)
        assertThat(built.first { it.entity.serverId == 1L }.isHiddenByBlock).isTrue()
        assertThat(built.first { it.entity.serverId == 2L }.isHiddenByBlock).isFalse()
    }

    @Test
    fun aHiddenRowDrawsNoSenderName() {
        val built = items(listOf(message(1, blocked)), setOf(blocked))
        assertThat(built.single().showSenderName).isFalse()
    }

    /**
     * THE regression this file exists for. Two messages from the blocked
     * member, then one from somebody else: hiding must not merge the runs,
     * so the visible message still starts its own run and keeps its
     * caption. Filtering hidden rows out before computing run boundaries
     * is the natural implementation and it breaks exactly this.
     */
    @Test
    fun hidingDoesNotMergeTheRunsAroundIt() {
        val minute = 60_000L
        val base = 1_700_000_000_000
        // Newest first: other(3), blocked(2), blocked(1).
        val withBlocks = items(
            listOf(
                message(3, other, at = base + 2 * minute),
                message(2, blocked, at = base + minute),
                message(1, blocked, at = base),
            ),
            setOf(blocked),
        )
        val without = items(
            listOf(
                message(3, other, at = base + 2 * minute),
                message(2, blocked, at = base + minute),
                message(1, blocked, at = base),
            ),
            emptySet(),
        )

        fun runFlags(list: List<ChatListItem.MessageItem>) =
            list.map { it.entity.serverId to (it.isRunStart to it.isRunEnd) }

        // Grouping is IDENTICAL with and without the block.
        assertThat(runFlags(withBlocks)).isEqualTo(runFlags(without))
        // And the unblocked sender still gets its caption.
        assertThat(withBlocks.first { it.entity.serverId == 3L }.showSenderName).isTrue()
    }

    // MARK: - Quotes, which mask independently

    @Test
    fun theTwoQuoteLevelsMaskIndependently() {
        // A reply BY an unblocked member TO a blocked one, whose own
        // parent is a third person.
        val onlyReplyBlocked = items(
            listOf(message(1, other, replySenderId = blocked, replyParentSenderId = other)),
            setOf(blocked),
        ).single()
        assertThat(onlyReplyBlocked.isHiddenByBlock).isFalse()
        assertThat(onlyReplyBlocked.isReplyHidden).isTrue()
        assertThat(onlyReplyBlocked.isParentHidden).isFalse()

        // And the other way round: a readable reply quoting a blocked
        // grandparent.
        val onlyParentBlocked = items(
            listOf(message(1, other, replySenderId = other, replyParentSenderId = blocked)),
            setOf(blocked),
        ).single()
        assertThat(onlyParentBlocked.isReplyHidden).isFalse()
        assertThat(onlyParentBlocked.isParentHidden).isTrue()
    }
}
