/*
 * RoundVideoBubble.kt
 * Family Connect (Android)
 *
 * A VIDEO MESSAGE in a thread: the circle (#79,
 * docs/audio-video-messages-2026-10-04.md, S5.2-S5.6, S6; docs/protocol.md,
 * "Video messages").
 *
 *  - NO BALLOON, like a sticker: the circle alone on the chat background
 *    (MessageBubble draws it bare). 200 dp across in a window under 600 dp,
 *    240 dp from 600 dp — larger than a sticker (160), smaller than a video
 *    tile — and the same size from the first frame to the last, so the row
 *    never changes height.
 *  - The square POSTER fills it; until it lands, a neutral disc. Only the
 *    poster is fetched to draw it — a tile never downloads a video to draw
 *    itself (protocol.md).
 *  - On a soft shadow (the approved design): the length in a dark
 *    translucent capsule at the bottom centre, with a white dot in it until
 *    THIS DEVICE has played somebody else's circle, and a 48 dp play disc in
 *    the middle that fades out while it plays.
 *  - A tap PLAYS IT IN PLACE through a TextureView — the one platform video
 *    surface a Compose clip can make round; a SurfaceView would punch a
 *    square hole — with exactly ONE 3 dp accent ring running round just
 *    outside the edge, a loading ring while the stream starts and a sentence
 *    when it fails
 *    (RoundVideoPlayback.kt has the rules). While it plays, an expand
 *    control at its top trailing edge opens the existing full-screen viewer,
 *    which the message menu also offers.
 *  - The sender's own circle, before the server has it, is drawn from the
 *    local poster with "Sending…" and a thin neutral ring (S5.6).
 *
 * Its "Show text" line (#62) is drawn under it by the bubble, outside its
 * gestures, as under a video tile.
 */

package me.nettrash.familyconnect.ui.chat

import android.graphics.SurfaceTexture
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.graphicsLayer
import android.view.Surface
import android.view.TextureView
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.clickable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.OpenInFull
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.delay
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.ui.components.KeepScreenAwake
import me.nettrash.familyconnect.ui.components.findActivity
import me.nettrash.familyconnect.ui.components.formatDuration
import me.nettrash.familyconnect.ui.components.rememberTileImage
import me.nettrash.familyconnect.ui.components.windowWidthDp

/** The numbers a circle is drawn at (S5.2). */
object RoundVideoDrawing {
    /** In a window under [WIDE_FROM] dp. */
    val COMPACT: Dp = 200.dp

    /** From [WIDE_FROM] dp: a tablet, a foldable open, a wide window. */
    val REGULAR: Dp = 240.dp

    /** Android's line between the two (S5.2): the app's own wide-window line. */
    const val WIDE_FROM = 600

    fun diameter(windowWidthDp: Int): Dp = if (windowWidthDp >= WIDE_FROM) REGULAR else COMPACT

    val PLAY_DISC: Dp = 48.dp
    val UNPLAYED_DOT: Dp = 7.dp
    val PROGRESS_RING: Dp = 3.dp

    /**
     * The room round the circle for its ONE accent ring, which runs just
     * OUTSIDE the edge (the approved design) so it never covers a face: a
     * 1.5-unit gap, then the 3-unit ring.
     */
    val RING_GAP: Dp = 1.5.dp
    val RING_ROOM: Dp = 6.dp

    /** The soft shadow the circle sits on, with no balloon behind it. */
    val SHADOW: Dp = 6.dp
    val EXPAND_TARGET: Dp = 44.dp
    val EXPAND_GLYPH: Dp = 28.dp
}

/** The wash under everything drawn on a picture — play disc, the failed line. */
private val SCRIM = Color.Black.copy(alpha = 0.45f)

