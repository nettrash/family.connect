/*
 * RoundVideoRecorderLayer.kt
 * Family Connect (Android)
 *
 * The video message recorder on screen (#79, docs/audio-video-messages-2026-10-04.md,
 * S3.3, S3.4, S6, S8.4, S8.5).
 *
 * A LAYER AT THE TOP OF THE APP'S CONTENT, above the list and detail panes —
 * never a Dialog, which would shift the layout and could be dismissed by
 * accident (S3.3). [VideoRecorderHost] wraps everything MainActivity draws:
 * while the recorder is open the app beneath it is out of TalkBack's reach
 * and keyboard focus cannot enter it, and the scrim takes every touch.
 *
 * REVISED 2026-10-06 (decision 41): on the owner's iPhone the composer row
 * showed through the recorder and its controls landed on the paperclip, ✨,
 * the field and Send, and a thin scrim let the chat compete with the camera.
 * So, here as on every platform: the composer row is not drawn while the
 * recorder is open (ChatScreen's ComposerUnderRecorder) and takes no hits;
 * the scrim is black at 85 % (75 % over a wide window's conversation pane)
 * and, from Android 12, the app beneath is blurred too (RenderEffect, which
 * costs nothing while the chat is still); the controls sit on their OWN
 * solid dark bar at the bottom of the pane, which runs under the navigation
 * bar and pads its contents clear of it; the status line sits in its own
 * capsule at the top; and the circle is fitted into what is left between
 * the two, so it never overlaps either at any size — a compact phone, a
 * phone on its side, a tablet, large text.
 *
 * A pane shorter than 480 dp — a phone on its side — stands the controls in
 * a column on their own bar at the trailing edge. The layout chosen when
 * RECORDING starts is kept until Stop (S3.3).
 *
 * The recorder itself lives outside the activity (AppVideoMessageRecorder);
 * a rebuilt activity draws this again, hands CameraX its new PreviewView and
 * re-applies the orientation hold, and the take carries on (S4, S8.4).
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.BlurredEdgeTreatment
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.semantics.hideFromAccessibility
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.graphics.StrokeCap
import android.Manifest
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.pm.PackageManager
import android.graphics.SurfaceTexture
import android.media.AudioAttributes
import android.media.MediaPlayer
import android.net.Uri
import android.view.OrientationEventListener
import android.view.Surface
import android.view.TextureView
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.view.PreviewView
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Cameraswitch
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.MicOff
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Replay
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material.icons.filled.VideocamOff
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.delay
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.calls.CallState
import me.nettrash.familyconnect.ui.components.KeepScreenAwake
import me.nettrash.familyconnect.ui.components.findActivity
import me.nettrash.familyconnect.ui.components.windowWidthDp
import java.io.File
import kotlin.math.roundToInt

/** The ring's red, and the warning's orange (S3.4) — words beside it too, never colour alone. */
private val RecordingRed = Color(0xFFE53935)
private val ScrimColor = Color.Black

/** The controls' own bar: solid, so nothing beneath can show through it (decision 41). */
internal val RecorderBarColor = Color(0xFF111216)

/** How dark the scrim is: over a compact window and a wide one's side panes, and over a wide one's conversation. */
internal const val SCRIM_ALPHA = 0.85f
internal const val SCRIM_PANE_ALPHA = 0.75f

/** How much the app beneath is blurred, where the platform does it cheaply (Android 12+). */
private val BACKDROP_BLUR = 16.dp

/**
 * Everything MainActivity draws, with the recorder over it while it is open.
 * While a call is live the recorder stays open underneath the call screen —
 * a clip in REVIEW is still there when the call ends (S4).
 */
@Composable
fun VideoRecorderHost(recorder: AppVideoMessageRecorder?, content: @Composable () -> Unit) {
    if (recorder == null) {
        content()
        return
    }
    val state by recorder.state.collectAsStateWithLifecycle()
    val call by recorder.callState.collectAsStateWithLifecycle()
    val showing = state.isOpen && call !is CallState.Live
    Box(modifier = Modifier.fillMaxSize()) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                // The app beneath is out of reach while the recorder is up:
                // TalkBack's traversal stays inside it (S3.4) and keyboard
                // focus cannot enter the panes.
                .then(
                    if (showing) {
                        Modifier
                            .clearAndSetSemantics { }
                            .focusProperties { onEnter = { cancelFocusChange() } }
                            // So the chat does not compete with the camera
                            // (decision 41): a RenderEffect from Android 12,
                            // nothing below it — where the scrim alone is
                            // dark enough.
                            .blur(BACKDROP_BLUR, BlurredEdgeTreatment.Rectangle)
                    } else {
                        Modifier
                    },
                )
                .focusGroup(),
        ) {
            content()
        }
        if (showing) RecorderOnScreen(recorder, state)
    }
}

/**
 * The composer row while the video recorder is open (decision 41): NOT
 * DRAWN, not hittable, not focusable and out of TalkBack's reach — kept
 * composed and laid out, so the conversation above it does not jump and the
 * draft under it is kept. Closed, it is simply [content].
 */
@Composable
internal fun ComposerUnderRecorder(recorderOpen: Boolean, content: @Composable () -> Unit) {
    Box(
        modifier = if (recorderOpen) {
            Modifier
                .testTag("composer-under-recorder")
                .graphicsLayer { alpha = 0f }
                .clearAndSetSemantics { }
                .focusProperties { onEnter = { cancelFocusChange() } }
                .focusGroup()
                // Every touch stops here, before anything inside can see it.
                .pointerInput(Unit) {
                    awaitEachGesture {
                        do {
                            val event = awaitPointerEvent(PointerEventPass.Initial)
                            event.changes.forEach { it.consume() }
                        } while (event.changes.any { it.pressed })
                    }
                }
        } else {
            Modifier
        },
    ) {
        content()
    }
}

