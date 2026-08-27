/*
 * CallScreen.kt
 * Family Connect (Android)
 *
 * The one screen a call has — voice or video, the KIND fixed when the
 * call was placed (docs/protocol.md, "Video"). Voice: the other person's
 * face and name, a status line, and mute / speaker / hang up. Video adds
 * the far side full-bleed behind everything, a local preview pinned
 * top-end, and camera controls — with the avatar and status still shown
 * until the remote picture is actually flowing. Navigated to by
 * AppNavHost whenever CallManager leaves Idle, and popped when it
 * returns there.
 *
 * Accepting needs the microphone, asked for here at the moment of use
 * exactly as the voice-note button asks — and the grant answers the call.
 * A VIDEO call asks for the camera at the same time, but only the
 * microphone decides: a denied camera still answers, camera off — the
 * protocol's rule, so a privacy setting never turns into missed calls.
 *
 * The SurfaceViewRenderers are created against the ONE EglBase context
 * the WebRTC factory owns, registered as sinks with the manager, and
 * RELEASED here (AndroidView.onRelease) — the media client detaches
 * sinks on close but never releases a renderer; that is the UI's job.
 */

package me.nettrash.familyconnect.ui.call

import android.Manifest
import android.content.pm.PackageManager
import android.widget.Toast
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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.CallEnd
import androidx.compose.material.icons.filled.FlipCameraAndroid
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.MicOff
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material.icons.filled.VideocamOff
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.delay
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.calls.CallEnding
import me.nettrash.familyconnect.calls.CallState
import me.nettrash.familyconnect.ui.chat.CallRecordWording
import me.nettrash.familyconnect.ui.components.Avatar
import org.webrtc.RendererCommon
import org.webrtc.SurfaceViewRenderer

