/*
 * PushNotifications.kt
 * Family Connect (Android)
 *
 * The notification surface, matching docs/protocol.md ("Push
 * notifications") field for field:
 *
 *   channel  "messages"  — the server's android.notification.channel_id;
 *                          created at app start (FamilyConnectApp) so the
 *                          very first push, even one rendered by the
 *                          system tray while the app is dead, lands in a
 *                          channel that exists.
 *   tag      "chat-<id>" — the server's android.notification.tag: one
 *                          notification slot per chat, newest message
 *                          replaces the previous one instead of stacking.
 *
 * The tap intent carries the SAME extras keys as the FCM data payload
 * ("kind", "chat_id") because that is what the FCM SDK puts on the
 * launcher intent when the *system tray* built the notification — one
 * parser (PushRouteParser) then serves both delivery paths.
 *
 * THE NUMBER. `setNumber` is the whole of Android's badge story (see
 * UnreadBadge.kt): there is no icon-badge API, so the count a launcher
 * may draw is `Notification.number`, summed across this app's live
 * notifications. The server sends it per chat as
 * `android.notification.notification_count` (docs/protocol.md), and
 * which of the two delivery paths a push takes decides who applies it:
 *
 *   system tray  — the COMMON path, app backgrounded or dead. FCM builds
 *                  the notification itself and calls setNumber for us
 *                  (CommonNotificationBuilder, from NotificationParams'
 *                  `gcm.n.notification_count`). Nothing in this file
 *                  runs. This is exactly why the count had to travel on
 *                  the wire rather than be worked out here.
 *   this file    — the RARE path, [build] via FcPushService: foreground
 *                  with no live socket. The same wire value is read off
 *                  RemoteMessage and passed to [build], so the number a
 *                  launcher sees does not depend on which path a push
 *                  happened to take.
 *
 * No notification GROUP and no summary, deliberately. A group is not
 * what makes a number readable — `setNumber` alone is — and the tray
 * path above cannot be given one: the server sends no `group` and this
 * process is not running to add it. Grouping only the rare path would
 * make two structurally different trays for one product, with the
 * summary as an extra row to be counted or not counted depending on the
 * launcher. Android auto-bundles four or more notifications from one app
 * anyway.
 *
 * iOS counterpart: ios/FamilyConnect/Core/ChatNotifier.swift (the
 * dismissal half; the badge half is UnreadBadge.swift, which has an icon
 * to write a total onto and so needs none of this).
 */

package me.nettrash.familyconnect.data.push

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import me.nettrash.familyconnect.MainActivity
import me.nettrash.familyconnect.R

object PushNotifications {

    /** Protocol: android.notification.channel_id — must match the server. */
    const val CHANNEL_ID = "messages"

    /** Intent-extra keys == FCM data-payload keys, see file header. */
    const val EXTRA_KIND = PushRouteParser.KEY_KIND
    const val EXTRA_CHAT_ID = PushRouteParser.KEY_CHAT_ID

    /** One id, distinct tags: the (tag, id) pair is the dedup slot. */
    const val NOTIFICATION_ID = 1

    /** Protocol: android.notification.tag = "chat-<id>". */
    fun chatTag(chatId: Long): String = "chat-$chatId"

    /**
     * Idempotent — createNotificationChannel is a no-op for an existing
     * id. minSdk 26 ⇒ channels always exist, no version guard needed.
     */
    fun ensureChannel(context: Context) {
        val channel = NotificationChannel(
            CHANNEL_ID,
            "Messages",
            // Protocol sends priority HIGH; heads-up on the client side too.
            NotificationManager.IMPORTANCE_HIGH,
        ).apply {
            description = "New family messages and join requests"
        }
        context.getSystemService(NotificationManager::class.java)
            .createNotificationChannel(channel)
    }