/** The recorder's own effects — permission, orientation, haptics, the camera surface — and its layer. */
@Composable
private fun RecorderOnScreen(recorder: AppVideoMessageRecorder, state: VideoMessageRecorder.State) {
    val context = LocalContext.current
    val activity = remember(context) { context.findActivity() }
    val configuration = LocalConfiguration.current
    val phone = RoundRecorderRules.holdsOrientation(configuration.smallestScreenWidthDp)

    // The keyboard goes down: the conversation cannot change underneath (S3.3).
    val focusManager = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    LaunchedEffect(Unit) {
        focusManager.clearFocus(force = true)
        keyboard?.hide()
    }

    // S3.2: ONE system question for what is missing. Android says a refusal
    // is for good only after asking; that is remembered for next time.
    val permissions = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) { grants ->
        fun granted(permission: String) = grants[permission]
            ?: (ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED)
        val camera = granted(Manifest.permission.CAMERA)
        val microphone = granted(Manifest.permission.RECORD_AUDIO)
        val host = activity
        if (host != null) {
            if (!camera) {
                recorder.cameraRefusedForGood =
                    !androidx.core.app.ActivityCompat.shouldShowRequestPermissionRationale(host, Manifest.permission.CAMERA)
            }
            if (!microphone) {
                recorder.microphoneRefusedForGood = !androidx.core.app.ActivityCompat
                    .shouldShowRequestPermissionRationale(host, Manifest.permission.RECORD_AUDIO)
            }
        }
        recorder.permissionsAnswered(camera, microphone)
    }
    val asking = state.phase == VideoMessageRecorder.Phase.Asking
    LaunchedEffect(asking) {
        if (!asking) return@LaunchedEffect
        val missing = listOf(Manifest.permission.CAMERA, Manifest.permission.RECORD_AUDIO).filter {
            ContextCompat.checkSelfPermission(context, it) != PackageManager.PERMISSION_GRANTED
        }
        if (missing.isEmpty()) {
            recorder.permissionsAnswered(camera = true, microphone = true)
        } else {
            permissions.launch(missing.toTypedArray())
        }
    }

    // The capture angle at Record (S3.5): the device's orientation, read
    // from an OrientationEventListener while the recorder is up.
    val orientation = remember { mutableIntStateOf(OrientationEventListener.ORIENTATION_UNKNOWN) }
    DisposableEffect(context) {
        val listener = object : OrientationEventListener(context) {
            override fun onOrientationChanged(degrees: Int) {
                orientation.intValue = degrees
            }
        }
        if (listener.canDetectOrientation()) listener.enable()
        onDispose { listener.disable() }
    }

    // On a phone, from Record until Stop, the screen holds its CURRENT
    // orientation — never forced to portrait (S3.5, S8.4). Re-applied by a
    // rebuilt activity, since the request belongs to the activity.
    DisposableEffect(activity, state.holdOrientation) {
        val host = activity
        if (host == null || !state.holdOrientation) return@DisposableEffect onDispose { }
        val before = host.requestedOrientation
        host.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_LOCKED
        onDispose {
            if (!host.isChangingConfigurations) host.requestedOrientation = before
        }
    }

    // A medium haptic at Record, on phones (S3.4).
    val haptics = LocalHapticFeedback.current
    val recording = state.phase is VideoMessageRecorder.Phase.Recording
    LaunchedEffect(recording) {
        if (recording && voiceHapticsOn(configuration.smallestScreenWidthDp)) {
            haptics.performHapticFeedback(HapticFeedbackType.LongPress)
        }
    }

    // Auto-lock must not end a take or turn the camera off under somebody framing it (S1.7).
    KeepScreenAwake(active = true)
    // Back: PREVIEW closes, RECORDING stops, REVIEW asks (S3.4, S8.4).
    BackHandler { recorder.back() }

    val displayRotation = @Suppress("DEPRECATION") (activity?.windowManager?.defaultDisplay?.rotation ?: Surface.ROTATION_0)
    RecorderLayer(
        recorder = recorder,
        state = state,
        pane = recorder.paneBounds,
        compactWindow = windowWidthDp() < 600,
        onRecord = {
            recorder.record(
                rotation = RoundRecorderRules.surfaceRotation(orientation.intValue, displayRotation),
                holdOrientation = phone,
            )
        },
        onOpenSettings = {
            runCatching {
                context.startActivity(
                    Intent(
                        android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                        Uri.fromParts("package", context.packageName, null),
                    ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }
        },
        preview = { modifier ->
            // COMPATIBLE (a TextureView), so it clips to a circle and its
            // frames can be read for the black check; FILL_CENTER, so the
            // square fills the circle. PreviewView mirrors the front camera.
            AndroidView(
                factory = { viewContext ->
                    PreviewView(viewContext).apply {
                        implementationMode = PreviewView.ImplementationMode.COMPATIBLE
                        scaleType = PreviewView.ScaleType.FILL_CENTER
                    }
                },
                modifier = modifier,
                update = { view -> recorder.cameraX.attachPreview(view) },
                onRelease = { view -> if (recorder.cameraX.isAttached(view)) recorder.cameraX.attachPreview(null) },
            )
        },
    )
}

/**
 * The layer itself, drawn from [state] alone: what each of S3.4's states
 * shows, and the controls that move between them. [preview] is the live
 * camera — a PreviewView in the app, anything in a test.
 */
@Composable
internal fun RecorderLayer(
    recorder: VideoMessageRecorder,
    state: VideoMessageRecorder.State,
    /** The conversation pane, in root pixels; null draws over the whole window. */
    pane: Rect?,
    compactWindow: Boolean,
    onRecord: () -> Unit,
    onOpenSettings: () -> Unit,
    preview: @Composable (Modifier) -> Unit,
) {
    val density = LocalDensity.current
    val slotFocus = remember { FocusRequester() }
    val phase = state.phase
    val review = phase as? VideoMessageRecorder.Phase.Review
    val coordinator = LocalPlaybackCoordinator.current
    val player = remember(review?.clip?.prepared?.file) {
        review?.let { ReviewPlayer(it.clip.prepared.file, coordinator) }
    }
    DisposableEffect(player) { onDispose { player?.release() } }

    val paneTitle = stringResource(R.string.s_video_message)
    Box(
        modifier = Modifier
            .fillMaxSize()
            .testTag("round-video-recorder")
            .semantics { this.paneTitle = paneTitle }
            .onPreviewKeyEvent { event ->
                when {
                    // Esc: PREVIEW closes, RECORDING stops, REVIEW asks (S3.4).
                    event.key == Key.Escape -> {
                        if (event.type == KeyEventType.KeyUp) recorder.back()
                        true
                    }
                    // Return/Enter is the SLOT wherever focus is — Record,
                    // Stop, Send (S3.4, S6) — caught before the focused
                    // control, so it never closes, deletes or retakes. A
                    // dimmed slot does nothing. Where there is no slot to
                    // speak of (the permission screens) the focused control
                    // keeps it.
                    RecorderKeys.isReturn(event.key) && RecorderKeys.slotOwnsReturn(phase) -> {
                        if (event.type == KeyEventType.KeyDown && event.nativeKeyEvent.repeatCount == 0) {
                            slotAction(state, recorder, onRecord)?.let { action ->
                                recorder.used()
                                action()
                            }
                        }
                        true
                    }
                    // In REVIEW, Space plays and pauses WHEREVER focus is —
                    // caught before the focused control: Space never sends,
                    // deletes or retakes (S3.4).
                    event.key == Key.Spacebar && player != null -> {
                        if (event.type == KeyEventType.KeyDown && event.nativeKeyEvent.repeatCount == 0) {
                            player.toggle()
                        }
                        true
                    }
                    else -> {
                        if (event.type == KeyEventType.KeyDown) recorder.used()
                        false
                    }
                }
            },
    ) {
        Scrim(pane = pane, compactWindow = compactWindow, onTouch = recorder::used)
        val paneOffset = pane?.let { IntOffset(it.left.roundToInt(), it.top.roundToInt()) } ?: IntOffset.Zero
        val paneModifier = if (pane != null) {
            with(density) {
                Modifier
                    .offset { paneOffset }
                    .size(pane.width.toDp(), pane.height.toDp())
            }
        } else {
            Modifier.fillMaxSize()
        }
        BoxWithConstraints(modifier = paneModifier) {
            // Measured as S3.3 measures it — the pane inside the safe area and
            // an 8-unit margin — though the bars themselves run to its edges.
            val insets = WindowInsets.safeDrawing.asPaddingValues()
            val direction = LocalLayoutDirection.current
            val inner = maxWidth - insets.calculateLeftPadding(direction) - insets.calculateRightPadding(direction) - 16.dp
            val innerHeight = maxHeight - insets.calculateTopPadding() - insets.calculateBottomPadding() - 16.dp
            val bannerHeight = if (state.session?.reply != null) BANNER_HEIGHT_DP else 0
            val measured = RoundRecorderRules.layout(
                paneWidth = inner.value.toInt(),
                paneHeight = innerHeight.value.toInt(),
                bannerHeight = bannerHeight,
            )
            recorder.currentLayout = measured
            // Kept from Record until Stop: the controls never move under the thumb (S3.5).
            val layout = state.lockedLayout ?: measured
            val status: @Composable () -> Unit = {
                StatusLine(state = state, recorder = recorder, onOpenSettings = onOpenSettings)
            }
            // The circle in whatever room the status and the bar leave it:
            // S3.3's diameter at most, never more than fits — so it can never
            // reach the status above it or the bar below it.
            val circle: @Composable (Modifier) -> Unit = { modifier ->
                BoxWithConstraints(modifier = modifier.testTag("round-video-circle-room"), contentAlignment = Alignment.Center) {
                    val room = (minOf(maxWidth, maxHeight) - RING_GAP * 2).coerceAtLeast(0.dp)
                    RecorderCircle(
                        recorder = recorder,
                        state = state,
                        diameter = minOf(layout.diameter.dp, room),
                        player = player,
                        preview = preview,
                    )
                }
            }
            val banner: @Composable () -> Unit = {
                val session = state.session
                val reply = session?.reply
                if (session != null && reply != null) {
                    RecorderReplyBanner(
                        author = session.replyAuthor,
                        excerpt = session.replyExcerpt,
                        onDrop = recorder::dropReply,
                    )
                }
            }
            val controls: @Composable (Boolean) -> Unit = { column ->
                Controls(
                    state = state,
                    recorder = recorder,
                    column = column,
                    slotFocus = slotFocus,
                    onRecord = onRecord,
                )
            }
            if (layout.column) {
                Row(modifier = Modifier.fillMaxSize()) {
                    Column(
                        modifier = Modifier
                            .weight(1f)
                            .fillMaxHeight()
                            .windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.Vertical + WindowInsetsSides.Start))
                            .padding(8.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        status()
                        Spacer(Modifier.height(8.dp))
                        circle(Modifier.weight(1f).fillMaxWidth())
                    }
                    // The bar at the trailing edge, running under the system
                    // bars there; scrollable, so large text never pushes a
                    // control out of reach or onto another.
                    Column(
                        modifier = Modifier
                            .fillMaxHeight()
                            .background(RecorderBarColor)
                            .testTag("round-video-controls-bar")
                            .windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.Vertical + WindowInsetsSides.End))
                            .width(CONTROL_COLUMN_WIDTH)
                            .verticalScroll(rememberScrollState())
                            .padding(8.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterVertically),
                    ) {
                        banner()
                        controls(true)
                    }
                }
            } else {
                Column(modifier = Modifier.fillMaxSize()) {
                    Column(
                        modifier = Modifier
                            .weight(1f)
                            .fillMaxWidth()
                            .windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.Top + WindowInsetsSides.Horizontal))
                            .padding(8.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        status()
                        Spacer(Modifier.height(8.dp))
                        circle(Modifier.weight(1f).fillMaxWidth())
                    }
                    // The bar where the composer row was, on its own solid
                    // ground, running under the navigation bar.
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .background(RecorderBarColor)
                            .testTag("round-video-controls-bar")
                            .windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.Bottom + WindowInsetsSides.Horizontal))
                            .padding(start = 8.dp, end = 8.dp, top = 12.dp, bottom = 8.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        banner()
                        controls(false)
                    }
                }
            }
        }
        RecorderAnnouncer(state.announcement)
        DeleteQuestion(state = state, recorder = recorder)
    }
}

