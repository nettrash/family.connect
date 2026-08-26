/*
 * PushNotificationsTest.kt
 * Family Connect (Android)
 *
 * Pins the wire-visible notification constants to docs/protocol.md —
 * channel_id "messages" and tag "chat-<id>" are written by the SERVER
 * into every FCM message, so a client-side rename would silently split
 * pushes across two channels / stack instead of replace. Robolectric for
 * the channel-creation and builder checks (real android.app types).
 */

package me.nettrash.familyconnect.data.push

import android.app.NotificationManager
import android.content.Context
import android.os.Bundle
import com.google.firebase.messaging.RemoteMessage
import com.google.common.truth.Truth.assertThat
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@RunWith(RobolectricTestRunner::class)
class PushNotificationsTest {

    // RuntimeEnvironment, not androidx.test ApplicationProvider — the
    // test classpath deliberately has no androidx.test (see TestDb.kt).
    private val context: Context = RuntimeEnvironment.getApplication()

    @Test
    fun channelIdMatchesTheProtocol() {
        // Protocol: android.notification.channel_id = "messages".
        assertThat(PushNotifications.CHANNEL_ID).isEqualTo("messages")
    }

    @Test
    fun chatTagMatchesTheProtocol() {
        // Protocol: android.notification.tag = "chat-42".
        assertThat(PushNotifications.chatTag(42L)).isEqualTo("chat-42")
    }

    @Test
    fun extrasKeysMatchTheFcmDataPayloadKeys() {
        // The system-tray path copies the DATA payload keys onto the
        // launcher intent — our own tap intent must use the same ones.
        assertThat(PushNotifications.EXTRA_KIND).isEqualTo("kind")
        assertThat(PushNotifications.EXTRA_CHAT_ID).isEqualTo("chat_id")
    }

    @Test
    fun ensureChannelCreatesTheHighImportanceMessagesChannel() {
        PushNotifications.ensureChannel(context)

        val manager = context.getSystemService(NotificationManager::class.java)
        val channel = manager.getNotificationChannel(PushNotifications.CHANNEL_ID)
        assertThat(channel).isNotNull()
        assertThat(channel!!.importance).isEqualTo(NotificationManager.IMPORTANCE_HIGH)

        // Idempotent — app start runs it on every launch.
        PushNotifications.ensureChannel(context)
        assertThat(manager.notificationChannels.count { it.id == PushNotifications.CHANNEL_ID })
            .isEqualTo(1)
    }

    @Test
    fun builtNotificationTargetsTheMessagesChannelAndCarriesATapIntent() {
        PushNotifications.ensureChannel(context)

        val notification = PushNotifications.build(
            context = context,
            title = "Anna",
            body = "Dinner at 7?",
            kind = "message",
            chatId = 42L,
        )

        assertThat(notification.channelId).isEqualTo(PushNotifications.CHANNEL_ID)
        assertThat(notification.contentIntent).isNotNull()
    }

    // -- the number ---------------------------------------------------------------------
    //
    // Android has no icon-badge API; `Notification.number` is the whole
    // of it (UnreadBadge.kt). The server sends the per-chat count as
    // `android.notification.notification_count` and FCM applies it for
    // the system-tray path itself — this file's job is the OTHER path,
    // which must end up with the same number for the same push.

    @Test
    fun theWireCountBecomesTheNotificationsNumber() {
        PushNotifications.ensureChannel(context)

        val notification = PushNotifications.build(
            context = context,
            title = "Anna",
            body = "Dinner at 7?",
            kind = "message",
            chatId = 42L,
            unreadInChat = 3,
        )

        assertThat(notification.number).isEqualTo(3)
    }

    /**
     * WHERE THE NUMBER COMES FROM, pinned against the SDK.
     *
     * The server writes `android.notification.notification_count`
     * (docs/protocol.md); FCM lands it in the message bundle under
     * `gcm.n.notification_count`, and reads it back off there for BOTH
     * delivery paths — `CommonNotificationBuilder` calls setNumber with
     * it when the system tray builds the notification, and
     * `RemoteMessage.Notification.getNotificationCount()` is the same
     * value handed to a running app. FcPushService reads exactly that,
     * so the two paths cannot produce different numbers for one push.
     *
     * `gcm.n.e = "1"` is what marks a bundle as a notification message at
     * all (`NotificationParams.isNotification`); without it the
     * notification block is absent and the count with it.
     */
    @Test
    fun theCountRidesOnTheSameBundleKeyFcmItselfReads() {
        val bundle = Bundle().apply {
            putString("gcm.n.e", "1")
            putString("gcm.n.title", "Anna")
            putString("gcm.n.body", "Dinner at 7?")
            putString("gcm.n.notification_count", "3")
        }

        val notification = RemoteMessage(bundle).notification

        assertThat(notification).isNotNull()
        assertThat(notification!!.notificationCount).isEqualTo(3)
    }

    /** A push from a server that predates the field says nothing at all. */
    @Test
    fun aBundleWithoutTheCountKeyReportsNoCount() {
        val bundle = Bundle().apply {
            putString("gcm.n.e", "1")
            putString("gcm.n.title", "Anna")
            putString("gcm.n.body", "Dinner at 7?")
        }

        assertThat(RemoteMessage(bundle).notification!!.notificationCount).isNull()
    }

    /**
     * A push that carried no count — an older server, or one of the
     * kinds that has no chat to count — leaves the number unset. Writing
     * a 0 instead is not "no number": a launcher that sums live
     * notifications would draw a badge saying nothing is waiting on a
     * notification that says something is.
     */
    @Test
    fun aPushWithNoCountLeavesTheNumberUnset() {
        PushNotifications.ensureChannel(context)

        val notification = PushNotifications.build(
            context = context,
            title = "The Smiths",
            body = "Junior asked to join",
            kind = "join_request",
            chatId = null,
        )

        assertThat(notification.number).isEqualTo(0)
    }

    // -- taking them down ---------------------------------------------------------------

    /**
     * By TAG, and a set at a time: a resync answers for the whole chat
     * list at once, and the ids are the system tray's when it built the
     * notification itself — only the tag is ever ours.
     */
    @Test
    fun cancelChatsSweepsExactlyTheTagsItWasGiven() {
        PushNotifications.ensureChannel(context)
        val manager = context.getSystemService(NotificationManager::class.java)
        for (chatId in listOf(1L, 42L, 77L)) {
            manager.notify(
                PushNotifications.chatTag(chatId),
                // A DIFFERENT id per chat, the way the system tray picks
                // its own: matching on the tag is what has to work.
                chatId.toInt(),
                PushNotifications.build(context, "Anna", "Hi", "message", chatId),
            )
        }

        PushNotifications.cancelChats(context, setOf(1L, 77L))

        assertThat(manager.activeNotifications.map { it.tag })
            .containsExactly(PushNotifications.chatTag(42L))
    }

    @Test
    fun cancelChatsWithNothingToCancelLeavesTheTrayAlone() {
        PushNotifications.ensureChannel(context)
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.notify(
            PushNotifications.chatTag(42L),
            PushNotifications.NOTIFICATION_ID,
            PushNotifications.build(context, "Anna", "Hi", "message", 42L),
        )

        PushNotifications.cancelChats(context, emptySet())

        assertThat(manager.activeNotifications).hasLength(1)
    }
}