/** The length capsule's dark translucent ground (the approved design). */
private val BADGE = Color.Black.copy(alpha = 0.55f)

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun RoundVideoBubble(
    attachment: AttachmentDto,
    /** The reader's own: no unplayed dot. */
    isMine: Boolean,
    /**
     * The server has the message. Before that the circle is the sender's own
     * being sent (S5.6): it draws, and plays nothing.
     */
    acked: Boolean,
    /** Still on its way up — the thin ring and "Sending…". */
    sending: Boolean,
    streamUrl: suspend (Long) -> Pair<String, Map<String, String>>?,
    onOpenFullScreen: () -> Unit,
    onLongPress: () -> Unit,
    onDoubleTap: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val diameter = RoundVideoDrawing.diameter(windowWidthDp())
    val context = LocalContext.current
    val coordinator = LocalPlaybackCoordinator.current ?: remember { PlaybackCoordinator() }
    val factory = LocalVideoPlayerFactory.current
        ?: remember(context) { MediaPlayerVideoFactory(context, streamUrl) }
    val gate = LocalRecordingGate.current
    val reducedMotion = animationsRemoved()

    // Only a message the server has can be streamed; the sender's own circle
    // in flight has a provisional id nobody can fetch.
    val playback = if (acked) {
        remember(attachment.id, coordinator) { coordinator.round(attachment.id, factory) }
    } else {
        null
    }
    if (playback != null) {
        val activity = remember(context) { context.findActivity() }
        DisposableEffect(playback) {
            playback.bind()
            onDispose { playback.unbind(rebuilding = activity?.isChangingConfigurations == true) }
        }
        LaunchedEffect(playback, playback.phase, reducedMotion) {
            while (playback.phase == RoundVideoPlayback.Phase.PLAYING) {
                playback.track()
                // Reduced motion: the ring steps once a second instead of
                // sweeping (S6).
                delay(if (reducedMotion) 1_000L else 50L)
            }
        }
    }
    val phase = playback?.phase ?: RoundVideoPlayback.Phase.IDLE
    val playingOrPaused = phase == RoundVideoPlayback.Phase.PLAYING || phase == RoundVideoPlayback.Phase.PAUSED
    // The screen stays awake while something of the app's plays (S1.7).
    KeepScreenAwake(active = phase == RoundVideoPlayback.Phase.PLAYING)

    val played by coordinator.played.ids.collectAsStateWithLifecycle()
    val showsDot = RoundVideoRules.showsUnplayedDot(isMine = isMine, acked = acked, played = attachment.id in played)
    val durationMs = attachment.durationMs ?: 0
    val poster = rememberTileImage(attachment)
    val ink = LocalContentColor.current
    val accent = MaterialTheme.colorScheme.primary

    // Resolved out here: a semantics block is not a composable context.
    val label = stringResource(R.string.s_video_message_a11y, formatDuration(durationMs))
    val notPlayed = stringResource(R.string.s_not_played)
    val playedWord = stringResource(R.string.s_played)
    val waitsForTheRecording = stringResource(R.string.s_play_after_recording)
    val playLabel = stringResource(
        if (phase == RoundVideoPlayback.Phase.PLAYING) R.string.s_pause else R.string.s_play,
    )
    val fullScreenLabel = stringResource(R.string.s_open_full_screen)
    val openFullScreen: () -> Unit = {
        // The viewer plays it alone: the circle stops where it is.
        if (playback?.phase == RoundVideoPlayback.Phase.PLAYING) playback.tap()
        onOpenFullScreen()
    }

    val discAlpha by animateFloatAsState(
        targetValue = if (phase == RoundVideoPlayback.Phase.PLAYING) 0f else 1f,
        animationSpec = if (reducedMotion) snap() else tween(ComposerSlot.SLOT_CROSSFADE_MS.toInt()),
        label = "roundPlayDisc",
    )
    // Room for the ring outside the edge, so nothing the bubble clips to cuts it.
    Box(
        modifier = modifier
            .size(diameter + RoundVideoDrawing.RING_ROOM * 2)
            .testTag("round-video-frame-${attachment.id}")
            .outsideRing(diameter = diameter, color = accent) {
                if (playingOrPaused) {
                    RoundVideoRules.progress(
                        positionMs = playback!!.positionMs,
                        durationMs = durationMs,
                        stepped = reducedMotion,
                    )
                } else {
                    null
                }
            },
        contentAlignment = Alignment.Center,
    ) {
    Box(modifier = Modifier.size(diameter).testTag("round-video-${attachment.id}")) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .shadow(RoundVideoDrawing.SHADOW, CircleShape, clip = false)
                .clip(CircleShape)
                // The neutral disc of the final size, until the poster lands
                // (S5.2) — never a placeholder of another shape.
                .background(ink.copy(alpha = 0.12f))
                .combinedClickable(
                    interactionSource = remember { MutableInteractionSource() },
                    indication = null,
                    onClickLabel = playLabel,
                    // Waits out the double-tap window, which stays the
                    // heart (S5.3, the sticker's precedent).
                    onClick = {
                        when {
                            playback == null -> Unit
                            // Dimmed, not disabled: it says why (S1.7).
                            gate.recording -> gate.explain()
                            else -> playback.tap()
                        }
                    },
                    onLongClick = onLongPress,
                    onDoubleClick = onDoubleTap,
                )
                .semantics {
                    contentDescription = label
                    role = Role.Button
                    when {
                        gate.recording && playback != null -> stateDescription = waitsForTheRecording
                        showsDot -> stateDescription = notPlayed
                        // Somebody else's, played here: "Played", as the voice
                        // bubble says and every other client's circle does.
                        !isMine && acked -> stateDescription = playedWord
                    }
                    if (playback != null) {
                        customActions = listOf(
                            CustomAccessibilityAction(fullScreenLabel) {
                                openFullScreen()
                                true
                            },
                        )
                    }
                },
            contentAlignment = Alignment.Center,
        ) {
            if (playback != null && playback.hasPlayer) {
                VideoSurface(playback)
            }
            // The poster stays over the picture surface until the video is
            // actually going, so a stream that is still starting shows the
            // poster and not a black disc.
            if (poster != null && !playingOrPaused) {
                Image(
                    bitmap = poster,
                    contentDescription = null,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.fillMaxSize(),
                )
            }
            when (phase) {
                RoundVideoPlayback.Phase.LOADING -> Box(
                    modifier = Modifier.size(RoundVideoDrawing.PLAY_DISC).clip(CircleShape).background(SCRIM),
                    contentAlignment = Alignment.Center,
                ) {
                    CircularProgressIndicator(
                        color = Color.White,
                        strokeWidth = 2.dp,
                        modifier = Modifier.size(24.dp).testTag("round-video-loading"),
                    )
                }
                RoundVideoPlayback.Phase.FAILED -> Box(
                    modifier = Modifier.fillMaxSize().background(SCRIM),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = stringResource(R.string.s_couldnt_load_video_tap),
                        style = MaterialTheme.typography.labelMedium,
                        color = Color.White,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.padding(horizontal = 28.dp),
                    )
                }
                RoundVideoPlayback.Phase.PLAYING,
                RoundVideoPlayback.Phase.IDLE,
                RoundVideoPlayback.Phase.PAUSED,
                -> if (!sending && discAlpha > 0f) {
                    Box(
                        modifier = Modifier
                            .size(RoundVideoDrawing.PLAY_DISC)
                            .graphicsLayer { alpha = discAlpha }
                            .clip(CircleShape)
                            .background(SCRIM)
                            .testTag("round-video-play-disc"),
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(Icons.Filled.PlayArrow, contentDescription = null, tint = Color.White)
                    }
                }
            }
            // The capsule at the bottom centre, inside the circle: the length,
            // and the white dot in it until it has been played.
            Row(
                modifier = Modifier
                    .align(Alignment.BottomCenter)
                    .padding(bottom = 14.dp)
                    .clip(RoundedCornerShape(10.dp))
                    .background(BADGE)
                    .padding(horizontal = 8.dp, vertical = 2.dp)
                    .testTag("round-video-badge"),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                val shownMs = if (playingOrPaused) {
                    // Whole seconds under reduced motion: the ring steps, so
                    // the clock does too.
                    if (reducedMotion) playback!!.positionMs / 1_000 * 1_000 else playback!!.positionMs
                } else {
                    durationMs
                }
                Text(
                    text = if (sending) stringResource(R.string.s_sending_ellipsis) else formatDuration(shownMs),
                    style = MaterialTheme.typography.labelSmall.copy(fontFeatureSettings = "tnum"),
                    color = Color.White,
                )
                if (showsDot) {
                    Box(
                        modifier = Modifier
                            .size(RoundVideoDrawing.UNPLAYED_DOT)
                            .clip(CircleShape)
                            .background(Color.White)
                            .testTag("round-video-unplayed"),
                    )
                }
            }
        }
        if (playingOrPaused) {
            // The ring itself is drawn by the box round this one, outside the edge.
            Box(Modifier.fillMaxSize().testTag("round-video-progress"))
            // The expand control, while it plays (S5.4): a 28 dp glyph in a
            // 44 dp target at the circle's top trailing edge.
            Box(
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .size(RoundVideoDrawing.EXPAND_TARGET)
                    .clickable(onClickLabel = fullScreenLabel) { openFullScreen() }
                    .testTag("round-video-expand"),
                contentAlignment = Alignment.Center,
            ) {
                Box(
                    modifier = Modifier.size(RoundVideoDrawing.EXPAND_GLYPH).clip(CircleShape).background(SCRIM),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        Icons.Filled.OpenInFull,
                        contentDescription = fullScreenLabel,
                        tint = Color.White,
                        modifier = Modifier.size(16.dp),
                    )
                }
            }
        }
        if (sending) {
            // A thin neutral ring for the upload (S5.6). Indeterminate: the
            // outbox reports no byte counts.
            CircularProgressIndicator(
                color = ink.copy(alpha = 0.45f),
                strokeWidth = 2.dp,
                modifier = Modifier.fillMaxSize().testTag("round-video-sending"),
            )
        }
    }
    }
}