/** The scrim: takes every touch, so nothing under it can change (S3.3). */
@Composable
private fun Scrim(pane: Rect?, compactWindow: Boolean, onTouch: () -> Unit) {
    Canvas(
        modifier = Modifier
            .fillMaxSize()
            .pointerInput(Unit) {
                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    down.consume()
                    onTouch()
                    do {
                        val event = awaitPointerEvent()
                        event.changes.forEach { it.consume() }
                    } while (event.changes.any { it.pressed })
                }
            },
    ) {
        if (pane == null || compactWindow) {
            drawRect(ScrimColor.copy(alpha = SCRIM_ALPHA))
            return@Canvas
        }
        // 85 % over the sidebar and rail, 75 % over the conversation pane:
        // dark enough that the chat does not compete (decision 41).
        val outer = ScrimColor.copy(alpha = SCRIM_ALPHA)
        drawRect(outer, topLeft = Offset.Zero, size = Size(size.width, pane.top))
        drawRect(outer, topLeft = Offset(0f, pane.bottom), size = Size(size.width, size.height - pane.bottom))
        drawRect(outer, topLeft = Offset(0f, pane.top), size = Size(pane.left, pane.height))
        drawRect(outer, topLeft = Offset(pane.right, pane.top), size = Size(size.width - pane.right, pane.height))
        drawRect(ScrimColor.copy(alpha = SCRIM_PANE_ALPHA), topLeft = pane.topLeft, size = pane.size)
    }
}

