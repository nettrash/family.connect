/*
 * ChatVanished.kt
 * Family Connect (Android)
 *
 * The "chat vanished" signal ChatScreen and GroupInfoScreen act on, as a
 * flow operator a ViewModel can hold and a JVM test can pin: a chat row
 * that was SEEN and then reads as null has left this device — a peer's
 * deleted account, a group left, removed from or deleted (`chat_removed`,
 * a `GET /chats` prune, or Group Info's own Leave/Delete; docs/protocol.md,
 * "Groups"). A first null is Room not having answered yet, never a
 * departure, so it does not count.
 */

package me.nettrash.familyconnect.util

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.scan

/** True from the moment a row this flow has shown goes away; false before. One-way. */
fun <T : Any> Flow<T?>.goneAfterSeen(): Flow<Boolean> =
    scan(Pair(false, false)) { (seen, gone), row ->
        when {
            gone -> Pair(true, true)
            row != null -> Pair(true, false)
            else -> Pair(seen, seen)
        }
    }
        .map { it.second }
        .distinctUntilChanged()
