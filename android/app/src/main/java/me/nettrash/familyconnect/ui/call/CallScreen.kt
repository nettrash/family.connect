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
 *
 * A video call is drawn DARK whatever the system theme (the app theme is
 * nested here, forced dark), on a black ground, and holds the screen
 * awake; a voice call follows the system and sleeps like a phone call.
 */

package me.nettrash.familyconnect.ui.call

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.PackageManager
import android.view.WindowManager
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
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
import androidx.compose.runtime.DisposableEffect
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
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.delay
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.calls.CallEnding
import me.nettrash.familyconnect.calls.CallNotifications
import me.nettrash.familyconnect.calls.CallState
import me.nettrash.familyconnect.calls.CallVideoLog
import me.nettrash.familyconnect.ui.chat.CallRecordWording
import me.nettrash.familyconnect.ui.components.Avatar
import me.nettrash.familyconnect.ui.theme.FamilyConnectTheme
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

    // A video call is DARK whatever the system theme, like the iPhone's:
    // the far side's SurfaceViewRenderer is black until the first frame
    // arrives — and for the whole call when their camera is off — and in
    // the light theme the name, the status and the captions were
    // onSurface, black on black, on every incoming video call among
    // others. The app's one theme composable, nested and forced dark
    // while the call is video; a voice call keeps following the system.
    FamilyConnectTheme(darkTheme = isVideoCall || isSystemInDarkTheme()) {
        // Only a VIDEO call holds the screen awake: the picture is the
        // point, and nobody touches the phone for minutes. A voice call
        // held to the ear must still go dark as any phone call does —
        // nothing else in the app holds this flag, so clearing it on the
        // way out hands the screen back to the system untouched.
        val keepAwake = isVideoCall && live != null
        DisposableEffect(keepAwake) {
            val window = if (keepAwake) context.findActivity()?.window else null
            window?.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            onDispose { window?.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) }
        }
        Surface(
            modifier = Modifier.fillMaxSize(),
            // Black under the renderer, not the dark scheme's surface: the
            // strip beside a letterboxed picture would otherwise be a
            // slightly different dark than the renderer's own black.
            color = if (isVideoCall) Color.Black else MaterialTheme.colorScheme.background,
        ) {
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
                // moment the identity block steps aside into its pill (and
                // turns white, since it now sits on a picture that may be
                // any colour, not on the black ground) and the control row
                // takes its scrim, for the same reason.
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
                    // Over the picture the identity is a top-START pill — the
                    // iPhone's — kept clear of the local preview pinned
                    // top-end: centred, any name wider than ~108 dp on a
                    // 360 dp phone ran under the preview, on the raw picture,
                    // with nothing behind it. Centred over the avatar
                    // otherwise, exactly as before.
                    val identityModifier = if (overVideo) {
                        Modifier
                            .align(Alignment.Start)
                            // The preview's width plus its inset, so the pill
                            // ends before the preview begins.
                            .padding(end = if (isCameraOn) 126.dp else 0.dp)
                            .background(Color.Black.copy(alpha = 0.35f), RoundedCornerShape(20.dp))
                            .padding(horizontal = 14.dp, vertical = 8.dp)
                    } else {
                        Modifier
                    }
                    Column(
                        modifier = identityModifier,
                        horizontalAlignment = if (overVideo) Alignment.Start else Alignment.CenterHorizontally,
                    ) {
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
                                textAlign = if (overVideo) TextAlign.Start else TextAlign.Center,
                                // One line in the pill; a long name ends in
                                // an ellipsis rather than growing the pill
                                // into the preview's row.
                                maxLines = if (overVideo) 1 else Int.MAX_VALUE,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                        Spacer(Modifier.height(8.dp))
                        Text(
                            text = statusLine(state),
                            style = MaterialTheme.typography.bodyLarge,
                            color = if (overVideo) Color.White else MaterialTheme.colorScheme.onSurfaceVariant,
                            textAlign = if (overVideo) TextAlign.Start else TextAlign.Center,
                        )
                    }

                    when (state) {
                        is CallState.Incoming -> IncomingControls(onDecline = viewModel::decline, onAccept = accept)
                        is CallState.Outgoing, is CallState.Connecting, is CallState.Active -> ActiveControls(
                            isMuted = isMuted,
                            isSpeaker = isSpeaker,
                            isVideoCall = isVideoCall,
                            isCameraOn = isCameraOn,
                            overVideo = overVideo,
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
}

/**
 * The Activity behind a Compose context. LocalContext is seldom the
 * Activity itself (a theme or configuration wrapper, usually), so this
 * walks the ContextWrapper chain until it finds one — or gives up, in
 * which case there is no window to keep awake and nothing to clean up.
 */
private fun Context.findActivity(): Activity? {
    var current: Context? = this
    while (current is ContextWrapper) {
        if (current is Activity) return current
        current = current.baseContext
    }
    return null
}

/**
 * The far side's picture. init against the factory's ONE EglBase context,
 * registered as the remote sink; onRelease detaches the sink FIRST, then
 * releases the renderer — the reverse order draws into freed GL state.
 *
 * The detach names THIS renderer rather than clearing whatever is
 * registered. Two compositions can be alive at once — an Activity
 * recreated mid-call builds the new tree before it disposes the old one —
 * and a blind clear from the dying surface would unhook the live one,
 * leaving the call with no remote picture for the rest of its life and
 * nothing to re-attach it. The manager ignores a release that is not the
 * current sink and logs that it did.
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
                CallVideoLog.event("remote surface created renderer=${CallVideoLog.id(this)}")
                viewModel.setRemoteVideoSink(this)
            }
        },
        onRelease = { renderer ->
            CallVideoLog.event("remote surface released renderer=${CallVideoLog.id(renderer)}")
            viewModel.detachRemoteVideoSink(renderer)
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
            // By name, for the same reason as RemoteVideo above — and
            // this one leaves and re-enters composition on every camera
            // toggle, so an overlap is that much likelier.
            viewModel.detachLocalVideoSink(renderer)
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
    is CallState.Ended -> {
        // A hung-up call keeps its length on the linger line — the same
        // "Video call · 3:12" the chat's call record is about to show,
        // where the iPhone says only "Call ended". Every other ending
        // words itself by reason and DIRECTION, exactly as the iPhone
        // does (CallNotifications.endedTextRes).
        val hungUpAfter = state.durationSecs?.takeIf { state.reason == CallEnding.HANGUP }
        if (hungUpAfter != null) {
            stringResource(
                if (state.video) R.string.s_video_call_with_duration else R.string.s_voice_call_with_duration,
                CallRecordWording.duration(hungUpAfter),
            )
        } else {
            stringResource(CallNotifications.endedTextRes(state.reason, state.outgoing, state.video))
        }
    }
}

@Composable
private fun IncomingControls(onDecline: () -> Unit, onAccept: () -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceEvenly,
        verticalAlignment = Alignment.Top,
    ) {
        RoundButton(
            icon = Icons.Filled.CallEnd,
            label = stringResource(R.string.s_decline),
            container = MaterialTheme.colorScheme.error,
            content = MaterialTheme.colorScheme.onError,
            modifier = Modifier.weight(1f),
            onClick = onDecline,
        )
        RoundButton(
            icon = Icons.Filled.Call,
            label = stringResource(R.string.s_answer),
            container = Color(0xFF2E7D32),
            content = Color.White,
            modifier = Modifier.weight(1f),
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
    /** The row sits on the remote picture — captions white, a scrim behind. */
    overVideo: Boolean,
    onToggleMute: () -> Unit,
    onToggleSpeaker: () -> Unit,
    onToggleCamera: () -> Unit,
    onFlipCamera: () -> Unit,
    onHangUp: () -> Unit,
) {
    // Up to five discs on a video call: 56 dp, the iPhone's, so all five
    // fit a 360 dp phone inside the scrim's padding (5 × 56 = 280 ≤ 288)
    // — 60 did not, and the last disc took the squeeze. A voice call's
    // three have room for 72.
    val buttonSize = if (isVideoCall) 56 else 72
    // Over the far side's picture the row sits on whatever their camera
    // shows — a bright wall, a window — so white captions and a
    // translucent scrim under the whole row keep discs and captions
    // readable over any picture. On the ground (black on a video call,
    // see CallScreen) neither is needed, and a scrim there would look
    // like a stray card.
    val captionColor = if (overVideo) Color.White else Color.Unspecified
    val scrim = if (overVideo) {
        Modifier
            .background(Color.Black.copy(alpha = 0.35f), RoundedCornerShape(28.dp))
            .padding(12.dp)
    } else {
        Modifier
    }
    // Every disc takes an EQUAL share of the row (weight) and sits at the
    // top of it: left to SpaceEvenly alone, the LAST child got whatever
    // width was left over on a 360 dp phone, in German, at a large font
    // scale — its disc squashed and its caption wrapped letter per line —
    // and a caption that wraps to two lines must not pull its disc out of
    // line with the others.
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .then(scrim),
        horizontalArrangement = Arrangement.SpaceEvenly,
        verticalAlignment = Alignment.Top,
    ) {
        RoundButton(
            icon = if (isMuted) Icons.Filled.MicOff else Icons.Filled.Mic,
            label = stringResource(if (isMuted) R.string.s_unmute else R.string.s_mute),
            container = if (isMuted) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,
            content = if (isMuted) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.weight(1f),
            labelColor = captionColor,
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
                modifier = Modifier.weight(1f),
                labelColor = captionColor,
                size = buttonSize,
                onClick = onToggleCamera,
            )
            if (isCameraOn) {
                RoundButton(
                    icon = Icons.Filled.FlipCameraAndroid,
                    label = stringResource(R.string.s_flip_camera),
                    container = MaterialTheme.colorScheme.surfaceVariant,
                    content = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.weight(1f),
                    labelColor = captionColor,
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
            modifier = Modifier.weight(1f),
            labelColor = captionColor,
            size = buttonSize,
            onClick = onHangUp,
        )
        RoundButton(
            icon = Icons.Filled.VolumeUp,
            label = stringResource(R.string.s_speaker),
            container = if (isSpeaker) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,
            content = if (isSpeaker) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.weight(1f),
            labelColor = captionColor,
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
    /** The row's share for this disc — `weight(1f)` from both rows, see ActiveControls. */
    modifier: Modifier = Modifier,
    /** The caption under the disc — Unspecified (onSurface) on the ground, white over video. */
    labelColor: Color = Color.Unspecified,
    size: Int = 72,
    onClick: () -> Unit,
) {
    Column(modifier = modifier, horizontalAlignment = Alignment.CenterHorizontally) {
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
        // The icon already carries the label as its contentDescription, so
        // a caption TalkBack also read made every disc say its name twice
        // — decorative to accessibility, then. Centred and at most two
        // lines: a long caption ("Kamera ausschalten") wraps rather than
        // widening its share, and an absurd one ends in an ellipsis.
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            color = labelColor,
            textAlign = TextAlign.Center,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.clearAndSetSemantics {},
        )
    }
}