/**
 * The status line in its own capsule above the circle (S3.4, decision 41),
 * and whatever the state has to say under it.
 */
@Composable
private fun StatusLine(
    state: VideoMessageRecorder.State,
    recorder: VideoMessageRecorder,
    onOpenSettings: () -> Unit,
) {
    val reducedMotion = animationsRemoved()
    var ticker by remember { mutableLongStateOf(0L) }
    val phase = state.phase
    LaunchedEffect(phase) {
        while (phase is VideoMessageRecorder.Phase.Recording) {
            ticker = recorder.recordedMs()
            delay(if (reducedMotion) 1_000L else 200L)
        }
    }
    val lines = mutableListOf<String>()
    var recordingClock: String? = null
    var warned = false
    when (phase) {
        VideoMessageRecorder.Phase.Closed -> Unit
        VideoMessageRecorder.Phase.Asking -> lines += stringResource(R.string.s_video_messages_need_camera_and_microphone)
        VideoMessageRecorder.Phase.CameraRefused -> lines += stringResource(R.string.e_camera_permission_settings)
        VideoMessageRecorder.Phase.MicrophoneRefused -> lines += stringResource(R.string.e_microphone_permission)
        is VideoMessageRecorder.Phase.Preview -> {
            if (!phase.busy) {
                lines += stringResource(if (phase.firstFrame) R.string.s_not_recording else R.string.s_starting_camera)
            }
            if (state.firstTime) lines += stringResource(R.string.s_only_you_can_see_this)
            if (phase.looksBlack) lines += stringResource(R.string.s_cant_see_anything)
        }
        is VideoMessageRecorder.Phase.Recording -> {
            recordingClock = VoiceNoteRules.clock(ticker)
            warned = phase.warned
        }
        is VideoMessageRecorder.Phase.Finishing ->
            lines += stringResource(R.string.s_video_message_with_length, VoiceNoteRules.clock(phase.recordedMs))
        is VideoMessageRecorder.Phase.Review -> {
            lines += stringResource(R.string.s_video_message_with_length, VoiceNoteRules.clock(phase.clip.durationMs))
            when (phase.clip.notRound) {
                RoundRecorderRules.NotRound.COULDNT_MAKE_ROUND -> lines += stringResource(R.string.s_couldnt_make_it_round)
                RoundRecorderRules.NotRound.TOO_BIG -> lines += stringResource(R.string.s_too_big_for_video_message)
                null -> Unit
            }
        }
    }
    state.notice?.let { lines += stringResource(it.text) }
    // A capsule while it is one line; rounded corners once it says more.
    val oneLine = lines.size + (if (recordingClock != null) 1 else 0) <= 1 &&
        phase !is VideoMessageRecorder.Phase.CameraRefused &&
        phase !is VideoMessageRecorder.Phase.MicrophoneRefused &&
        !(phase is VideoMessageRecorder.Phase.Preview && phase.busy)
    Surface(
        color = RecorderBarColor,
        contentColor = Color.White,
        shape = if (oneLine) RoundedCornerShape(percent = 50) else RoundedCornerShape(16.dp),
        modifier = Modifier.widthIn(max = 480.dp).testTag("round-video-status"),
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            if (recordingClock != null) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    // Pulsing, as the composer's; steady without animations (S2.9, S6).
                    RecordingDot(steady = reducedMotion, size = 8.dp, color = RecordingRed)
                    Spacer(Modifier.width(6.dp))
                    // The ticking clock is never announced (S6).
                    Text(text = recordingClock, style = MaterialTheme.typography.titleSmall)
                    if (warned) {
                        Spacer(Modifier.width(10.dp))
                        Text(
                            text = stringResource(R.string.s_ten_seconds_left),
                            color = VoiceWarningOrange,
                            style = MaterialTheme.typography.titleSmall,
                        )
                    }
                }
            }
            lines.forEachIndexed { index, line ->
                Text(
                    text = line,
                    textAlign = TextAlign.Center,
                    style = if (index == 0 && recordingClock == null) {
                        MaterialTheme.typography.titleSmall
                    } else {
                        MaterialTheme.typography.bodySmall
                    },
                )
            }
            when (phase) {
                VideoMessageRecorder.Phase.CameraRefused -> Row {
                    TextButton(onClick = onOpenSettings) { Text(stringResource(R.string.s_open_settings), color = Color.White) }
                    VoiceInsteadText(state = state, recorder = recorder)
                }
                VideoMessageRecorder.Phase.MicrophoneRefused ->
                    TextButton(onClick = onOpenSettings) { Text(stringResource(R.string.s_open_settings), color = Color.White) }
                is VideoMessageRecorder.Phase.Preview -> if (phase.busy) VoiceInsteadText(state = state, recorder = recorder)
                else -> Unit
            }
        }
    }
}

