/*
 * MainActivity.kt
 * Family Connect (Android)
 *
 * The single Activity. Edge-to-edge; waits for MainViewModel's boot
 * snapshot (spinner while null), then composes the theme + NavHost
 * exactly once with the start destination the snapshot dictates. All
 * later routing is event-driven inside AppNavHost.
 *
 * Push taps land here as intent extras carrying the FCM data-payload
 * keys ("kind", "chat_id") — identically for a system-tray notification
 * (FCM copies the data payload onto the launcher intent) and for one the
 * app built itself (PushNotifications uses the same keys on purpose).
 * Cold start reads them in onCreate; a tap while running arrives via
 * onNewIntent (manifest launchMode singleTop). Either way they parse to
 * a PendingRoute held in MainViewModel until AppNavHost consumes it.
 *
 * iOS counterpart: ios/FamilyConnect/App/RootView.swift
 */

package me.nettrash.familyconnect

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dagger.hilt.android.AndroidEntryPoint
import me.nettrash.familyconnect.calls.CallNotifications
import me.nettrash.familyconnect.data.push.PendingRoute
import me.nettrash.familyconnect.data.push.PushNotifications
import me.nettrash.familyconnect.data.push.PushRouteParser
import me.nettrash.familyconnect.navigation.AppNavHost
import androidx.core.content.IntentCompat
import me.nettrash.familyconnect.data.repo.AttachmentRepository
import me.nettrash.familyconnect.data.repo.AvatarRepository
import me.nettrash.familyconnect.data.repo.ShareIn
import me.nettrash.familyconnect.navigation.startDestinationFor
import me.nettrash.familyconnect.ui.components.LocalAttachments
import me.nettrash.familyconnect.ui.components.LocalAvatars
import me.nettrash.familyconnect.ui.theme.FamilyConnectTheme
import javax.inject.Inject

@AndroidEntryPoint
class MainActivity : ComponentActivity() {

    private val viewModel: MainViewModel by viewModels()

    /**
     * App-scoped so profile pictures survive navigation; handed to the
     * whole tree through LocalAvatars rather than threaded through every
     * screen's ViewModel.
     */
    @Inject
    lateinit var avatars: AvatarRepository

