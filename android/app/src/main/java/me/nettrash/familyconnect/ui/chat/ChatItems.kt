/*
 * ChatItems.kt
 * Family Connect (Android)
 *
 * Pure mapping from the DAO's newest-first message list to what the
 * reverseLayout LazyColumn renders. Kept free of Compose and Android so
 * ChatViewModelTest can pin the grouping rules directly:
 *
 *   - DateSeparator after the last (oldest) message of each day — in
 *     reverseLayout that draws the pill *above* the day's first bubble.
 *   - NewMessagesDivider directly after (= visually above) the oldest
 *     message the reader has not seen, when the screen opened anchored
 *     there. Its id and count come from OpenAnchor, captured ONCE at
 *     open — this builder never derives them, because the count is
 *     zeroed the instant the reader reaches the bottom and a derived
 *     divider would vanish mid-read. Inserted BELOW the day pill in
 *     list order so the two read top-down as [date][N new][bubble].
 *   - showSenderName: family chat only, not my own messages, and only
 *     on the first message of a same-sender run (previous OLDER item
 *     differs in sender or day).
 *   - showTimestamp: on the last message of a same-sender same-minute
 *     run — consecutive rapid-fire messages share one timestamp.
 *   - isRunStart/isRunEnd: position within a same-sender same-day run;
 *     in reverseLayout the OLDER neighbor renders above, so isRunStart
 *     is the visually-top bubble of the run and isRunEnd the bottom one.
 *     Drives the bubble corner tightening in ChatScreen.
 *   - reactionChips: the row's reactionsJson aggregated into
 *     (emoji, count, includesMe) chips in first-seen order.
 *   - poll: the row's pollJson resolved into options with their share of
 *     the vote and the people behind it (buildPollView).
 *   - buildReactionDetails: the same raw reactions resolved to display
 *     names for the who-reacted popup (chip long-press).
 *
 * iOS counterpart: ios/FamilyConnect/UI/Chat/ChatItems.swift
 */

package me.nettrash.familyconnect.ui.chat

import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.net.dto.PollCodec
import me.nettrash.familyconnect.ui.components.AttachmentAlbum
import me.nettrash.familyconnect.data.net.dto.PollDto
import me.nettrash.familyconnect.data.net.dto.ReactionDto
import me.nettrash.familyconnect.data.net.dto.ReactionsCodec
import me.nettrash.familyconnect.util.TimeFormat
import java.time.ZoneId

/**
 * The quick-set the long-press sheet offers. Client UI only — the
 * protocol accepts any emoji ≤ 32 bytes, so other clients' choices
 * outside this list still render as chips.
 */
val QUICK_REACTIONS = listOf("❤️", "👍", "👎", "😂", "😮", "😢")

/**
 * The reaction a bubble double-tap toggles (the Tapback-heart idiom).
 * Kept inside the quick set so the capsule shows it selected. Same
 * value on iOS.
 */
const val DOUBLE_TAP_REACTION = "❤️"

/** One aggregated reaction chip under a bubble. */
data class ReactionChip(
    val emoji: String,
    val count: Int,
    /** My reaction is in this group — the chip highlights, tap removes. */
    val includesMe: Boolean,
)

/**
 * Aggregate raw per-user reactions into ordered chips: one per emoji in
 * FIRST-SEEN order (stable while people pile onto existing emojis).
 */
fun buildReactionChips(reactions: List<ReactionDto>, myUserId: Long): List<ReactionChip> {
    if (reactions.isEmpty()) return emptyList()
    val byEmoji = LinkedHashMap<String, Pair<Int, Boolean>>()
    for (reaction in reactions) {
        val (count, mine) = byEmoji[reaction.emoji] ?: (0 to false)
        byEmoji[reaction.emoji] = (count + 1) to (mine || reaction.userId == myUserId)
    }
    return byEmoji.map { (emoji, aggregate) ->
        ReactionChip(emoji = emoji, count = aggregate.first, includesMe = aggregate.second)
    }
}