@Composable
fun CallScreen(
    viewModel: CallViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val peer by viewModel.peer.collectAsStateWithLifecycle()
    val isMuted by viewModel.isMuted.collectAsStateWithLifecycle()
    val isSpeaker by viewModel.isSpeaker.collectAsStateWithLifecycle()
    val isCameraOn by viewModel.isCameraOn.collectAsStateWithLifecycle()
    val isFrontCamera by viewModel.isFrontCamera.collectAsStateWithLifecycle()
    val remoteVideoActive by viewModel.remoteVideoActive.collectAsStateWithLifecycle()
    val answerRequested by viewModel.answerRequested.collectAsStateWithLifecycle()
    val context = LocalContext.current

    val live = state as? CallState.Live
    val isVideoCall = live?.video ?: ((state as? CallState.Ended)?.video ?: false)

    val micPermission = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        if (granted) viewModel.accept() else viewModel.decline()
    }

    // A video call asks for both at once — the MICROPHONE decides
    // (denied = decline, as for a voice call); a denied CAMERA still
    // answers, camera off (docs/protocol.md, "Video"). The camera's
    // verdict rides INTO the answer: granting it in this dialog starts
    // the call with the camera ON, exactly as iOS does — accept() sets
    // the state before the media opens, so the grant is not lost to the
    // camera-off enforcement that ran while the call was still ringing.
    val videoAcceptPermissions = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) { grants ->
        if (grants[Manifest.permission.RECORD_AUDIO] == true) {
            val cameraGranted = grants[Manifest.permission.CAMERA] == true
            if (!cameraGranted) {
                Toast.makeText(context, R.string.e_camera_permission, Toast.LENGTH_LONG).show()
            }
            viewModel.accept(cameraGranted = cameraGranted)
        } else {
            viewModel.decline()
        }
    }

    val accept: () -> Unit = {
        val micHeld = ContextCompat.checkSelfPermission(
            context,
            Manifest.permission.RECORD_AUDIO,
        ) == PackageManager.PERMISSION_GRANTED
        if ((state as? CallState.Incoming)?.video == true) {
            val camHeld = ContextCompat.checkSelfPermission(
                context,
                Manifest.permission.CAMERA,
            ) == PackageManager.PERMISSION_GRANTED
            if (micHeld && camHeld) {
                viewModel.accept()
            } else {
                videoAcceptPermissions.launch(
                    arrayOf(Manifest.permission.RECORD_AUDIO, Manifest.permission.CAMERA),
                )
            }
        } else {
            if (micHeld) viewModel.accept() else micPermission.launch(Manifest.permission.RECORD_AUDIO)
        }
    }

    // The notification's Answer button landed here: answer, through the
    // permission flow, as if the on-screen button had been tapped.
    LaunchedEffect(answerRequested, state is CallState.Incoming) {
        if (answerRequested && state is CallState.Incoming) accept()
    }

    // A video call without the camera grant runs with the camera OFF —
    // still placed, still answered (docs/protocol.md, "Video"). Enforced
    // here so every way into a call (button, notification, push) ends
    // honest; WebRtcClient additionally refuses to capture without the
    // grant, so this is about the toggle telling the truth.
    LaunchedEffect(isVideoCall, live != null) {
        if (isVideoCall && live != null) {
            val camHeld = ContextCompat.checkSelfPermission(
                context,
                Manifest.permission.CAMERA,
            ) == PackageManager.PERMISSION_GRANTED
            if (!camHeld) viewModel.setCameraEnabled(false)
        }
    }

    // Turning the camera ON may be the first time the grant is asked for
    // (a call answered camera-off, then reconsidered).
    val cameraTogglePermission = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        if (granted) {
            viewModel.toggleCamera()
        } else {
            Toast.makeText(context, R.string.e_camera_permission, Toast.LENGTH_LONG).show()
        }
    }
    val toggleCamera: () -> Unit = {
        val camHeld = ContextCompat.checkSelfPermission(
            context,
            Manifest.permission.CAMERA,
        ) == PackageManager.PERMISSION_GRANTED
        if (isCameraOn || camHeld) {
            viewModel.toggleCamera()
        } else {
            cameraTogglePermission.launch(Manifest.permission.CAMERA)
        }
    }

    Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        Box(modifier = Modifier.fillMaxSize()) {
            if (isVideoCall && live != null) {
                // The far side, full-bleed behind everything. Black until
                // frames arrive; the avatar and status stay drawn over it
                // until they do.
                RemoteVideo(
                    viewModel = viewModel,
                    modifier = Modifier.fillMaxSize(),
                )
                if (isCameraOn) {
                    LocalPreview(
                        viewModel = viewModel,
                        isFrontCamera = isFrontCamera,
                        modifier = Modifier
                            .align(Alignment.TopEnd)
                            .systemBarsPadding()
                            .padding(16.dp)
                            .size(width = 110.dp, height = 150.dp)
                            .clip(RoundedCornerShape(12.dp)),
                    )
                }
            }

            // True once the remote picture is actually flowing — the
            // moment the identity block steps aside (and turns white,
            // since it now sits on video, not on the theme background).
            val overVideo = isVideoCall && remoteVideoActive && live != null

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
                        if (!overVideo) {
                            Avatar(
                                name = person.name,
                                userId = person.userId,
                                size = 112,
                                avatarVersion = person.avatarVersion,
                            )
                            Spacer(Modifier.height(20.dp))
                        }
                        Text(
                            text = person.name,
                            style = MaterialTheme.typography.headlineMedium,
                            color = if (overVideo) Color.White else Color.Unspecified,
                            textAlign = TextAlign.Center,
                        )
                    }
                    Spacer(Modifier.height(8.dp))
                    Text(
                        text = statusLine(state),
                        style = MaterialTheme.typography.bodyLarge,
                        color = if (overVideo) Color.White else MaterialTheme.colorScheme.onSurfaceVariant,
                        textAlign = TextAlign.Center,
                    )
                }

                when (state) {
                    is CallState.Incoming -> IncomingControls(onDecline = viewModel::decline, onAccept = accept)
                    is CallState.Outgoing, is CallState.Connecting, is CallState.Active -> ActiveControls(
                        isMuted = isMuted,
                        isSpeaker = isSpeaker,
                        isVideoCall = isVideoCall,
                        isCameraOn = isCameraOn,
                        onToggleMute = viewModel::toggleMute,
                        onToggleSpeaker = viewModel::toggleSpeaker,
                        onToggleCamera = toggleCamera,
                        onFlipCamera = viewModel::flipCamera,
                        onHangUp = viewModel::hangUp,
                    )
                    CallState.Idle, is CallState.Ended -> Spacer(Modifier.height(72.dp))
                }
            }
        }
    }
}

/**
 * The far side's picture. init against the factory's ONE EglBase context,
 * registered as the remote sink; onRelease detaches the sink FIRST, then
 * releases the renderer — the reverse order draws into freed GL state.
 */
@Composable
private fun RemoteVideo(
    viewModel: CallViewModel,
    modifier: Modifier = Modifier,
) {
    AndroidView(
        modifier = modifier,
        factory = { ctx ->
            SurfaceViewRenderer(ctx).apply {
                init(viewModel.eglBaseContext, null)
                setEnableHardwareScaler(true)
                setScalingType(RendererCommon.ScalingType.SCALE_ASPECT_FILL)
                viewModel.setRemoteVideoSink(this)
            }
        },
        onRelease = { renderer ->
            viewModel.setRemoteVideoSink(null)
            renderer.release()
        },
    )
}