    /** Photos and previews, cached on disk; provided the same way. */
    @Inject
    lateinit var attachments: AttachmentRepository

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        // Cold start only (savedInstanceState == null): on a recreation
        // (rotation, process restore) the same intent is redelivered and
        // must not re-fire an already-consumed deep link.
        if (savedInstanceState == null) {
            handlePushIntent(intent)
            handleShareIntent(intent)
        }
        showOverLockScreenForCall(intent)
        setContent {
            CompositionLocalProvider(
                LocalAvatars provides avatars,
                LocalAttachments provides attachments,
            ) {
                val boot by viewModel.bootState.collectAsStateWithLifecycle()
                val pendingRoute by viewModel.pendingRoute.collectAsStateWithLifecycle()
                // What the share flow has to say — one sentence, once. A
                // toast rather than UI state: the share may be refused
                // before there is any family UI to say it in.
                val shareNotice by viewModel.shareNotice.collectAsStateWithLifecycle()
                LaunchedEffect(shareNotice) {
                    shareNotice?.let { notice ->
                        val text = notice.arg
                            ?.let { getString(notice.resId, it) }
                            ?: getString(notice.resId)
                        Toast.makeText(this@MainActivity, text, Toast.LENGTH_LONG).show()
                        viewModel.consumeShareNotice()
                    }
                }
                val snapshot = boot
                if (snapshot == null) {
                    // Themed boot chrome: without FamilyConnectTheme the first
                    // frame renders Compose's baseline purple on the raw window
                    // background before the real UI appears.
                    FamilyConnectTheme {
                        Surface(
                            modifier = Modifier.fillMaxSize(),
                            color = MaterialTheme.colorScheme.background,
                        ) {
                            Column(
                                modifier = Modifier.fillMaxSize(),
                                verticalArrangement = Arrangement.Center,
                                horizontalAlignment = Alignment.CenterHorizontally,
                            ) {
                                CircularProgressIndicator()
                                Spacer(modifier = Modifier.height(12.dp))
                                Text(
                                    text = stringResource(R.string.s_connecting),
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }
                } else {
                    FamilyConnectTheme {
                        // remember → the NavHost keeps its original start
                        // destination even if recomposition delivers a newer
                        // snapshot; reroutes go through session events.
                        val start = remember { startDestinationFor(snapshot.status) }
                        AppNavHost(
                            startDestination = start,
                            sessionEvents = viewModel.sessionEvents,
                            pendingRoute = pendingRoute,
                            onPendingRouteConsumed = viewModel::consumePendingRoute,
                            isOwner = snapshot.isOwner,
                            callState = viewModel.callState,
                            shareFlow = viewModel.shareFlow,
                            onShareChatChosen = viewModel::shareChatChosen,
                            onShareCancelled = viewModel::cancelShare,
                            sessionStatus = viewModel.sessionStatus,
                        )
                    }
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handlePushIntent(intent)
        handleShareIntent(intent)
        showOverLockScreenForCall(intent)
    }

    private fun handlePushIntent(intent: Intent?) {
        intent ?: return
        val data = buildMap {
            intent.getStringExtra(PushNotifications.EXTRA_KIND)
                ?.let { put(PushNotifications.EXTRA_KIND, it) }
            intent.getStringExtra(PushNotifications.EXTRA_CHAT_ID)
                ?.let { put(PushNotifications.EXTRA_CHAT_ID, it) }
            intent.getStringExtra(PushRouteParser.KEY_CALL_ACTION)
                ?.let { put(PushRouteParser.KEY_CALL_ACTION, it) }
        }
        val route = PushRouteParser.parse(data) ?: return
        if (route is PendingRoute.Call && route.answer) viewModel.requestAnswer()
        viewModel.onPendingRoute(route)
    }

    /**
     * Another app shared something here (ACTION_SEND / ACTION_SEND_MULTIPLE).
     *
     * Only the DESCRIPTIONS are built on this thread — scheme and media
     * type per stream, which is what ShareIn decides over. The bytes are
     * copied off-main by the ViewModel, and immediately: the read grants
     * on shared Uris are transient.
     */
    private fun handleShareIntent(intent: Intent?) {
        intent ?: return
        val action = intent.action
        if (action != Intent.ACTION_SEND && action != Intent.ACTION_SEND_MULTIPLE) return
        val uris: List<Uri> = when (action) {
            Intent.ACTION_SEND -> listOfNotNull(
                IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java),
            )
            else -> IntentCompat.getParcelableArrayListExtra(
                intent,
                Intent.EXTRA_STREAM,
                Uri::class.java,
            ).orEmpty()
        }
        val streams = uris.map { uri ->
            ShareIn.Stream(
                scheme = uri.scheme,
                // The provider's own answer first — it describes THIS item,
                // where the intent's type describes the whole share.
                // Guarded: `getType` reaches into another app's provider
                // and throws for a Uri whose grant has lapsed.
                mime = runCatching { contentResolver.getType(uri) }.getOrNull() ?: intent.type,
            )
        }
        viewModel.onShared(uris, streams, intent.getStringExtra(Intent.EXTRA_TEXT))
    }

    /**
     * A ringing call's full-screen intent (or its tap) lands on a phone
     * that may be locked and dark: show over the lock screen and wake it,
     * the way the dialler does. Only for a call — a message tap on a
     * locked phone should still ask for the unlock.
     */
    private fun showOverLockScreenForCall(intent: Intent?) {
        if (!CallNotifications.isCallIntent(intent)) return
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O_MR1) {
            setShowWhenLocked(true)
            setTurnScreenOn(true)
        }
    }
}
