/*
 * PollTest.kt
 * Family Connect (Android)
 *
 * The pure half of polls: the composer's rules (PollDraft) and what a
 * bubble draws (buildPollView, and its wiring through buildChatItems).
 *
 * Plain JUnit — nothing here touches Android, which is the point of
 * keeping both of them free of Compose.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.dto.PollCodec
import me.nettrash.familyconnect.data.net.dto.PollDto
import me.nettrash.familyconnect.data.net.dto.PollOptionDto
import org.junit.Test
import java.time.ZoneOffset

class PollTest {

    private companion object {
        const val ME = 7L
        const val PEER = 9L
        const val THIRD = 11L
        const val NOON = 1_786_795_200_000L
        val ZONE: ZoneOffset = ZoneOffset.UTC
    }

    private fun poll(
        pollSeq: Long = 88,
        closed: Boolean = false,
        vararg options: Pair<String, List<Long>>,
    ) = PollDto(
        pollSeq = pollSeq,
        closed = closed,
        options = options.mapIndexed { index, (text, votes) ->
            PollOptionDto(id = 5L + index, text = text, votes = votes)
        },
    )

    // -- The composer's rules ------------------------------------------------

    @Test
    fun aFreshDraftIsTwoEmptyOptionsAndCannotBeSent() {
        val draft = PollDraft()

        assertThat(draft.question).isEmpty()
        assertThat(draft.options).hasSize(2)
        assertThat(draft.isValid).isFalse()
        // Two is the floor, so there is nothing to take away yet.
        assertThat(draft.canRemoveOption).isFalse()
        assertThat(draft.canAddOption).isTrue()
    }

    @Test
    fun aQuestionAndTwoOptionsIsSendable() {
        val draft = PollDraft()
            .withQuestion("Pizza or pasta?")
            .withOption(0, "Pizza")
            .withOption(1, "Pasta")

        assertThat(draft.isValid).isTrue()
        assertThat(draft.sendableOptions).containsExactly("Pizza", "Pasta").inOrder()
    }

    @Test
    fun aPollWithNoQuestionIsRefused() {
        // `message_empty` applies to a poll with no question: unlike a
        // message carrying an attachment, a poll's body may not be empty.
        val draft = PollDraft()
            .withQuestion("   ")
            .withOption(0, "Pizza")
            .withOption(1, "Pasta")

        assertThat(draft.isValid).isFalse()
    }

    @Test
    fun blankOptionBoxesAreDroppedRatherThanRefused() {
        val draft = PollDraft()
            .withQuestion("Pizza or pasta?")
            .withOption(0, "Pizza")
            .withOption(1, "Pasta")
            .plusOption()

        // The third box is empty: somebody who added it and left it blank
        // meant a two-option poll, not an error.
        assertThat(draft.options).hasSize(3)
        assertThat(draft.sendableOptions).containsExactly("Pizza", "Pasta").inOrder()
        assertThat(draft.isValid).isTrue()
    }

    @Test
    fun oneRealOptionIsNotAQuestion() {
        val draft = PollDraft()
            .withQuestion("Pizza?")
            .withOption(0, "Pizza")

        assertThat(draft.sendableOptions).hasSize(1)
        assertThat(draft.isValid).isFalse()
    }

    @Test
    fun twoOptionsTheSameIgnoringCaseAreRefused() {
        // The server's rule, mirrored: a poll can never rename an option,
        // so two identical ones would be unfixable as well as unanswerable.
        val draft = PollDraft()
            .withQuestion("Pizza or pizza?")
            .withOption(0, "Pizza")
            .withOption(1, "PIZZA")

        assertThat(draft.isValid).isFalse()
    }

    @Test
    fun anOptionOverAHundredCharactersIsRefused() {
        val draft = PollDraft()
            .withQuestion("Long?")
            .withOption(0, "x".repeat(PollDraft.MAX_OPTION_CHARS + 1))
            .withOption(1, "Pasta")

        assertThat(draft.isValid).isFalse()
        // Exactly at the limit is fine — the cap is inclusive, like the server's.
        assertThat(draft.withOption(0, "x".repeat(PollDraft.MAX_OPTION_CHARS)).isValid).isTrue()
    }

    @Test
    fun theTenthOptionIsTheLast() {
        var draft = PollDraft()
        repeat(20) { draft = draft.plusOption() }

        assertThat(draft.options).hasSize(PollDraft.MAX_OPTIONS)
        assertThat(draft.canAddOption).isFalse()
    }

    @Test
    fun removingAnOptionNeverGoesBelowTwo() {
        val three = PollDraft()
            .withOption(0, "Pizza")
            .withOption(1, "Pasta")
            .plusOption()
            .withOption(2, "Sushi")

        val two = three.minusOption(1)
        assertThat(two.options).containsExactly("Pizza", "Sushi").inOrder()
        // And no further: one option is not a question.
        assertThat(two.minusOption(0).options).containsExactly("Pizza", "Sushi").inOrder()
    }

    // -- What a bubble draws -------------------------------------------------

    @Test
    fun optionsCarryTheirShareOfTheVoteAndWhoCastIt() {
        val view = buildPollView(
            poll = poll(
                options = arrayOf(
                    "Pizza" to listOf(PEER, ME),
                    "Pasta" to listOf(THIRD),
                    "Neither" to emptyList(),
                ),
            ),
            myUserId = ME,
            names = mapOf(ME to "Anna", PEER to "Ben", THIRD to "Junior"),
            familySize = 5,
        )

        assertThat(view.options.map { it.text })
            .containsExactly("Pizza", "Pasta", "Neither").inOrder()
        assertThat(view.options.map { it.count }).containsExactly(2, 1, 0).inOrder()
        // Share of the TOTAL (three votes), so two options at half draw
        // half each rather than both drawing full.
        assertThat(view.options[0].fraction).isWithin(0.001f).of(2f / 3f)
        assertThat(view.options[1].fraction).isWithin(0.001f).of(1f / 3f)
        assertThat(view.options[2].fraction).isEqualTo(0f)
        // Mine leads its option's faces, wherever it was cast.
        assertThat(view.options[0].isMine).isTrue()
        assertThat(view.options[0].voters.map { it.name })
            .containsExactly("Anna", "Ben").inOrder()
        assertThat(view.options[1].isMine).isFalse()
        assertThat(view.hasVoted).isTrue()
        assertThat(view.votedCount).isEqualTo(3)
        assertThat(view.familySize).isEqualTo(5)
        assertThat(view.closed).isFalse()
    }

    @Test
    fun aPollNobodyHasAnsweredDrawsNoBars() {
        val view = buildPollView(
            poll = poll(options = arrayOf("Pizza" to emptyList(), "Pasta" to emptyList())),
            myUserId = ME,
            names = emptyMap(),
            familySize = 4,
        )

        assertThat(view.options.map { it.fraction }).containsExactly(0f, 0f)
        assertThat(view.votedCount).isEqualTo(0)
        assertThat(view.hasVoted).isFalse()
    }

    @Test
    fun anUnknownVoterStillGetsAName() {
        // Same fallback the bubble's sender line uses — a voter with no
        // roster row must not render as an empty avatar.
        val view = buildPollView(
            poll = poll(options = arrayOf("Pizza" to listOf(404L), "Pasta" to emptyList())),
            myUserId = ME,
            names = emptyMap(),
        )

        assertThat(view.options[0].voters.single().name).isEqualTo("Member 404")
    }

    @Test
    fun aClosedPollSaysSo() {
        val view = buildPollView(
            poll = poll(closed = true, options = arrayOf("Pizza" to listOf(ME), "Pasta" to emptyList())),
            myUserId = ME,
            names = mapOf(ME to "Anna"),
            familySize = 2,
        )

        assertThat(view.closed).isTrue()
        // A closed poll keeps its result — the votes are still there.
        assertThat(view.options[0].count).isEqualTo(1)
    }

    // -- Through the list builder -------------------------------------------

    private fun entity(pollJson: String?) = MessageEntity(
        clientMsgId = "s1",
        serverId = 1,
        chatId = 42,
        senderId = PEER,
        body = "Pizza or pasta?",
        createdAt = NOON,
        status = MessageStatus.SENT,
        pollJson = pollJson,
    )

    @Test
    fun aStoredPollReachesTheItemItBelongsTo() {
        val stored = PollCodec.encode(
            poll(options = arrayOf("Pizza" to listOf(PEER), "Pasta" to emptyList())),
        )

        val item = buildChatItems(
            messagesNewestFirst = listOf(entity(stored)),
            isFamilyChat = true,
            myUserId = ME,
            memberNames = mapOf(PEER to "Ben"),
            nowMillis = NOON,
            zone = ZONE,
            familyMemberCount = 3,
        ).filterIsInstance<ChatListItem.MessageItem>().single()

        // The QUESTION is the body — the poll carries no copy of it.
        assertThat(item.entity.body).isEqualTo("Pizza or pasta?")
        val poll = checkNotNull(item.poll)
        assertThat(poll.options.map { it.text })
            .containsExactly("Pizza", "Pasta").inOrder()
        assertThat(poll.familySize).isEqualTo(3)
    }

    @Test
    fun anOrdinaryMessageHasNoPoll() {
        val item = buildChatItems(
            messagesNewestFirst = listOf(entity(pollJson = null)),
            isFamilyChat = true,
            myUserId = ME,
            memberNames = emptyMap(),
            nowMillis = NOON,
            zone = ZONE,
        ).filterIsInstance<ChatListItem.MessageItem>().single()

        assertThat(item.poll).isNull()
    }

    @Test
    fun anUnreadableStoredPollDegradesToAnOrdinaryMessage() {
        // A row a future build wrote in a shape this one cannot parse
        // must draw as the question it always was, never crash the list.
        val item = buildChatItems(
            messagesNewestFirst = listOf(entity(pollJson = "{not json")),
            isFamilyChat = true,
            myUserId = ME,
            memberNames = emptyMap(),
            nowMillis = NOON,
            zone = ZONE,
        ).filterIsInstance<ChatListItem.MessageItem>().single()

        assertThat(item.poll).isNull()
        assertThat(item.entity.body).isEqualTo("Pizza or pasta?")
    }
}