/**
 * The circle's picture: a TextureView whose surface is handed to the
 * playback, and taken back when it goes — so a rebuilt activity's new
 * TextureView can pick up a player that never stopped.
 */
@Composable
private fun VideoSurface(playback: RoundVideoPlayback) {
    AndroidView(
        factory = { context ->
            TextureView(context).apply {
                surfaceTextureListener = object : TextureView.SurfaceTextureListener {
                    private var surface: Surface? = null

                    override fun onSurfaceTextureAvailable(texture: SurfaceTexture, width: Int, height: Int) {
                        val made = Surface(texture)
                        surface = made
                        playback.attachSurface(made)
                    }

                    override fun onSurfaceTextureSizeChanged(texture: SurfaceTexture, width: Int, height: Int) = Unit

                    override fun onSurfaceTextureDestroyed(texture: SurfaceTexture): Boolean {
                        playback.detachSurface(surface)
                        surface?.release()
                        surface = null
                        return true
                    }

                    override fun onSurfaceTextureUpdated(texture: SurfaceTexture) = Unit
                }
            }
        },
        modifier = Modifier.fillMaxSize(),
    )
}

/**
 * The circle's ONE progress ring (the approved design): [RoundVideoDrawing.PROGRESS_RING]
 * wide, [RoundVideoDrawing.RING_GAP] OUTSIDE the edge of a [diameter] circle
 * centred in this box, running clockwise from 12 o'clock to [fraction] — or
 * nothing while [fraction] is null (at rest).
 */
