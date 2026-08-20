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
 *   - showSenderName: family chat only, not my own messages, and only
 *     on the first message of a same-sender run (previous OLDER item
 *     differs in sender or day).
 *   - showTimestamp: on the last message of a same-sender same-minute
 *     run — consecutive rapid-fire messages share one timestamp.
 *   - reactionChips: the row's reactionsJson aggregated into
 *     (emoji, count, includesMe) chips in first-seen order.
 *
 * iOS counterpart: ios/FamilyConnect/UI/Chat/ChatItems.swift
 */

package me.nettrash.familyconnect.ui.chat

import me.nettrash.familyconnect.data.db.MessageEntity
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

sealed interface ChatListItem {
    /** Stable LazyColumn key. */
    val key: String

    data class MessageItem(
        val entity: MessageEntity,
        val showSenderName: Boolean,
        val senderName: String?,
        val showTimestamp: Boolean,
        val reactionChips: List<ReactionChip> = emptyList(),
    ) : ChatListItem {
        override val key: String get() = entity.clientMsgId

        /** My current reaction, if any (one per user per message). */
        val myReaction: String? get() = reactionChips.firstOrNull { it.includesMe }?.emoji
    }

    data class DateSeparator(
        val label: String,
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
): List<ChatListItem> {
    val items = ArrayList<ChatListItem>(messagesNewestFirst.size + 8)
    messagesNewestFirst.forEachIndexed { index, message ->
        val newer = messagesNewestFirst.getOrNull(index - 1)
        val older = messagesNewestFirst.getOrNull(index + 1)

        val startsRun = older == null ||
            older.senderId != message.senderId ||
            !TimeFormat.sameDay(older.createdAt, message.createdAt, zone)
        val endsMinuteRun = newer == null ||
            newer.senderId != message.senderId ||
            TimeFormat.bubbleTime(newer.createdAt, zone) != TimeFormat.bubbleTime(message.createdAt, zone)

        items += ChatListItem.MessageItem(
            entity = message,
            showSenderName = isFamilyChat && message.senderId != myUserId && startsRun,
            senderName = memberNames[message.senderId],
            showTimestamp = endsMinuteRun,
            reactionChips = buildReactionChips(
                reactions = ReactionsCodec.decode(message.reactionsJson),
                myUserId = myUserId,
            ),
        )

        val dayEnds = older == null ||
            !TimeFormat.sameDay(older.createdAt, message.createdAt, zone)
        if (dayEnds) {
            val label = TimeFormat.dateSeparator(message.createdAt, nowMillis, zone)
            items += ChatListItem.DateSeparator(label = label, key = "sep-$label-${message.clientMsgId}")
        }
    }
    return items
}
