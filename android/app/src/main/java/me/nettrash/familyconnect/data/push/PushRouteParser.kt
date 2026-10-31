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
 *   {"kind": "report", "family_id": …}        → Reports (owner-only push)
 *   {"kind": "joined", "family_id": …}        → ChatList
 *   {"kind": "invitation", "family_id": …,
 *    "invitation_id": …}                      → Invitations — the chat
 *                                                list, whose banner (and the
 *                                                switcher) holds them
 *   {"kind": "family_plan", "family_id": …,
 *    "stage": "unpaid"|"tomorrow"|"deleted"}  → FamilyPlan — that family's
 *                                                chat list, whose banner says
 *                                                when; for "deleted", the
 *                                                list of the families left
 *                                                (docs/protocol.md, "Paid
 *                                                families")
 *
 * Since several families a push also says WHICH family (`family_id`,
 * [familyId]): the app switches to it before following the route
 * (docs/protocol.md, "Several families" — "Pushes").
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

    /**
     * kind "report" — the owner's moderation inbox.
     *
     * A report raises a push and NO WebSocket frame, exactly as a join
     * request does, so this push is the owner's only live signal that one
     * arrived (docs/protocol.md, "Reporting a member").
     */
    data object Reports : PendingRoute
    data object ChatList : PendingRoute

    /**
     * kind "invitation" — somebody invited this account into their family
     * (docs/protocol.md, "Several families"). The chat list's banner and the
     * switcher are where invitations are answered.
     */
    data object Invitations : PendingRoute

    /**
     * kind "family_plan" — a paid family's deadline (docs/protocol.md, "Paid
     * families", added 2026-10-30): it stopped being paid for, it is deleted
     * tomorrow, or it was [deleted]. The first two open that family's chat
     * list, where the banner says when; a deleted one opens on the families
     * that are left, so it carries no family to switch to. The server wrote
     * the title and body.
     */
    data class FamilyPlan(val deleted: Boolean) : PendingRoute

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

    /** The family a push is about — present on every family-level push since several families. */
    const val KEY_FAMILY_ID = "family_id"

    /** Where a paid family's deadline is: "unpaid", "tomorrow" or "deleted" (kind "family_plan"). */
    const val KEY_STAGE = "stage"
    const val KIND_FAMILY_PLAN = "family_plan"
    const val STAGE_DELETED = "deleted"
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
        "report" -> PendingRoute.Reports
        "joined" -> PendingRoute.ChatList
        "invitation" -> PendingRoute.Invitations
        KIND_FAMILY_PLAN -> PendingRoute.FamilyPlan(deleted = data[KEY_STAGE] == STAGE_DELETED)
        KIND_CALL -> PendingRoute.Call(answer = data[KEY_CALL_ACTION] == ACTION_ANSWER)
        else -> null
    }

    /**
     * The family a push opens in, or null when it names none — a direct
     * chat's, which opens wherever that chat is listed, and every push from
     * a server that predates several families.
     *
     * Null as well for a family that was DELETED (kind "family_plan", stage
     * "deleted"): there is nothing left to open, and a device that has not
     * heard yet must not switch to it on the way to finding out.
     */
    fun familyId(data: Map<String, String>): Long? {
        if (data[KEY_KIND] == KIND_FAMILY_PLAN && data[KEY_STAGE] == STAGE_DELETED) return null
        return data[KEY_FAMILY_ID]?.toLongOrNull()
    }
}
