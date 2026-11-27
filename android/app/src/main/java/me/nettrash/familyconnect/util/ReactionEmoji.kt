/*
 * ReactionEmoji.kt
 * Family Connect (Android)
 *
 * Emoji keys for search and recents, matching the shared Rust search.
 */

package me.nettrash.familyconnect.util

internal fun reactionEmojiKey(emoji: String): String = emoji.replace("\uFE0F", "")
