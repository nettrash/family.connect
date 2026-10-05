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
 *  - On it: the length in a capsule at the bottom, a 44 dp play disc in the
 *    middle, and an 8 dp accent dot beside the capsule until THIS DEVICE has
 *    played somebody else's circle.
 *  - A tap PLAYS IT IN PLACE through a TextureView — the one platform video
 *    surface a Compose clip can make round; a SurfaceView would punch a
 *    square hole — with a 3 dp accent ring running round the edge, a loading
 *    ring while the stream starts and a sentence when it fails
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
import android.view.Surface
import android.view.TextureView
import androidx.compose.foundation.Canvas
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

    val PLAY_DISC: Dp = 44.dp
    val UNPLAYED_DOT: Dp = 8.dp
    val PROGRESS_RING: Dp = 3.dp
    val EXPAND_TARGET: Dp = 44.dp
    val EXPAND_GLYPH: Dp = 28.dp
}

/** The wash under everything drawn on a picture — play disc, capsule, the failed line. */
private val SCRIM = Color.Black.copy(alpha = 0.45f)

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

    Box(modifier = modifier.size(diameter).testTag("round-video-${attachment.id}")) {
        Box(
            modifier = Modifier
                .fillMaxSize()
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
                RoundVideoPlayback.Phase.PLAYING -> Unit
                RoundVideoPlayback.Phase.IDLE, RoundVideoPlayback.Phase.PAUSED -> if (!sending) {
                    Box(
                        modifier = Modifier.size(RoundVideoDrawing.PLAY_DISC).clip(CircleShape).background(SCRIM),
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(Icons.Filled.PlayArrow, contentDescription = null, tint = Color.White)
                    }
                }
            }
            // The capsule at the bottom centre, inside the circle, with the
            // dot beside it.
            Row(
                modifier = Modifier.align(Alignment.BottomCenter).padding(bottom = 14.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
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
                    style = MaterialTheme.typography.labelSmall,
                    color = Color.White,
                    modifier = Modifier
                        .clip(RoundedCornerShape(8.dp))
                        .background(SCRIM)
                        .padding(horizontal = 6.dp, vertical = 2.dp),
                )
                if (showsDot) {
                    Box(
                        modifier = Modifier
                            .size(RoundVideoDrawing.UNPLAYED_DOT)
                            .clip(CircleShape)
                            .background(accent)
                            .testTag("round-video-unplayed"),
                    )
                }
            }
        }
        if (playingOrPaused) {
            val fraction = RoundVideoRules.progress(
                positionMs = playback!!.positionMs,
                durationMs = durationMs,
                stepped = reducedMotion,
            )
            Canvas(modifier = Modifier.fillMaxSize().testTag("round-video-progress")) {
                val stroke = RoundVideoDrawing.PROGRESS_RING.toPx()
                drawArc(
                    color = accent,
                    startAngle = -90f,
                    sweepAngle = 360f * fraction,
                    useCenter = false,
                    topLeft = Offset(stroke / 2, stroke / 2),
                    size = Size(size.width - stroke, size.height - stroke),
                    style = Stroke(width = stroke),
                )
            }
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