/** "Record a voice message instead" as words, beside a refusal or a busy camera (S3.2, S3.6). */
@Composable
private fun VoiceInsteadText(state: VideoMessageRecorder.State, recorder: VideoMessageRecorder) {
    val blocked = state.session?.voiceBlocked == true
    val reason = stringResource(R.string.e_send_or_delete_the_unsent_first)
    TextButton(
        onClick = recorder::voiceInstead,
        modifier = Modifier.semantics { if (blocked) stateDescription = reason },
    ) {
        Text(
            stringResource(R.string.s_record_voice_message_instead),
            color = Color.White.copy(alpha = if (blocked) 0.5f else 1f),
        )
    }
}

/** The circle (S3.3, S3.4): live and mirrored, then the clip as it will be sent; the ring just outside it. */
@Composable
private fun RecorderCircle(
    recorder: VideoMessageRecorder,
    state: VideoMessageRecorder.State,
    diameter: Dp,
    player: ReviewPlayer?,
    preview: @Composable (Modifier) -> Unit,
) {
    val phase = state.phase
    val accent = MaterialTheme.colorScheme.primary
    val reducedMotion = animationsRemoved()
    var progress by remember { mutableStateOf(0f) }
    var warned by remember { mutableStateOf(false) }
    LaunchedEffect(phase, player?.playing) {
        when {
            // Red, filling clockwise from 12 o'clock over the length limit;
            // orange from the warning (S3.4). Steps once a second under
            // reduced motion (S6).
            phase is VideoMessageRecorder.Phase.Recording -> while (true) {
                progress = (recorder.recordedMs().toFloat() / recorder.capMs().coerceAtLeast(1L)).coerceIn(0f, 1f)
                warned = phase.warned
                delay(if (reducedMotion) 1_000L else 50L)
            }
            player != null && player.playing -> while (player.playing) {
                progress = player.progress()
                delay(if (reducedMotion) 1_000L else 50L)
            }
            else -> {
                progress = 0f
                warned = false
            }
        }
    }
    Box(
        modifier = Modifier.size(diameter + RING_GAP * 2).testTag("round-video-circle"),
        contentAlignment = Alignment.Center,
    ) {
        // The ring just OUTSIDE the circle, so it never covers a face (the
        // approved design): a thin white track all the way round, and over it
        // the arc filling clockwise from 12 o'clock — red over the minute while
        // recording (orange from the warning), the accent while the clip plays.
        Canvas(modifier = Modifier.fillMaxSize().testTag("round-video-recorder-ring")) {
            val stroke = 3.dp.toPx()
            val radius = diameter.toPx() / 2f + 2.dp.toPx() + stroke / 2f
            val topLeft = Offset(center.x - radius, center.y - radius)
            val arcSize = Size(radius * 2, radius * 2)
            val showsTrack = phase is VideoMessageRecorder.Phase.Preview ||
                phase is VideoMessageRecorder.Phase.Recording ||
                phase is VideoMessageRecorder.Phase.Finishing ||
                phase is VideoMessageRecorder.Phase.Review
            if (showsTrack) {
                drawCircle(color = Color.White.copy(alpha = 0.25f), radius = radius, style = Stroke(width = 1.dp.toPx()))
            }
            val arcColor = when {
                phase is VideoMessageRecorder.Phase.Recording -> if (warned) VoiceWarningOrange else RecordingRed
                player != null && (player.playing || progress > 0f) -> accent
                else -> null
            }
            if (arcColor != null && progress > 0f) {
                drawArc(
                    color = arcColor,
                    startAngle = -90f,
                    sweepAngle = 360f * progress,
                    useCenter = false,
                    topLeft = topLeft,
                    size = arcSize,
                    style = Stroke(width = stroke, cap = StrokeCap.Round),
                )
            }
        }
        Box(
            modifier = Modifier
                .size(diameter)
                .testTag("round-video-circle-face")
                .clip(CircleShape)
                .background(Color(0xFF2B2B2B)),
            contentAlignment = Alignment.Center,
        ) {
            when (phase) {
                is VideoMessageRecorder.Phase.Preview ->
                    if (phase.busy) {
                        NeutralGlyph(Icons.Filled.VideocamOff)
                    } else {
                        preview(Modifier.fillMaxSize())
                    }
                is VideoMessageRecorder.Phase.Recording -> preview(Modifier.fillMaxSize())
                is VideoMessageRecorder.Phase.Finishing -> CircularProgressIndicator(color = Color.White)
                is VideoMessageRecorder.Phase.Review -> if (player != null) ReviewCircle(phase.clip, player)
                VideoMessageRecorder.Phase.CameraRefused -> NeutralGlyph(Icons.Filled.VideocamOff)
                VideoMessageRecorder.Phase.MicrophoneRefused -> NeutralGlyph(Icons.Filled.MicOff)
                VideoMessageRecorder.Phase.Asking, VideoMessageRecorder.Phase.Closed -> NeutralGlyph(Icons.Filled.Videocam)
            }
        }
    }
}

