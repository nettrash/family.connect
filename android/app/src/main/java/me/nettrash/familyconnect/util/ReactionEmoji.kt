package me.nettrash.familyconnect.util

/** Compare presentation variants while preserving the selected catalogue sequence on the wire. */
internal fun reactionEmojiKey(emoji: String): String =
    emoji.replace("\uFE0E", "").replace("\uFE0F", "")