/** The local camera, picture-in-picture. Mirrored while the FRONT camera captures. */
@Composable
private fun LocalPreview(
    viewModel: CallViewModel,
    isFrontCamera: Boolean,
    modifier: Modifier = Modifier,
) {
    AndroidView(
        modifier = modifier,
        factory = { ctx ->
            SurfaceViewRenderer(ctx).apply {
                init(viewModel.eglBaseContext, null)
                setEnableHardwareScaler(true)
                setScalingType(RendererCommon.ScalingType.SCALE_ASPECT_FILL)
                // Two SurfaceViews otherwise fight over one Z slot, and
                // the preview loses — this puts it above the remote view.
                setZOrderMediaOverlay(true)
                viewModel.setLocalVideoSink(this)
            }
        },
        update = { renderer -> renderer.setMirror(isFrontCamera) },
        onRelease = { renderer ->
            viewModel.setLocalVideoSink(null)
            renderer.release()
        },
    )
}

/** What the line under the name says for [state] — the timer ticks once a second while Active. */
@Composable
private fun statusLine(state: CallState): String = when (state) {
    CallState.Idle -> ""
    is CallState.Outgoing -> stringResource(if (state.ringing) R.string.s_ringing else R.string.s_calling)
    is CallState.Incoming -> stringResource(
        if (state.video) R.string.s_incoming_video_call else R.string.s_incoming_voice_call,
    )
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
            ?.let {
                stringResource(
                    if (state.video) R.string.s_video_call_with_duration else R.string.s_voice_call_with_duration,
                    CallRecordWording.duration(it),
                )
            }
            ?: stringResource(R.string.s_call_ended)
        CallEnding.DECLINE -> stringResource(
            if (state.video) R.string.s_declined_video_call else R.string.s_declined_voice_call,
        )
        CallEnding.CANCEL -> stringResource(R.string.s_call_ended)
        CallEnding.TIMEOUT -> stringResource(R.string.s_no_answer)
        CallEnding.FAILED -> stringResource(R.string.s_call_failed)
        CallEnding.ANSWERED_ELSEWHERE -> stringResource(R.string.s_answered_on_another_device)
        CallEnding.BUSY, CallEnding.PEER_BUSY -> stringResource(R.string.s_busy)
        CallEnding.PEER_UNREACHABLE -> stringResource(R.string.s_not_reachable)
        CallEnding.NO_OFFER -> stringResource(R.string.s_call_ended)
        CallEnding.DISABLED -> stringResource(R.string.s_calls_are_off_on_this_server)
        CallEnding.VIDEO_DISABLED -> stringResource(R.string.s_video_calls_are_off_on_this_server)
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
    isVideoCall: Boolean,
    isCameraOn: Boolean,
    onToggleMute: () -> Unit,
    onToggleSpeaker: () -> Unit,
    onToggleCamera: () -> Unit,
    onFlipCamera: () -> Unit,
    onHangUp: () -> Unit,
) {
    // Up to five buttons on a video call — smaller discs, or they do not
    // fit a narrow phone side by side.
    val buttonSize = if (isVideoCall) 60 else 72
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
            size = buttonSize,
            onClick = onToggleMute,
        )
        if (isVideoCall) {
            // The label names the ACTION, like Mute/Unmute beside it.
            RoundButton(
                icon = if (isCameraOn) Icons.Filled.Videocam else Icons.Filled.VideocamOff,
                label = stringResource(if (isCameraOn) R.string.s_camera_off else R.string.s_camera_on),
                container = if (!isCameraOn) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,
                content = if (!isCameraOn) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant,
                size = buttonSize,
                onClick = onToggleCamera,
            )
            if (isCameraOn) {
                RoundButton(
                    icon = Icons.Filled.FlipCameraAndroid,
                    label = stringResource(R.string.s_flip_camera),
                    container = MaterialTheme.colorScheme.surfaceVariant,
                    content = MaterialTheme.colorScheme.onSurfaceVariant,
                    size = buttonSize,
                    onClick = onFlipCamera,
                )
            }
        }
        RoundButton(
            icon = Icons.Filled.CallEnd,
            label = stringResource(R.string.s_hang_up),
            container = MaterialTheme.colorScheme.error,
            content = MaterialTheme.colorScheme.onError,
            size = buttonSize,
            onClick = onHangUp,
        )
        RoundButton(
            icon = Icons.Filled.VolumeUp,
            label = stringResource(R.string.s_speaker),
            container = if (isSpeaker) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,
            content = if (isSpeaker) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant,
            size = buttonSize,
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
    size: Int = 72,
    onClick: () -> Unit,
) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Box(
            modifier = Modifier
                .size(size.dp)
                .background(Color.Transparent, CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            FilledIconButton(
                onClick = onClick,
                modifier = Modifier.size(size.dp),
                colors = IconButtonDefaults.filledIconButtonColors(
                    containerColor = container,
                    contentColor = content,
                ),
            ) {
                Icon(icon, contentDescription = label, modifier = Modifier.size((size * 32 / 72).dp))
            }
        }
        Spacer(Modifier.height(8.dp))
        Text(text = label, style = MaterialTheme.typography.labelMedium)
    }
}
