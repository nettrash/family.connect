/*
 * PushRouteParser.kt
 * Family Connect (Android)
 *
 * Pure mapping from an FCM data payload (docs/protocol.md, "Push
 * notifications") to the screen a notification tap should open:
 *
 *   {"kind": "message", "chat_id": "42", ...} → Chat(42)
 *   {"kind": "board_note", "family_id": …, "note_id": …} → Board
 *   {"kind": "join_request", "family_id": …}  → JoinRequests (owner-only push)
 *   {"kind": "joined", "family_id": …}        → ChatList
 *   {"kind": "call", "call_id": …, …}          → Call — the call screen
 *                                                (docs/protocol.md,
 *                                                "Incoming calls"); a tap
 *                                                on the notification, or
 *                                                its Answer button
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
    data object Board : PendingRoute
    data object JoinRequests : PendingRoute
    data object ChatList : PendingRoute

    /** The call screen; [answer] when the tap was the notification's Answer button. */
    data class Call(val answer: Boolean = false) : PendingRoute

    /**
     * The Phone app's call log asked for a call back (TelecomCalls,
     * CallBackRegistry): open the chat and ring them again.
     */
    data class CallBack(val chatId: Long, val peerUserId: Long, val video: Boolean) : PendingRoute
}

object PushRouteParser {

    /** Payload keys, exactly as the server writes them into `data`. */
    const val KEY_KIND = "kind"
    const val KEY_CHAT_ID = "chat_id"
    const val KEY_CALL_ID = "call_id"
    const val KEY_FROM_USER_ID = "from_user_id"
    const val KEY_CALLER_NAME = "caller_name"

    /** The `kind` of an incoming-call data push, and of the notification it becomes. */
    const val KIND_CALL = "call"

    /** The call push's kind flag (docs/protocol.md, "Video") — a STRING, like every FCM data value. */
    const val KEY_VIDEO = "video"

    /** Intent extra a call notification's Answer button sets to [ACTION_ANSWER]. */
    const val KEY_CALL_ACTION = "call_action"
    const val ACTION_ANSWER = "answer"

    /**
     * True when the call push announces a VIDEO call — `"video": "true"`
     * on the wire; absent (every older server) reads as voice.
     */
    fun isVideoCall(data: Map<String, String>): Boolean = data[KEY_VIDEO] == "true"

    fun parse(data: Map<String, String>): PendingRoute? = when (data[KEY_KIND]) {
        // FCM data values are always strings — chat_id arrives as "42".
        "message" -> data[KEY_CHAT_ID]?.toLongOrNull()?.let { PendingRoute.Chat(it) }
        "board_note" -> PendingRoute.Board
        "join_request" -> PendingRoute.JoinRequests
        "joined" -> PendingRoute.ChatList
        KIND_CALL -> PendingRoute.Call(answer = data[KEY_CALL_ACTION] == ACTION_ANSWER)
        else -> null
    }
}
