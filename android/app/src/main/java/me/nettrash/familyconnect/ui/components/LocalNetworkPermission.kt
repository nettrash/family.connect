/*
 * LocalNetworkPermission.kt
 * Family Connect (Android)
 *
 * The two moments Android 17's local network grant is asked for (see
 * data/net/LocalNetworkAccess.kt): a family server on the home network,
 * and a call, whose two ends may share a Wi-Fi. Neither blocks anything:
 * a refusal leaves the server unreachable from this phone — the person's
 * call, made in the system's own words — and a call on its internet path.
 * Both no-ops before Android 17, once granted, and after one ask a process.
 */

package me.nettrash.familyconnect.ui.components

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.platform.LocalContext
import me.nettrash.familyconnect.data.net.LocalNetworkAccess

/** Asks once, when the family's server at [serverUrl] is on the local network. */
@Composable
fun LocalNetworkPermissionForServer(serverUrl: String?) {
    val context = LocalContext.current
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { }
    LaunchedEffect(serverUrl) {
        val url = serverUrl ?: return@LaunchedEffect
        if (!LocalNetworkAccess.needsGrant(context)) return@LaunchedEffect
        if (LocalNetworkAccess.serverIsLocal(url) && LocalNetworkAccess.claimAsk(context)) {
            launcher.launch(LocalNetworkAccess.PERMISSION)
        }
    }
}

/**
 * Asks once, when a call is under way ([underWay]: placed, or answered) —
 * after the microphone's own prompt, never on top of it.
 */
@Composable
fun LocalNetworkPermissionForCall(underWay: Boolean) {
    val context = LocalContext.current
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { }
    LaunchedEffect(underWay) {
        if (underWay && LocalNetworkAccess.claimAsk(context)) {
            launcher.launch(LocalNetworkAccess.PERMISSION)
        }
    }
}
