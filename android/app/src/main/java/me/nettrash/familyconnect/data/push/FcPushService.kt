/*
 * FcPushService.kt
 * Family Connect (Android)
 *
 * The FCM entry point (registered in AndroidManifest.xml under
 * com.google.firebase.MESSAGING_EVENT). Two paths matter:
 *
 *   onNewToken /     — the OS/Firebase rotated the registration token;
 *   onRegistered       hand it to PushTokenRepository, which re-POSTs
 *                      /devices when a session exists and caches it
 *                      otherwise (protocol: "Clients re-POST whenever the
 *                      OS rotates the token").
 *   onMessageReceived— fires ONLY while the app is foregrounded for the
 *                      server's notification+data messages (backgrounded/
 *                      dead, the system tray renders them itself). The
 *                      server never pushes to a user with a live socket,
 *                      so a foreground delivery is a race — the push left
 *                      the server just as our socket came up. If the
 *                      socket is Open the frame is already (or about to
 *                      be) rendered in the UI: show nothing. Otherwise
 *                      (reconnect backoff, network flap) build the
 *                      notification manually from the payload.
 *
 * This whole class is inert in builds without a google-services.json:
 * FirebaseApp never initializes, so FCM never binds the service.
 *
 * iOS counterpart: none yet (push is not ported to ios/ at this time).
 */

package me.nettrash.familyconnect.data.push

import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import dagger.hilt.android.AndroidEntryPoint
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.SocketState
import me.nettrash.familyconnect.di.AppScope
import javax.inject.Inject

@AndroidEntryPoint
class FcPushService : FirebaseMessagingService() {

    @Inject
    lateinit var pushTokenRepository: PushTokenRepository

    @Inject
    lateinit var chatSocket: ChatSocket

    @Inject
    @AppScope
    lateinit var appScope: CoroutineScope

    // Deprecated alongside FirebaseMessaging.getToken() (the classic
    // registration-token flow) but still the callback that fires for it —
    // and the classic flow is the one the server sends to; see the
    // rationale in PushTokenProvider.kt.
    @Suppress("OVERRIDE_DEPRECATION")
    override fun onNewToken(token: String) {
        handleToken(token)
    }

    // The 25.x replacement callback (register()/installation-id flow). We
    // don't call register(), so today this never fires — wired to the same
    // idempotent path purely so a future SDK/server migration can't drop
    // tokens on the floor.
    override fun onRegistered(token: String) {
        handleToken(token)
    }

    private fun handleToken(token: String) {
        // App scope, not a service scope: FCM may destroy this service
        // long before the POST finishes, and losing the registration
        // wouldn't be fatal anyway — the next login/resync retries.
        appScope.launch { pushTokenRepository.onNewToken(token) }
    }

    override fun onMessageReceived(message: RemoteMessage) {
        // Foreground + live socket = the race described in the header;
        // the message frame renders in the open UI, a banner would dupe it.
        if (chatSocket.state.value == SocketState.Open) return

        val data = message.data
        val chatId = data[PushRouteParser.KEY_CHAT_ID]?.toLongOrNull()
        PushNotifications.show(
            context = this,
            // Title/body ride the notification block (protocol: FCM
            // messages are notification + data); the data payload has
            // only ids. Fall back to the server's own no-body wording.
            title = message.notification?.title ?: getString(R.string.app_name),
            body = message.notification?.body ?: "New message",
            kind = data[PushRouteParser.KEY_KIND],
            chatId = chatId,
        )
    }
}
