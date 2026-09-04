/*
 * CallNotifications.kt
 * Family Connect (Android)
 *
 * The two notifications a call has: the RINGING one (a full-screen,
 * ringtone-backed CallStyle with Answer / Decline, which is what makes a
 * phone with the app dead actually ring) and the ONGOING one (silent,
 * with Hang up) that CallService lives behind for the rest of the call.
 *
 * Two channels rather than one: the ringing channel carries the
 * ringtone and vibration, and an update on the ongoing channel must not
 * play them again. Both are created at app start, like "messages".
 *
 * The actions: Answer opens MainActivity carrying a `call` route, because
 * taking a call needs the microphone permission and only a screen can ask
 * for it; Decline and Hang up go straight to CallService, which needs no
 * screen at all.
 */

package me.nettrash.familyconnect.calls

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.Person
import androidx.core.content.ContextCompat
import me.nettrash.familyconnect.MainActivity
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.push.PushNotifications

object CallNotifications {

    const val CHANNEL_RINGING = "calls"
    const val CHANNEL_ONGOING = "call_ongoing"

    /** Distinct from PushNotifications.NOTIFICATION_ID (1), which is per-chat by tag. */
    const val INCOMING_ID = 2
    const val ONGOING_ID = 3

    /** The `kind` extra a call notification's tap or Answer carries. */
    const val KIND_CALL = "call"

    /** Extra naming what the tap asked for: [ACTION_ANSWER], or absent to just show the call. */
    const val EXTRA_CALL_ACTION = "call_action"
    const val ACTION_ANSWER = "answer"