    /**
     * Build the notification the app shows itself (foreground delivery
     * with no live socket — see FcPushService). The system-tray path
     * builds its own; both funnel taps through the same extras.
     */
    fun build(
        context: Context,
        title: String,
        body: String,
        kind: String?,
        chatId: Long?,
        /**
         * The recipient's unread IN THIS CHAT, straight off the wire
         * (`android.notification.notification_count`). Null leaves the
         * number unset, which is the honest answer for a push that
         * carried none — an older server, or one of the kinds that has no
         * chat to count (`board_note`, `join_request`, `joined`). It is
         * never guessed from the local store: this path runs precisely
         * when the app has NOT stored the message being announced, so the
         * store is one behind by construction and a number derived from
         * it would be wrong by one.
         */
        unreadInChat: Int? = null,
    ): Notification {
        val tapIntent = Intent(context, MainActivity::class.java).apply {
            kind?.let { putExtra(EXTRA_KIND, it) }
            // String, not long — FCM data values are strings and the
            // system-tray path delivers them as string extras; keeping
            // the type identical keeps MainActivity's parsing single-path.
            chatId?.let { putExtra(EXTRA_CHAT_ID, it.toString()) }
            // singleTop (with the manifest launchMode): an already-running
            // task gets onNewIntent and navigates instead of relaunching.
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        }
        val contentIntent = PendingIntent.getActivity(
            context,
            // Distinct request code per chat so two chats' pending intents
            // don't collapse into one (extras alone don't disambiguate).
            (chatId ?: 0L).toInt(),
            tapIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(context, CHANNEL_ID)
            // Small icons render as alpha masks; ic_notification is the
            // knocked-out silhouette on the 24dp canvas (the launcher
            // foreground would draw as a half-size solid blob).
            .setSmallIcon(R.drawable.ic_notification)
            .setColor(ContextCompat.getColor(context, R.color.ic_launcher_background))
            .setContentTitle(title)
            .setContentText(body)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            // Which icon the launcher draws INSIDE the badge when someone
            // long-presses it — not whether the number appears, which is
            // setNumber's job alone. Small, because this app sets no large
            // icon at all, so the default (BADGE_ICON_LARGE) resolves to
            // the same silhouette by a longer road.
            .setBadgeIconType(NotificationCompat.BADGE_ICON_SMALL)
            .setAutoCancel(true)
            .setContentIntent(contentIntent)
            // Set only when the wire said so. `setNumber(0)` is not
            // "no number" — it is a zero, and a launcher that sums live
            // notifications would happily draw a badge saying nothing is
            // waiting on a notification that says something is.
            .apply { unreadInChat?.let(::setNumber) }
            .build()
    }

    /**
     * Drop everything this chat has in the tray, because it has been read.
     *
     * On Android the tray entry IS the badge — the number rides on it
     * ([build], setNumber) and a launcher that draws only a dot draws
     * that dot for as long as the notification lives. Nothing else
     * cancels it: setAutoCancel covers the TAP alone, so a chat read
     * anywhere else used to leave the badge lit indefinitely, advertising
     * messages the user had already read.
     *
     * "Read" is a PERSON's state and not a device's — the marker is
     * shared across every device its owner has (docs/protocol.md, `GET
     * /chats` → `last_read_message_id`) — so the callers are all four
     * ways this device can learn of it; see UnreadNotifications.
     */
    fun cancelChat(context: Context, chatId: Long) {
        cancelChats(context, setOf(chatId))
    }

    /**
     * The same, for several chats at once.
     *
     * A set rather than a chat at a time because a resync answers for the
     * whole list in one go, and one pass over the tray settles all of it —
     * `activeNotifications` is a binder round trip, and doing it per chat
     * would repeat that for every silent chat in the family.
     *
     * Swept by tag rather than cancelled at the (tag, [NOTIFICATION_ID])
     * slot: when a push lands while the app is dead the SYSTEM tray builds
     * the notification and picks its own id — only the tag is ours
     * (protocol: android.notification.tag) — and those are precisely the
     * ones still sitting there when the app is finally opened.
     */
    fun cancelChats(context: Context, chatIds: Set<Long>) {
        if (chatIds.isEmpty()) return
        val tags = chatIds.mapTo(HashSet(), ::chatTag)
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.activeNotifications
            .filter { it.tag in tags }
            .forEach { manager.cancel(it.tag, it.id) }
    }

    fun show(
        context: Context,
        title: String,
        body: String,
        kind: String?,
        chatId: Long?,
        unreadInChat: Int? = null,
    ) {
        // 13+ runtime permission may be denied (notify() would be silently
        // dropped anyway; checking keeps lint and the intent explicit).
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        val tag = chatId?.let(::chatTag) ?: kind ?: "push"
        NotificationManagerCompat.from(context)
            .notify(tag, NOTIFICATION_ID, build(context, title, body, kind, chatId, unreadInChat))
    }
}
