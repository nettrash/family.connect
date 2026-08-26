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
 *   onMessageReceived— for the server's notification+data messages, fires
 *                      ONLY while the app is foregrounded (backgrounded/
 *                      dead, the system tray renders them itself); for
 *                      the data-only `kind: "call"` message it fires in
 *                      EVERY state, which is the whole point of that
 *                      message being data-only (docs/protocol.md,
 *                      "Incoming calls"). The
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
import me.nettrash.familyconnect.calls.CallManager
import me.nettrash.familyconnect.calls.CallService
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
    lateinit var callManager: CallManager

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
        val data = message.data
        if (data[PushRouteParser.KEY_KIND] == PushRouteParser.KIND_CALL) {
            onIncomingCall(data)
            return
        }
        // Foreground + live socket = the race described in the header;
        // the message frame renders in the open UI, a banner would dupe it.
        if (chatSocket.state.value == SocketState.Open) return

        val chatId = data[PushRouteParser.KEY_CHAT_ID]?.toLongOrNull()
        PushNotifications.show(
            context = this,
            // Title/body ride the notification block (protocol: FCM
            // messages are notification + data); the data payload has
            // only ids. Fall back to the server's own no-body wording.
            title = message.notification?.title ?: getString(R.string.app_name),
            body = message.notification?.body ?: getString(R.string.s_new_message),
            kind = data[PushRouteParser.KEY_KIND],
            chatId = chatId,
            // The badge number, off the same `android.notification` block
            // the title and body come from — `notification_count`, the
            // recipient's unread in THIS chat (protocol.md, "Push
            // notifications"). Read here rather than computed, because
            // this is the RARE path: on the common one FCM has already
            // called setNumber with this same value before the app ever
            // hears about the message (PushNotifications' header), and
            // the two paths must not produce different numbers for the
            // same push. Null on a server that predates the field, and on
            // every push that is not about a chat.
            unreadInChat = message.notification?.notificationCount,
        )
    }

    /**
     * Somebody is calling (docs/protocol.md, "Incoming calls"). The
     * payload says WHO and WHICH call; the offer itself arrives over the
     * socket, replayed at registration — so the manager is told, which
     * makes it want the socket, and CallService is started HERE,
     * synchronously, while a high-priority push still allows a foreground
     * service to start from the background. With the socket already open
     * the offer frame is on its way (or here) and the manager holds the
     * call already; telling it again is a no-op.
     */
    private fun onIncomingCall(data: Map<String, String>) {
        val callId = data[PushRouteParser.KEY_CALL_ID] ?: return
        val chatId = data[PushRouteParser.KEY_CHAT_ID]?.toLongOrNull() ?: return
        val fromUserId = data[PushRouteParser.KEY_FROM_USER_ID]?.toLongOrNull() ?: return
        val callerName = data[PushRouteParser.KEY_CALLER_NAME]?.takeIf { it.isNotBlank() }
        callManager.onIncomingPush(callId, chatId, fromUserId, callerName)
        runCatching { startForegroundService(CallService.intent(this)) }
    }
}