@Composable
private fun NeutralGlyph(icon: androidx.compose.ui.graphics.vector.ImageVector) {
    Icon(imageVector = icon, contentDescription = null, tint = Color.White.copy(alpha = 0.8f), modifier = Modifier.size(56.dp))
}

/** REVIEW's circle: the clip as it will be sent, not mirrored; a tap plays it with sound, another pauses. */
@Composable
private fun ReviewCircle(clip: RoundClip, player: ReviewPlayer) {
    val poster = remember(clip.prepared.previewJpeg) {
        clip.prepared.previewJpeg?.let { bytes ->
            runCatching { android.graphics.BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap() }.getOrNull()
        }
    }
    val playLabel = stringResource(if (player.playing) R.string.s_pause else R.string.s_play)
    Box(
        modifier = Modifier
            .fillMaxSize()
            .clickable(onClickLabel = playLabel, role = Role.Button) { player.toggle() }
            .semantics { contentDescription = playLabel },
        contentAlignment = Alignment.Center,
    ) {
        if (player.started) {
            AndroidView(
                factory = { context ->
                    TextureView(context).apply {
                        surfaceTextureListener = object : TextureView.SurfaceTextureListener {
                            override fun onSurfaceTextureAvailable(texture: SurfaceTexture, width: Int, height: Int) {
                                player.attach(Surface(texture))
                            }

                            override fun onSurfaceTextureSizeChanged(texture: SurfaceTexture, width: Int, height: Int) = Unit
                            override fun onSurfaceTextureDestroyed(texture: SurfaceTexture): Boolean {
                                player.attach(null)
                                return true
                            }

                            override fun onSurfaceTextureUpdated(texture: SurfaceTexture) = Unit
                        }
                    }
                },
                modifier = Modifier.fillMaxSize(),
            )
        } else if (poster != null) {
            Image(bitmap = poster, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
        }
        if (!player.playing) {
            Box(
                modifier = Modifier.size(44.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.45f)),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Filled.PlayArrow, contentDescription = null, tint = Color.White)
            }
        }
    }
}

/**
 * The control row (or column) where the composer row is (S3.3, S3.4), as
 * the approved design draws it: the leading control (Close, or Delete), the
 * middle ones (Switch camera — and the way to a voice message instead — in
 * PREVIEW; Retake in REVIEW) as small round buttons with a caption under
 * each, and the ONE big slot in Send's place: Record → Stop → Send.
 */
@Composable
private fun Controls(
    state: VideoMessageRecorder.State,
    recorder: VideoMessageRecorder,
    column: Boolean,
    slotFocus: FocusRequester,
    onRecord: () -> Unit,
) {
    val phase = state.phase
    val leading: @Composable () -> Unit = {
        when (phase) {
            is VideoMessageRecorder.Phase.Recording ->
                RecorderButton(
                    Icons.Outlined.Delete,
                    stringResource(R.string.s_delete_recording),
                    caption = stringResource(R.string.s_delete),
                    onClick = recorder::delete,
                )
            is VideoMessageRecorder.Phase.Review, is VideoMessageRecorder.Phase.Finishing -> {
                val ready = phase is VideoMessageRecorder.Phase.Review
                RecorderButton(
                    Icons.Outlined.Delete,
                    stringResource(R.string.s_delete),
                    caption = stringResource(R.string.s_delete),
                    enabled = ready,
                    onClick = recorder::delete,
                )
            }
            else -> RecorderButton(
                Icons.Filled.Close,
                stringResource(R.string.s_close),
                caption = stringResource(R.string.s_close),
                onClick = recorder::close,
            )
        }
    }
    val middle: @Composable () -> Unit = {
        when (phase) {
            is VideoMessageRecorder.Phase.Preview -> {
                // "Switch camera" on phones and tablets, in PREVIEW only (S3.5).
                if (recorder.canSwitchCamera) {
                    RecorderButton(
                        Icons.Filled.Cameraswitch,
                        stringResource(R.string.s_switch_camera),
                        caption = stringResource(R.string.s_switch),
                        onClick = recorder::switchCamera,
                    )
                }
                val blocked = state.session?.voiceBlocked == true
                RecorderButton(
                    icon = Icons.Filled.Mic,
                    label = stringResource(R.string.s_record_voice_message_instead),
                    caption = stringResource(R.string.s_record_voice_message),
                    dimmedReason = if (blocked) stringResource(R.string.e_send_or_delete_the_unsent_first) else null,
                    onClick = recorder::voiceInstead,
                )
            }
            is VideoMessageRecorder.Phase.Review, is VideoMessageRecorder.Phase.Finishing ->
                RecorderButton(
                    Icons.Filled.Replay,
                    stringResource(R.string.s_retake),
                    caption = stringResource(R.string.s_retake),
                    enabled = phase is VideoMessageRecorder.Phase.Review,
                    onClick = recorder::retake,
                )
            else -> Unit
        }
    }
    val slot: @Composable () -> Unit = { Slot(state = state, recorder = recorder, focus = slotFocus, onRecord = onRecord) }
    if (column) {
        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row { leading() }
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) { middle() }
            slot()
        }
    } else {
        Row(
            // The cap BEFORE fillMaxWidth: after it, fillMaxWidth has already
            // fixed the minimum at the whole width and the cap does nothing.
            modifier = Modifier.widthIn(max = 420.dp).fillMaxWidth().padding(horizontal = 8.dp),
            verticalAlignment = Alignment.Top,
        ) {
            Row(verticalAlignment = Alignment.Top) { leading() }
            Row(
                modifier = Modifier.weight(1f),
                horizontalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterHorizontally),
                verticalAlignment = Alignment.Top,
            ) { middle() }
            slot()
        }
    }
}

/**
 * The slot (S3.4, the approved design): ONE big button, 64 units, in the
 * Send button's place — a red disc (Record), a red disc with a white rounded
 * square (Stop), then an accent disc with the Send arrow (Send) — with its
 * caption under it.
 */
