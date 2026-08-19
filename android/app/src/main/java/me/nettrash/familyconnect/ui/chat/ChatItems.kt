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
 *
 * iOS counterpart: ios/FamilyConnect/UI/Chat/ChatItems.swift
 */

package me.nettrash.familyconnect.ui.chat

import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.util.TimeFormat
import java.time.ZoneId

sealed interface ChatListItem {
    /** Stable LazyColumn key. */
    val key: String

    data class MessageItem(
        val entity: MessageEntity,
        val showSenderName: Boolean,
        val senderName: String?,
        val showTimestamp: Boolean,
    ) : ChatListItem {
        override val key: String get() = entity.clientMsgId
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