/** One emoji's reactors, resolved to display names, for the who-reacted popup. */
data class ReactionDetail(
    val emoji: String,
    /** Reaction order, except my own entry renders as "You" and leads. */
    val names: List<String>,
)

/**
 * Resolve raw per-user reactions into per-emoji name lists for the
 * who-reacted popup (chip long-press). Emojis appear in FIRST-SEEN order
 * — the same order buildReactionChips renders the chips — and within an
 * emoji names keep reaction order, except my own entry, which shows as
 * "You" and moves to the front. Unknown user ids fall back to
 * "Member <id>", matching the sender-name fallback in the bubble.
 */
fun buildReactionDetails(
    reactions: List<ReactionDto>,
    names: Map<Long, String>,
    myUserId: Long,
    /**
     * Display text for my own entry and for a user id the roster does not
     * know. Passed in rather than resourced here because this file stays
     * Compose- and Android-free (see the header); the English defaults
     * keep the pure tests pinning the ordering rules unchanged.
     */
    youLabel: String = "You",
    memberFallback: (Long) -> String = { "Member $it" },
): List<ReactionDetail> {
    if (reactions.isEmpty()) return emptyList()
    val othersByEmoji = LinkedHashMap<String, MutableList<String>>()
    val mine = HashSet<String>()
    for (reaction in reactions) {
        val others = othersByEmoji.getOrPut(reaction.emoji) { mutableListOf() }
        if (reaction.userId == myUserId) {
            mine += reaction.emoji
        } else {
            others += names[reaction.userId] ?: memberFallback(reaction.userId)
        }
    }
    return othersByEmoji.map { (emoji, others) ->
        ReactionDetail(
            emoji = emoji,
            names = if (emoji in mine) listOf(youLabel) + others else others,
        )
    }
}

/** One person who voted, as an option row draws them. */
data class PollVoter(
    val userId: Long,
    /** Their display name — the initials on the avatar come from it. */
    val name: String,
)

/** One option of a poll, resolved for drawing. */
data class PollOptionView(
    val id: Long,
    val text: String,
    val count: Int,
    /**
     * Share of all votes cast, 0..1 — the width of the bar. Of the TOTAL
     * rather than of the leader, so two options at 50% draw half each
     * and a poll nobody has answered draws no bar at all.
     */
    val fraction: Float,
    /** I hold this option — the row is marked, and tapping it clears. */
    val isMine: Boolean,
    /** Who chose it, in vote order, except mine, which leads. */
    val voters: List<PollVoter>,
)

/**
 * A poll as a bubble draws it (docs/protocol.md, "Polls").
 *
 * The QUESTION is not here: it is the message body, which the bubble
 * already renders.
 */
data class PollView(
    val options: List<PollOptionView>,
    val closed: Boolean,
    /** People who have voted, each counted once — a vote is one option. */
    val votedCount: Int,
    /** Live members of the family, for the "3 of 5 voted" footer; 0 = unknown. */
    val familySize: Int,
    /** Whether I have voted at all. */
    val hasVoted: Boolean,
)

/**
 * Resolve a stored poll into what a bubble draws.
 *
 * Names come from the same roster map the sender line uses, so a deleted
 * account's vote is still attributed (its name is already the client's
 * own translation of the placeholder — see MemberNames), and an unknown
 * id falls back to "Member <id>" exactly as a bubble's sender does.
 */
fun buildPollView(
    poll: PollDto,
    myUserId: Long,
    names: Map<Long, String>,
    familySize: Int = 0,
): PollView {
    val total = poll.totalVotes
    return PollView(
        options = poll.options.map { option ->
            val mine = option.votes.any { it == myUserId }
            // Mine first: on a busy option it is the one face the reader
            // is looking for, and it must not fall off the end of the row.
            val ordered = if (mine) {
                listOf(myUserId) + option.votes.filterNot { it == myUserId }
            } else {
                option.votes
            }
            PollOptionView(
                id = option.id,
                text = option.text,
                count = option.votes.size,
                fraction = if (total > 0) option.votes.size.toFloat() / total else 0f,
                isMine = mine,
                voters = ordered.map { PollVoter(it, names[it] ?: "Member $it") },
            )
        },
        closed = poll.closed,
        votedCount = poll.voters.size,
        familySize = familySize,
        hasVoted = poll.optionHeldBy(myUserId) != null,
    )
}

