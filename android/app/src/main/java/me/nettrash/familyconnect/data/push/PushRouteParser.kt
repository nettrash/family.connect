/*
 * PushRouteParser.kt
 * Family Connect (Android)
 *
 * Pure mapping from an FCM data payload (docs/protocol.md, "Push
 * notifications") to the screen a notification tap should open:
 *
 *   {"kind": "message", "chat_id": "42", ...} → Chat(42)
 *   {"kind": "join_request", "family_id": …}  → JoinRequests (owner-only push)
 *   {"kind": "joined", "family_id": …}        → ChatList
 *
 * The same map shape arrives two ways — as RemoteMessage.data when the
 * app builds the notification itself (foreground race), and as launcher-
 * intent extras when the SYSTEM tray built it (FCM copies the data
 * payload onto the tap intent) — so one parser covers both. Unknown or
 * malformed kinds return null: the tap just opens the app (protocol
 * compatibility rule — ignore what you don't understand).
 *
 * Kept free of Android/Firebase types on purpose: golden-tested on the
 * plain JVM against the protocol's example payloads.
 *
 * iOS counterpart: none yet (push is not ported to ios/ at this time).
 */

package me.nettrash.familyconnect.data.push

/** Where a notification tap should land, decoupled from nav-route strings. */
sealed interface PendingRoute {
    data class Chat(val chatId: Long) : PendingRoute
    data object JoinRequests : PendingRoute
    data object ChatList : PendingRoute
}

object PushRouteParser {

    /** Payload keys, exactly as the server writes them into `data`. */
    const val KEY_KIND = "kind"
    const val KEY_CHAT_ID = "chat_id"

    fun parse(data: Map<String, String>): PendingRoute? = when (data[KEY_KIND]) {
        // FCM data values are always strings — chat_id arrives as "42".
        "message" -> data[KEY_CHAT_ID]?.toLongOrNull()?.let { PendingRoute.Chat(it) }
        "join_request" -> PendingRoute.JoinRequests
        "joined" -> PendingRoute.ChatList
        else -> null
    }
}