internal fun Modifier.outsideRing(diameter: Dp, color: Color, fraction: () -> Float?): Modifier = drawWithContent {
    drawContent()
    val shown = fraction() ?: return@drawWithContent
    val stroke = RoundVideoDrawing.PROGRESS_RING.toPx()
    val radius = diameter.toPx() / 2f + RoundVideoDrawing.RING_GAP.toPx() + stroke / 2f
    drawArc(
        color = color,
        startAngle = -90f,
        sweepAngle = 360f * shown,
        useCenter = false,
        topLeft = Offset(center.x - radius, center.y - radius),
        size = Size(radius * 2, radius * 2),
        style = Stroke(width = stroke, cap = StrokeCap.Round),
    )
}

/** The circle's small decisions, pinned by RoundPresentationTest. */
object RoundVideoRules {
    /**
     * The 8 dp dot (S5.2): somebody else's circle, one the server has, that
     * THIS DEVICE has not played. Never on the reader's own — they made it.
     */
    fun showsUnplayedDot(isMine: Boolean, acked: Boolean, played: Boolean): Boolean = !isMine && acked && !played

    /**
     * How far round the accent ring runs, 0…1. [stepped] — reduced motion —
     * moves it once a second instead of sweeping (S6).
     */
    fun progress(positionMs: Int, durationMs: Int, stepped: Boolean): Float {
        if (durationMs <= 0) return 0f
        val shown = if (stepped) positionMs / 1_000 * 1_000 else positionMs
        return (shown.toFloat() / durationMs).coerceIn(0f, 1f)
    }
}