@Composable
private fun Slot(
    state: VideoMessageRecorder.State,
    recorder: VideoMessageRecorder,
    focus: FocusRequester,
    onRecord: () -> Unit,
) {
    val phase = state.phase
    val label = stringResource(
        when (phase) {
            is VideoMessageRecorder.Phase.Recording -> R.string.s_stop_recording
            is VideoMessageRecorder.Phase.Review, is VideoMessageRecorder.Phase.Finishing ->
                R.string.s_send_video_message
            else -> R.string.s_record
        },
    )
    val caption = stringResource(
        when (phase) {
            is VideoMessageRecorder.Phase.Recording -> R.string.s_stop
            is VideoMessageRecorder.Phase.Review, is VideoMessageRecorder.Phase.Finishing -> R.string.s_send
            else -> R.string.s_record
        },
    )
    val action = slotAction(state, recorder, onRecord)
    val live = action != null

    // Focus starts on the slot and stays on it as it changes (S3.4). Asked
    // HERE, by the slot itself: it is composed inside BoxWithConstraints'
    // subcomposition, which happens at layout — after an effect of the layer
    // above has already run — so a request from up there found nothing
    // attached and was lost. The slot is focusable even dimmed (before the
    // first frame, while the clip is checked), so the request at open lands
    // and Esc and Return reach the recorder from the start; a slot moved
    // between the row and the column asks again.
    LaunchedEffect(focus, phase::class) { runCatching { focus.requestFocus() } }
    val sends = phase is VideoMessageRecorder.Phase.Review || phase is VideoMessageRecorder.Phase.Finishing
    val container = if (sends) MaterialTheme.colorScheme.primary else RecordingRed
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
      // The design's halo: a soft white ring, 4 units, just outside the disc.
      Box(
        modifier = Modifier
            .size(SLOT_SIZE + SLOT_HALO * 2)
            .drawBehind {
                drawCircle(
                    color = Color.White.copy(alpha = 0.18f),
                    radius = size.minDimension / 2f - SLOT_HALO.toPx() / 2f,
                    style = Stroke(width = SLOT_HALO.toPx()),
                )
            },
        contentAlignment = Alignment.Center,
      ) {
        Box(
            modifier = Modifier
                .size(SLOT_SIZE)
                .focusRequester(focus)
                .testTag("round-video-slot")
                // Focusable whatever the input mode: the recorder is opened by a
                // tap, and a clickable's default focusability is the system's
                // (Focusability.SystemDefined), which on a touch-mode window may
                // refuse it — the request at open would fail and a hardware
                // keyboard's Esc and Return would reach nothing. (Robolectric
                // will not enter touch mode, so no JVM test pins this line.)
                .focusProperties { canFocus = true }
                .clip(CircleShape)
                // ALWAYS enabled as a node, so it keeps the focus while dimmed
                // (S3.4: "Focus starts on the slot"; S1.3: dimmed is not
                // disabled); a dimmed slot's click does nothing, and its
                // semantics still say disabled below.
                .clickable(role = Role.Button, onClickLabel = label) {
                    val now = slotAction(state, recorder, onRecord) ?: return@clickable
                    recorder.used()
                    now()
                }
                .semantics {
                    contentDescription = label
                    if (!live) disabled()
                }
                .background(container.copy(alpha = if (live) 1f else 0.4f)),
            contentAlignment = Alignment.Center,
        ) {
            when (phase) {
                is VideoMessageRecorder.Phase.Recording ->
                    Box(
                        Modifier
                            .size(22.dp)
                            .clip(RoundedCornerShape(5.dp))
                            .background(Color.White)
                            .testTag("round-video-slot-stop"),
                    )
                is VideoMessageRecorder.Phase.Review, is VideoMessageRecorder.Phase.Finishing ->
                    Icon(
                        Icons.AutoMirrored.Filled.Send,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.onPrimary,
                        modifier = Modifier.size(26.dp),
                    )
                // Record: the red disc itself.
                else -> Unit
            }
        }
      }
        RecorderCaption(caption)
    }
}

/** The big slot, 64 units across, and its halo (the approved design). */
private val SLOT_SIZE = 64.dp
private val SLOT_HALO = 4.dp

/** What a recorder button is called, under it, small and quiet — its label to TalkBack is the button's own. */
@Composable
private fun RecorderCaption(text: String) {
    Text(
        text = text,
        style = MaterialTheme.typography.labelSmall,
        color = Color(0xFFC9CBD6),
        textAlign = TextAlign.Center,
        maxLines = 2,
        overflow = TextOverflow.Ellipsis,
        modifier = Modifier
            .widthIn(max = 84.dp)
            .padding(top = 4.dp)
            .semantics { hideFromAccessibility() },
    )
}

/**
 * What the slot does now (S3.4) — Record once the camera has delivered its
 * first frame (dimmed before, and while another app has the camera), Stop
 * while recording, Send in REVIEW (dimmed while the clip is checked) — or
 * null while it is dimmed. The one answer the slot's click and Return share.
 */
internal fun slotAction(
    state: VideoMessageRecorder.State,
    recorder: VideoMessageRecorder,
    onRecord: () -> Unit,
): (() -> Unit)? = when (val phase = state.phase) {
    is VideoMessageRecorder.Phase.Recording -> recorder::stop
    is VideoMessageRecorder.Phase.Review -> recorder::send
    is VideoMessageRecorder.Phase.Preview -> onRecord.takeIf { phase.firstFrame && !phase.busy }
    else -> null
}

/** The recorder's keyboard (S3.4, S6). */
internal object RecorderKeys {
    /** Return, Enter and the keypad's Enter. */
    fun isReturn(key: Key): Boolean = key == Key.Enter || key == Key.NumPadEnter

    /** The states with a slot — Record, Stop, Send — where Return is that slot. */
    fun slotOwnsReturn(phase: VideoMessageRecorder.Phase): Boolean = when (phase) {
        is VideoMessageRecorder.Phase.Preview,
        is VideoMessageRecorder.Phase.Recording,
        is VideoMessageRecorder.Phase.Finishing,
        is VideoMessageRecorder.Phase.Review,
        -> true
        else -> false
    }
}