sealed interface ChatListItem {
    /** Stable LazyColumn key. */
    val key: String

    data class MessageItem(
        val entity: MessageEntity,
        val showSenderName: Boolean,
        val senderName: String?,
        val showTimestamp: Boolean,
        val reactionChips: List<ReactionChip> = emptyList(),
        /**
         * The poll on this message, resolved for drawing, or null when
         * the message is not one. Absent is the ONLY thing null means —
         * a poll dies with its message.
         */
        val poll: PollView? = null,
        /** Visually-top bubble of a same-sender same-day run. Defaults model a run of one. */
        val isRunStart: Boolean = true,
        /** Visually-bottom bubble of a same-sender same-day run. */
        val isRunEnd: Boolean = true,
    ) : ChatListItem {
        override val key: String get() = entity.clientMsgId

        /** My current reaction, if any (one per user per message). */
        val myReaction: String? get() = reactionChips.firstOrNull { it.includesMe }?.emoji
    }

    data class DateSeparator(
        val label: String,
        override val key: String,
    ) : ChatListItem

    /**
     * "N new messages", above the oldest message the reader has not
     * seen. At most one per thread, and only for a chat that opened
     * anchored.
     *
     * [count] is the chat's unread count as it stood AT OPEN. It does
     * not tick down as the reader reads: the divider is a mark of where
     * they started, and a number that melted while they scrolled would
     * be a different statement.
     */
    data class NewMessagesDivider(
        val count: Int,
        override val key: String,
    ) : ChatListItem
}

fun buildChatItems(
    messagesNewestFirst: List<MessageEntity>,
    isFamilyChat: Boolean,
    myUserId: Long,
    memberNames: Map<Long, String>,
    nowMillis: Long,
    zone: ZoneId = ZoneId.systemDefault(),
    /**
     * The assistant, from `GET /families/mine`. Its reserved account is in
     * no roster on purpose (it belongs to no family), so without this its
     * answers in the family chat would draw a bubble with no name above it.
     */
    assistantUserId: Long? = null,
    assistantName: String? = null,
    /**
     * Live members of the family, for a poll's "3 of 5 voted" footer.
     * 0 means the roster has not answered yet, and the footer then says
     * only how many votes there are rather than inventing a denominator.
     */
    familyMemberCount: Int = 0,
    /**
     * The oldest unread message, when this chat opened anchored at it —
     * OpenAnchor.Message.serverId, captured once. Null (the default) is
     * every other case: nothing unread, an anchor that gave up, or a
     * chat still deciding.
     */
    firstUnreadServerId: Long? = null,
    /** What the divider says. Ignored while [firstUnreadServerId] is null. */
    newMessageCount: Int = 0,
): List<ChatListItem> {
    val items = ArrayList<ChatListItem>(messagesNewestFirst.size + 8)
    messagesNewestFirst.forEachIndexed { index, message ->
        val newer = messagesNewestFirst.getOrNull(index - 1)
        val older = messagesNewestFirst.getOrNull(index + 1)

        val startsRun = older == null ||
            older.senderId != message.senderId ||
            !TimeFormat.sameDay(older.createdAt, message.createdAt, zone)
        val endsRun = newer == null ||
            newer.senderId != message.senderId ||
            !TimeFormat.sameDay(newer.createdAt, message.createdAt, zone)
        val endsMinuteRun = newer == null ||
            newer.senderId != message.senderId ||
            TimeFormat.bubbleTime(newer.createdAt, zone) != TimeFormat.bubbleTime(message.createdAt, zone)

        items += ChatListItem.MessageItem(
            entity = message,
            showSenderName = isFamilyChat && message.senderId != myUserId && startsRun,
            senderName = if (message.senderId == assistantUserId) {
                assistantName
            } else {
                memberNames[message.senderId]
            },
            showTimestamp = endsMinuteRun,
            reactionChips = buildReactionChips(
                reactions = ReactionsCodec.decode(message.reactionsJson),
                myUserId = myUserId,
            ),
            poll = PollCodec.decode(message.pollJson)?.let { poll ->
                buildPollView(
                    poll = poll,
                    myUserId = myUserId,
                    names = memberNames,
                    familySize = familyMemberCount,
                )
            },
            isRunStart = startsRun,
            isRunEnd = endsRun,
        )

        // Directly after the row in list order = directly above it on
        // screen. Before the day pill, so a first-unread that opens a
        // day reads [date][N new][bubble] rather than the other way up.
        if (firstUnreadServerId != null &&
            message.serverId == firstUnreadServerId &&
            newMessageCount > 0
        ) {
            items += ChatListItem.NewMessagesDivider(
                count = newMessageCount,
                key = "unread-$firstUnreadServerId",
            )
        }

        val dayEnds = older == null ||
            !TimeFormat.sameDay(older.createdAt, message.createdAt, zone)
        if (dayEnds) {
            val label = TimeFormat.dateSeparator(message.createdAt, nowMillis, zone)
            items += ChatListItem.DateSeparator(label = label, key = "sep-$label-${message.clientMsgId}")
        }
    }
    return items
}

