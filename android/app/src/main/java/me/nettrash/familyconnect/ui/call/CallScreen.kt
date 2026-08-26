/*
 * CallScreen.kt
 * Family Connect (Android)
 *
 * The one screen a voice call has: the other person's face and name, a
 * status line (calling, ringing, connecting, the timer, or why it ended),
 * and the controls — mute, speaker and hang up on a call; decline and
 * accept while one rings in. Navigated to by AppNavHost whenever
 * CallManager leaves Idle, and popped when it returns there.
 *
 * Accepting needs the microphone, asked for here at the moment of use
 * exactly as the voice-note button asks — and the grant answers the call.
 */

package me.nettrash.familyconnect.ui.call

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.CallEnd
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.MicOff
import androidx.compose.material.icons.filled.VolumeUp
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.delay
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.calls.CallEnding
import me.nettrash.familyconnect.calls.CallState
import me.nettrash.familyconnect.ui.chat.CallRecordWording
import me.nettrash.familyconnect.ui.components.Avatar

@Composable
fun CallScreen(
    viewModel: CallViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val peer by viewModel.peer.collectAsStateWithLifecycle()
    val isMuted by viewModel.isMuted.collectAsStateWithLifecycle()
    val isSpeaker by viewModel.isSpeaker.collectAsStateWithLifecycle()
    val answerRequested by viewModel.answerRequested.collectAsStateWithLifecycle()
    val context = LocalContext.current

    val micPermission = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        if (granted) viewModel.accept() else viewModel.decline()
    }
    val accept: () -> Unit = {
        val held = ContextCompat.checkSelfPermission(
            context,
            Manifest.permission.RECORD_AUDIO,
        ) == PackageManager.PERMISSION_GRANTED
        if (held) viewModel.accept() else micPermission.launch(Manifest.permission.RECORD_AUDIO)
    }

    // The notification's Answer button landed here: answer, through the
    // permission flow, as if the on-screen button had been tapped.
    LaunchedEffect(answerRequested, state is CallState.Incoming) {
        if (answerRequested && state is CallState.Incoming) accept()
    }

    Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .systemBarsPadding()
                .padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.SpaceBetween,
        ) {
            Spacer(Modifier.height(24.dp))
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                peer?.let { person ->
                    Avatar(
                        name = person.name,
                        userId = person.userId,
                        size = 112,
                        avatarVersion = person.avatarVersion,
                    )
                    Spacer(Modifier.height(20.dp))
                    Text(
                        text = person.name,
                        style = MaterialTheme.typography.headlineMedium,
                        textAlign = TextAlign.Center,
                    )
                }
                Spacer(Modifier.height(8.dp))
                Text(
                    text = statusLine(state),
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
            }

            when (state) {
                is CallState.Incoming -> IncomingControls(onDecline = viewModel::decline, onAccept = accept)
                is CallState.Outgoing, is CallState.Connecting, is CallState.Active -> ActiveControls(
                    isMuted = isMuted,
                    isSpeaker = isSpeaker,
                    onToggleMute = viewModel::toggleMute,
                    onToggleSpeaker = viewModel::toggleSpeaker,
                    onHangUp = viewModel::hangUp,
                )
                CallState.Idle, is CallState.Ended -> Spacer(Modifier.height(72.dp))
            }
        }
    }
}

/** What the line under the name says for [state] — the timer ticks once a second while Active. */
@Composable
private fun statusLine(state: CallState): String = when (state) {
    CallState.Idle -> ""
    is CallState.Outgoing -> stringResource(if (state.ringing) R.string.s_ringing else R.string.s_calling)
    is CallState.Incoming -> stringResource(R.string.s_incoming_voice_call)
    is CallState.Connecting -> stringResource(R.string.s_connecting)
    is CallState.Active -> {
        var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
        LaunchedEffect(state.sinceMillis) {
            while (true) {
                now = System.currentTimeMillis()
                delay(1_000L)
            }
        }
        CallRecordWording.duration(((now - state.sinceMillis) / 1000).toInt())
    }
    is CallState.Ended -> when (state.reason) {
        CallEnding.HANGUP -> state.durationSecs
            ?.let { stringResource(R.string.s_voice_call_with_duration, CallRecordWording.duration(it)) }
            ?: stringResource(R.string.s_call_ended)
        CallEnding.DECLINE -> stringResource(R.string.s_declined_voice_call)
        CallEnding.CANCEL -> stringResource(R.string.s_call_ended)
        CallEnding.TIMEOUT -> stringResource(R.string.s_no_answer)
        CallEnding.FAILED -> stringResource(R.string.s_call_failed)
        CallEnding.ANSWERED_ELSEWHERE -> stringResource(R.string.s_answered_on_another_device)
        CallEnding.BUSY, CallEnding.PEER_BUSY -> stringResource(R.string.s_busy)
        CallEnding.PEER_UNREACHABLE -> stringResource(R.string.s_not_reachable)
        CallEnding.NO_OFFER -> stringResource(R.string.s_call_ended)
        CallEnding.DISABLED -> stringResource(R.string.s_calls_are_off_on_this_server)
    }
}

@Composable
private fun IncomingControls(onDecline: () -> Unit, onAccept: () -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceEvenly,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RoundButton(
            icon = Icons.Filled.CallEnd,
            label = stringResource(R.string.s_decline),
            container = MaterialTheme.colorScheme.error,
            content = MaterialTheme.colorScheme.onError,
            onClick = onDecline,
        )
        RoundButton(
            icon = Icons.Filled.Call,
            label = stringResource(R.string.s_answer),
            container = Color(0xFF2E7D32),
            content = Color.White,
            onClick = onAccept,
        )
    }
}

@Composable
private fun ActiveControls(
    isMuted: Boolean,
    isSpeaker: Boolean,
    onToggleMute: () -> Unit,
    onToggleSpeaker: () -> Unit,
    onHangUp: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceEvenly,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RoundButton(
            icon = if (isMuted) Icons.Filled.MicOff else Icons.Filled.Mic,
            label = stringResource(if (isMuted) R.string.s_unmute else R.string.s_mute),
            container = if (isMuted) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,
            content = if (isMuted) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant,
            onClick = onToggleMute,
        )
        RoundButton(
            icon = Icons.Filled.CallEnd,
            label = stringResource(R.string.s_hang_up),
            container = MaterialTheme.colorScheme.error,
            content = MaterialTheme.colorScheme.onError,
            onClick = onHangUp,
        )
        RoundButton(
            icon = Icons.Filled.VolumeUp,
            label = stringResource(R.string.s_speaker),
            container = if (isSpeaker) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,
            content = if (isSpeaker) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant,
            onClick = onToggleSpeaker,
        )
    }
}

@Composable
private fun RoundButton(
    icon: ImageVector,
    label: String,
    container: Color,
    content: Color,
    onClick: () -> Unit,
) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Box(
            modifier = Modifier
                .size(72.dp)
                .background(Color.Transparent, CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            FilledIconButton(
                onClick = onClick,
                modifier = Modifier.size(72.dp),
                colors = IconButtonDefaults.filledIconButtonColors(
                    containerColor = container,
                    contentColor = content,
                ),
            ) {
                Icon(icon, contentDescription = label, modifier = Modifier.size(32.dp))
            }
        }
        Spacer(Modifier.height(8.dp))
        Text(text = label, style = MaterialTheme.typography.labelMedium)
    }
}
