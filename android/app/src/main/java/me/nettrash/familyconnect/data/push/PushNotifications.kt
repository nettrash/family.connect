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
 * iOS counterpart: none yet (push is not ported to ios/ at this time).
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
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle(title)
            .setContentText(body)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setAutoCancel(true)
            .setContentIntent(contentIntent)
            .build()
    }

    fun show(
        context: Context,
        title: String,
        body: String,
        kind: String?,
        chatId: Long?,
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
            .notify(tag, NOTIFICATION_ID, build(context, title, body, kind, chatId))
    }
}