/**
 * True when a message is nothing but photos and/or videos — no caption, no
 * quote, no poll, no call, nothing still arriving — and the bubble
 * therefore draws it BARE: no balloon, the way an emoji-only body draws.
 *
 * The balloon exists to give TEXT a surface. A photo brings its own, and a
 * photo inside a tinted balloon is a frame around a picture — which is why
 * every mainstream messenger draws a lone photo bare. Every one of them
 * also keeps the balloon for voice notes, documents and places, and so does
 * this rule: a file, audio or location row is words and controls, which
 * need the surface (and its wash and hairline are cut FOR a balloon — on
 * the chat background they vanish). A caption, a quote or a row beside the
 * photo keeps the balloon for the same reason: the words in it need the
 * surface, and a photo hanging half out of a balloon is the look everyone
 * moved away from. Same rule on iOS and macOS
 * (`MessagePresentation.isMediaOnly`), pinned by mirrored vectors.
 */
fun isMediaOnly(entity: MessageEntity, isStreaming: Boolean = false): Boolean {
    if (isStreaming || entity.body.isNotEmpty()) return false
    if (entity.replyToMessageId != null || entity.pollJson != null || entity.call != null) return false
    val attachments = entity.attachmentList
    return attachments.isNotEmpty() && AttachmentAlbum.rows(attachments).isEmpty()
}

/**
 * Whether a lone photo/video tile draws its hairline.
 *
 * ONE sentence covers all three surfaces: a media tile draws a hairline
 * only where its own pixels are not already the edge — over a balloon, or
 * before the picture has landed.
 *
 * The hairline was cut for a balloon ("so a pale photo does not melt into
 * a pale balloon"), and on a BARE message there is no balloon to melt
 * into: what is left is a frame around a picture, which is the thing the
 * bare treatment exists to remove. But a tile with no picture yet is not a
 * picture — it is a reserved rectangle holding a 14%→6% wash, and against
 * the chat background its lower corners stop existing. So the stroke stays
 * exactly as long as nothing else is drawing an edge.
 *
 * NOT the album's rule. A pile's cards keep their border always, because
 * there it separates photo from PHOTO: at the overlap the front card's top
 * edge is the one line between two shots of the same scene, and before
 * bytes the two washes stack to within 0.002 alpha of each other. Same
 * rule on iOS and macOS (`MessagePresentation.drawsHairline`), pinned by
 * mirrored vectors.
 */
fun drawsHairline(onBalloon: Boolean, hasImage: Boolean): Boolean = onBalloon || !hasImage