/**
 * One of the recorder's other controls (the approved design): a 44-unit
 * round button, white on a faint white disc over the dark window, in a 48-unit
 * target, with its caption under it.
 */
@Composable
private fun RecorderButton(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    label: String,
    caption: String? = null,
    enabled: Boolean = true,
    /** Dimmed is not disabled: it stays focusable and says why (S1.3). */
    dimmedReason: String? = null,
    onClick: () -> Unit,
) {
    val bright = enabled && dimmedReason == null
    Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.widthIn(min = 64.dp)) {
        Box(
            modifier = Modifier
                .size(ComposerSlot.MIN_TARGET_ANDROID_DP.dp)
                .clip(CircleShape)
                .clickable(enabled = enabled, role = Role.Button, onClickLabel = label, onClick = onClick)
                .semantics {
                    contentDescription = label
                    if (dimmedReason != null) stateDescription = dimmedReason
                    if (!enabled) disabled()
                },
            contentAlignment = Alignment.Center,
        ) {
            Box(
                modifier = Modifier
                    .size(44.dp)
                    .clip(CircleShape)
                    .background(Color.White.copy(alpha = 0.08f)),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = icon,
                    contentDescription = null,
                    tint = Color(0xFFE8E9EF).copy(alpha = if (bright) 1f else 0.45f),
                    modifier = Modifier.size(22.dp),
                )
            }
        }
        if (caption != null) RecorderCaption(caption)
    }
}

/** The reply the video will carry, with its ✕ (S3.3). */
@Composable
private fun RecorderReplyBanner(author: String, excerpt: String, onDrop: () -> Unit) {
    Surface(
        color = Color.Black.copy(alpha = 0.6f),
        contentColor = Color.White,
        shape = RoundedCornerShape(12.dp),
        modifier = Modifier.fillMaxWidth().height(BANNER_HEIGHT_DP.dp).padding(bottom = 4.dp),
    ) {
        Row(modifier = Modifier.padding(start = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.width(3.dp).height(28.dp).background(MaterialTheme.colorScheme.primary, RoundedCornerShape(1.5.dp)))
            Spacer(Modifier.width(8.dp))
            Column(modifier = Modifier.weight(1f)) {
                Text(stringResource(R.string.s_replying_to, author), style = MaterialTheme.typography.labelMedium, maxLines = 1)
                Text(excerpt, style = MaterialTheme.typography.bodySmall, maxLines = 1)
            }
            RecorderButton(Icons.Filled.Close, stringResource(R.string.s_cancel_reply), onClick = onDrop)
        }
    }
}

/** "Delete video message?" [Delete] [Keep] — dismissing it is Keep (S3.4). */
@Composable
private fun DeleteQuestion(state: VideoMessageRecorder.State, recorder: VideoMessageRecorder) {
    if (state.ask == null) return
    AlertDialog(
        onDismissRequest = { recorder.answer(delete = false) },
        title = { Text(stringResource(R.string.s_delete_video_message)) },
        confirmButton = {
            TextButton(onClick = { recorder.answer(delete = true) }) {
                Text(stringResource(R.string.s_delete), color = MaterialTheme.colorScheme.error)
            }
        },
        dismissButton = {
            TextButton(onClick = { recorder.answer(delete = false) }) { Text(stringResource(R.string.s_keep)) }
        },
    )
}

/** The recorder's polite live region (S6): state changes only, never the clock. */
@Composable
private fun RecorderAnnouncer(announcement: VideoMessageRecorder.Announcement?) {
    val text = announcement?.let { stringResource(it.text) }
    VoiceAnnouncer(text?.let { ChatViewModel.VoiceAnnouncement(it, announcement.serial) })
}

/** The reply banner's height, in dp — what S3.3's diameter takes off. */
private const val BANNER_HEIGHT_DP = 56

/** The ring's room outside the circle, on each side. */
private val RING_GAP = 6.dp

/** The trailing control bar's width, for a phone on its side (S3.3). */
private val CONTROL_COLUMN_WIDTH = 184.dp

/**
 * REVIEW's playback of the local clip: one MediaPlayer, the app's
 * now-playing owner told when it plays, so nothing else plays over it.
 */
@Stable
internal class ReviewPlayer(private val file: File, private val coordinator: PlaybackCoordinator?) {
    var playing by mutableStateOf(false)
        private set

    /** A player exists: the circle draws its picture surface instead of the poster. */
    var started by mutableStateOf(false)
        private set

    private var player: MediaPlayer? = null
    private var surface: Surface? = null

    val pause: () -> Unit = {
        player?.runCatching { if (isPlaying) pause() }
        playing = false
    }

    fun toggle() {
        if (playing) {
            pause()
            coordinator?.stopped(pause)
            return
        }
        val ready = player ?: runCatching {
            MediaPlayer().apply {
                setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_MOVIE)
                        .build(),
                )
                setDataSource(file.absolutePath)
                surface?.let(::setSurface)
                prepare()
                setOnCompletionListener {
                    playing = false
                    seekTo(0)
                    coordinator?.stopped(pause)
                }
            }
        }.getOrNull() ?: return
        player = ready
        started = true
        if (runCatching { ready.start() }.isFailure) return
        playing = true
        coordinator?.started(pause)
    }

    fun attach(next: Surface?) {
        surface = next
        player?.runCatching { setSurface(next) }
    }

    fun progress(): Float {
        val active = player ?: return 0f
        val duration = runCatching { active.duration }.getOrDefault(0)
        if (duration <= 0) return 0f
        return (runCatching { active.currentPosition }.getOrDefault(0).toFloat() / duration).coerceIn(0f, 1f)
    }

    fun release() {
        coordinator?.stopped(pause)
        player?.runCatching { release() }
        player = null
        playing = false
        started = false
    }
}