    fun ensureChannels(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        val ringing = NotificationChannel(
            CHANNEL_RINGING,
            context.getString(R.string.s_channel_calls),
            NotificationManager.IMPORTANCE_HIGH,
        ).apply {
            description = context.getString(R.string.s_channel_calls_desc)
            setSound(
                RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE),
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                    .build(),
            )
            enableVibration(true)
            vibrationPattern = longArrayOf(0, 800, 600, 800, 600, 800)
        }
        val ongoing = NotificationChannel(
            CHANNEL_ONGOING,
            context.getString(R.string.s_channel_ongoing),
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = context.getString(R.string.s_channel_ongoing_desc)
            setSound(null, null)
            enableVibration(false)
        }
        manager.createNotificationChannel(ringing)
        manager.createNotificationChannel(ongoing)
    }

    /**
     * The ringing line — chosen by the call's KIND, which the offer fixed
     * at placement (docs/protocol.md, "Video"). Pure, so the choice is
     * pinned on the plain JVM (CallNotificationsWordingTest).
     */
    fun incomingTextRes(video: Boolean): Int =
        if (video) R.string.s_incoming_video_call else R.string.s_incoming_voice_call

    /** The ongoing notification's status line while the call is up. Pure, same reason. */
    fun ongoingTextRes(video: Boolean): Int =
        if (video) R.string.s_ongoing_video_call else R.string.s_ongoing_voice_call

    /**
     * The call's kind as a title — what stands in for the peer's name when
     * the roster does not know them (CallService.nameOf). Pure, same reason.
     */
    fun kindTextRes(video: Boolean): Int =
        if (video) R.string.s_video_call else R.string.s_voice_call

    /**
     * What an ENDED call says — the screen's linger line and the ongoing
     * notification's last status. Direction-aware where the two sides
     * see different things, mirroring iOS CallStatusText.ended: a
     * timeout is "No answer" to the caller and a missed call to the
     * callee; a decline is "Declined" to the caller and plain "Call
     * ended" to the one who declined. Pure, same reason as the others.
     */
    fun endedTextRes(reason: CallEnding, outgoing: Boolean, video: Boolean): Int = when (reason) {
        CallEnding.HANGUP, CallEnding.CANCEL, CallEnding.NO_OFFER -> R.string.s_call_ended
        CallEnding.DECLINE -> if (outgoing) R.string.s_declined else R.string.s_call_ended
        CallEnding.TIMEOUT -> when {
            outgoing -> R.string.s_no_answer
            video -> R.string.s_missed_video_call
            else -> R.string.s_missed_voice_call
        }
        CallEnding.FAILED -> R.string.s_call_failed
        CallEnding.ANSWERED_ELSEWHERE -> R.string.s_answered_on_another_device
        CallEnding.BUSY, CallEnding.PEER_BUSY -> R.string.s_busy
        CallEnding.PEER_UNREACHABLE -> R.string.s_not_reachable
        CallEnding.DISABLED -> R.string.s_calls_are_off_on_this_server
        CallEnding.VIDEO_DISABLED -> R.string.s_video_calls_are_off_on_this_server
    }

    /** The full-screen ringing notification for [callerName]. */
    fun incoming(context: Context, callerName: String, peerUserId: Long, video: Boolean): Notification {
        val person = Person.Builder().setName(callerName).setKey(peerUserId.toString()).build()
        val open = activityIntent(context, action = null, requestCode = 10)
        val answer = activityIntent(context, action = ACTION_ANSWER, requestCode = 11)
        val decline = serviceIntent(context, CallService.ACTION_DECLINE, requestCode = 12)
        val builder = NotificationCompat.Builder(context, CHANNEL_RINGING)
            .setSmallIcon(R.drawable.ic_notification)
            .setColor(ContextCompat.getColor(context, R.color.ic_launcher_background))
            .setContentTitle(callerName)
            .setContentText(context.getString(incomingTextRes(video)))
            .setPriority(NotificationCompat.PRIORITY_MAX)
            .setCategory(NotificationCompat.CATEGORY_CALL)
            .setOngoing(true)
            .setAutoCancel(false)
            .setContentIntent(open)
            // The phone lights up with the call, lock screen or not — the
            // thing a notification banner cannot do. Needs
            // USE_FULL_SCREEN_INTENT (manifest).
            .setFullScreenIntent(open, true)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            builder.setStyle(NotificationCompat.CallStyle.forIncomingCall(person, decline, answer))
        } else {
            builder
                .addAction(0, context.getString(R.string.s_decline), decline)
                .addAction(0, context.getString(R.string.s_answer), answer)
        }
        return builder.build()
    }

    /** The silent notification the call runs behind; [status] is the timer or "Connecting…". */
    fun ongoing(context: Context, peerName: String, peerUserId: Long, status: String): Notification {
        val person = Person.Builder().setName(peerName).setKey(peerUserId.toString()).build()
        val open = activityIntent(context, action = null, requestCode = 10)
        val hangUp = serviceIntent(context, CallService.ACTION_HANG_UP, requestCode = 13)
        val builder = NotificationCompat.Builder(context, CHANNEL_ONGOING)
            .setSmallIcon(R.drawable.ic_notification)
            .setColor(ContextCompat.getColor(context, R.color.ic_launcher_background))
            .setContentTitle(peerName)
            .setContentText(status)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setCategory(NotificationCompat.CATEGORY_CALL)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(open)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            builder.setStyle(NotificationCompat.CallStyle.forOngoingCall(person, hangUp))
        } else {
            builder.addAction(0, context.getString(R.string.s_hang_up), hangUp)
        }
        return builder.build()
    }

    /**
     * Post the ringing notification without a foreground service — the
     * fallback for an offer that arrives while the app is in the
     * background and may not start one (Android 12+). The call still
     * rings; it just is not pinned to a service.
     */
    fun showIncoming(context: Context, callerName: String, peerUserId: Long, video: Boolean) {
        // Inline rather than through canNotify: lint's MissingPermission
        // check only sees a permission check in the SAME method.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        NotificationManagerCompat.from(context)
            .notify(INCOMING_ID, incoming(context, callerName, peerUserId, video))
    }

    fun cancelAll(context: Context) {
        val manager = NotificationManagerCompat.from(context)
        manager.cancel(INCOMING_ID)
        manager.cancel(ONGOING_ID)
    }

    /** True when the intent that opened MainActivity came from a call notification. */
    fun isCallIntent(intent: Intent?): Boolean =
        intent?.getStringExtra(PushNotifications.EXTRA_KIND) == KIND_CALL

    private fun activityIntent(context: Context, action: String?, requestCode: Int): PendingIntent {
        val intent = Intent(context, MainActivity::class.java).apply {
            putExtra(PushNotifications.EXTRA_KIND, KIND_CALL)
            action?.let { putExtra(EXTRA_CALL_ACTION, it) }
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        }
        return PendingIntent.getActivity(
            context,
            requestCode,
            intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }

    private fun serviceIntent(context: Context, action: String, requestCode: Int): PendingIntent {
        val intent = Intent(context, CallService::class.java).setAction(action)
        return PendingIntent.getService(
            context,
            requestCode,
            intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }
}
