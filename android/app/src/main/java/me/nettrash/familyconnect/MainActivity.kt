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
import android.os.Bundle
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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dagger.hilt.android.AndroidEntryPoint
import me.nettrash.familyconnect.data.push.PushNotifications
import me.nettrash.familyconnect.data.push.PushRouteParser
import me.nettrash.familyconnect.navigation.AppNavHost
import me.nettrash.familyconnect.data.repo.AvatarRepository
import me.nettrash.familyconnect.navigation.startDestinationFor
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

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        // Cold start only (savedInstanceState == null): on a recreation
        // (rotation, process restore) the same intent is redelivered and
        // must not re-fire an already-consumed deep link.
        if (savedInstanceState == null) {
            handlePushIntent(intent)
        }
        setContent {
            CompositionLocalProvider(LocalAvatars provides avatars) {
                val boot by viewModel.bootState.collectAsStateWithLifecycle()
                val pendingRoute by viewModel.pendingRoute.collectAsStateWithLifecycle()
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
                                    text = "Connecting…",
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
                        )
                    }
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handlePushIntent(intent)
    }

    private fun handlePushIntent(intent: Intent?) {
        intent ?: return
        val data = buildMap {
            intent.getStringExtra(PushNotifications.EXTRA_KIND)
                ?.let { put(PushNotifications.EXTRA_KIND, it) }
            intent.getStringExtra(PushNotifications.EXTRA_CHAT_ID)
                ?.let { put(PushNotifications.EXTRA_CHAT_ID, it) }
        }
        PushRouteParser.parse(data)?.let(viewModel::onPendingRoute)
    }
}
