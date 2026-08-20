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
}
