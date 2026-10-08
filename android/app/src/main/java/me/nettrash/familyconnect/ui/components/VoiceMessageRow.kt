/*
 * VoiceMessageRow.kt
 * Family Connect (Android)
 *
 * A VOICE MESSAGE in a bubble, as the approved design draws it (#79; the
 * issue's design page, "Voice message in the chat"; docs/protocol.md, "A
 * voice note's waveform"):
 *
 *  - a round ACCENT play/pause disc, 40 dp (a 48 dp target);
 *  - the WAVEFORM: the 48 levels the sender's meter made, reduced to the bars
 *    that fit (3 dp, 2 dp apart), each (2 + level) / 17 of the height — or a
 *    flat placeholder when the attachment carries none (a picked sound file,
 *    an older sender). Played bars take the accent as it plays. A tap or a
 *    drag on it seeks; to TalkBack it is an adjustable value;
 *  - under it the length at rest and the elapsed time while it plays, in
 *    tabular digits, an accent DOT until THIS DEVICE has started playing
 *    somebody else's message (the round video's played-store pattern, per
 *    account; gone the moment it starts, as on iOS, Windows and the web),
 *    and — once it has started — the SPEED chip, 1× → 1.5× → 2×, remembered
 *    per device and applied through MediaPlayer's PlaybackParams.
 *
 * "Show text" stays under it, drawn by the bubble (AttachmentGroup's footer).
 *
 * The colours are the theme's roles, never fixed values: the accent is
 * `primary` — on my own `primaryContainer` balloon as on theirs — the bars not
 * yet played are the content colour (theirs) or the accent (mine) at low
 * alpha, and the quiet text is the content colour at 70 %, so dark mode and
 * dynamic colour follow by themselves.
 *
 * MediaPlayer rather than ExoPlayer, as before: `setDataSource(context, uri,
 * headers)` carries the Authorization header the stream needs.
 *
 * iOS/macOS counterpart: ios/FamilyConnect/Views/AudioPlayerView.swift
 */

package me.nettrash.familyconnect.ui.components

import android.media.MediaPlayer
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.hideFromAccessibility
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.min
import androidx.core.net.toUri
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.repo.Waveform
import me.nettrash.familyconnect.ui.chat.LocalPlaybackCoordinator
import me.nettrash.familyconnect.ui.chat.LocalRecordingGate
import me.nettrash.familyconnect.ui.chat.PlaybackCoordinator
import me.nettrash.familyconnect.ui.chat.VoiceSpeed

/** The numbers a voice bubble is drawn at (the approved design). */
object VoiceDrawing {
    /** The play disc. */
    val PLAY_DISC: Dp = 40.dp

    /** Its touch target. */
    val PLAY_TARGET: Dp = 48.dp

    /** The waveform's height in the bubble. */
    val WAVE_HEIGHT: Dp = 28.dp

    /** One bar, and the gap between two. */
    val BAR: Dp = 3.dp
    val GAP: Dp = 2.dp

    /** The unplayed dot. */
    val DOT: Dp = 7.dp

    /** The bubble's own width: the design's 300, never wider than an attachment may be. */
    val WIDTH: Dp = 260.dp

    /** The staged and not-sent chips' play disc and waveform. */
    val CHIP_DISC: Dp = 30.dp
    val CHIP_WAVE_HEIGHT: Dp = 22.dp
}

/** A voice bubble's small decisions, pinned by VoiceMessageRowTest. */
object VoiceBubbleRules {
    /** The dot: somebody else's message, one the server has, that THIS DEVICE has not played. */
    fun showsUnplayedDot(isMine: Boolean, acked: Boolean, played: Boolean): Boolean = !isMine && acked && !played

    /** How many bars of [bar] wide, [gap] apart, fit [width] — at least one, at most the 48 there are. */
    fun barCount(width: Float, bar: Float, gap: Float): Int {
        if (width <= 0f || bar <= 0f) return 0
        val fits = ((width + gap) / (bar + gap)).toInt()
        return fits.coerceIn(1, Waveform.LEVELS)
    }

    /** Where a touch at [x] of [width] seeks to — mirrored when the layout runs right to left. */
    fun seekTo(x: Float, width: Float, durationMs: Long, rtl: Boolean): Long {
        if (width <= 0f || durationMs <= 0L) return 0L
        val fraction = (x / width).coerceIn(0f, 1f).let { if (rtl) 1f - it else it }
        return (fraction * durationMs).toLong().coerceIn(0L, durationMs)
    }

    /** The chip's words: 1×, 1.5×, 2× (a decimal comma where the language writes one). */
    fun speedLabel(speed: Float): Int = when (VoiceSpeed.normalised(speed)) {
        1.5f -> R.string.s_speed_1_5x
        2f -> R.string.s_speed_2x
        else -> R.string.s_speed_1x
    }
}

/**
 * The bars of [levels] that fit the width, [played] of them in [active], the
 * rest in [inactive]. Mirrored in a right-to-left layout, where time runs from
 * the right. Decoration to TalkBack: whoever draws it says what it means.
 */
@Composable
fun WaveformBars(
    levels: List<Int>,
    played: (bars: Int) -> Int,
    active: Color,
    inactive: Color,
    modifier: Modifier = Modifier,
    /** Hidden from TalkBack; false where the caller gives the bars a meaning of their own (the bubble's slider). */
    decorative: Boolean = true,
) {
    val rtl = LocalLayoutDirection.current == LayoutDirection.Rtl
    Canvas(modifier = if (decorative) modifier.semantics { hideFromAccessibility() } else modifier) {
        val bar = VoiceDrawing.BAR.toPx()
        val gap = VoiceDrawing.GAP.toPx()
        val count = VoiceBubbleRules.barCount(size.width, bar, gap)
        if (count == 0) return@Canvas
        val shown = Waveform.bars(levels, count)
        val lit = played(count)
        // Spread the bars across the whole width, the leftover shared by the gaps.
        val step = if (count > 1) (size.width - bar) / (count - 1) else 0f
        val radius = CornerRadius(bar / 2f, bar / 2f)
        shown.forEachIndexed { index, level ->
            val height = (size.height * Waveform.barFraction(level)).toFloat().coerceAtLeast(bar)
            val left = index * step
            val x = if (rtl) size.width - bar - left else left
            drawRoundRect(
                color = if (index < lit) active else inactive,
                topLeft = Offset(x, (size.height - height) / 2f),
                size = Size(bar, height),
                cornerRadius = radius,
            )
        }
    }
}

/** The design's round accent disc with a white play or pause glyph. */
@Composable
fun VoicePlayDisc(
    playing: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    disc: Dp = VoiceDrawing.PLAY_DISC,
    /** A recording runs: dimmed, not disabled — it says why (S1.7). */
    dimmed: Boolean = false,
    dimmedReason: String? = null,
) {
    val accent = MaterialTheme.colorScheme.primary
    val label = stringResource(if (playing) R.string.s_pause else R.string.s_play)
    Box(
        modifier = modifier
            .size(maxOf(disc, VoiceDrawing.PLAY_TARGET))
            .clip(CircleShape)
            .clickable(role = Role.Button, onClickLabel = label, onClick = onClick)
            .semantics {
                contentDescription = label
                if (dimmed && dimmedReason != null) stateDescription = dimmedReason
            },
        contentAlignment = Alignment.Center,
    ) {
        Box(
            modifier = Modifier
                .size(disc)
                .clip(CircleShape)
                .background(accent.copy(alpha = if (dimmed) 0.38f else 1f)),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = if (playing) Icons.Filled.Pause else Icons.Filled.PlayArrow,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onPrimary,
                modifier = Modifier.size(disc * 0.55f),
            )
        }
    }
}

/** The waveform's colours on this balloon: accent played, a quiet tone not yet. */
@Composable
fun waveformInactive(isMine: Boolean): Color =
    if (isMine) MaterialTheme.colorScheme.primary.copy(alpha = 0.38f) else LocalContentColor.current.copy(alpha = 0.28f)

/** m:ss, as the timer and every length read. */
private fun clock(ms: Long): String {
    val whole = (ms / 1000).coerceAtLeast(0)
    return "%d:%02d".format(whole / 60, whole % 60)
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun VoiceMessageRow(
    attachment: AttachmentDto,
    streamUrl: suspend (Long) -> Pair<String, Map<String, String>>?,
    onLongPress: () -> Unit,
    onDoubleTap: () -> Unit,
    modifier: Modifier = Modifier,
    /** The reader's own: no unplayed dot. */
    isMine: Boolean = false,
    /** The server has it; before that it is the sender's own on its way up. */
    acked: Boolean = true,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val ink = LocalContentColor.current
    val accent = MaterialTheme.colorScheme.primary
    val rtl = LocalLayoutDirection.current == LayoutDirection.Rtl

    val coordinator = LocalPlaybackCoordinator.current ?: remember { PlaybackCoordinator() }
    val gate = LocalRecordingGate.current
    val speed by coordinator.voiceSpeed.speed.collectAsStateWithLifecycle()
    val playedIds by coordinator.playedVoice.ids.collectAsStateWithLifecycle()
    val showsDot = VoiceBubbleRules.showsUnplayedDot(isMine, acked, attachment.id in playedIds)
    val played = !isMine && acked && attachment.id in playedIds

    val totalMs = (attachment.durationMs ?: 0).toLong().coerceAtLeast(1L)
    val levels = remember(attachment.waveform) { Waveform.levelsOrPlaceholder(attachment.waveform) }
    var player by remember(attachment.id) { mutableStateOf<MediaPlayer?>(null) }
    var isPlaying by remember(attachment.id) { mutableStateOf(false) }
    /** Started at least once and not yet run to its end: the speed chip shows, the clock counts up. */
    var underway by remember(attachment.id) { mutableStateOf(false) }
    var positionMs by remember(attachment.id) { mutableLongStateOf(0L) }
    var scrubbing by remember(attachment.id) { mutableStateOf(false) }
    val currentSpeed by rememberUpdatedState(speed)

    val pauseThis: () -> Unit = remember(attachment.id) {
        {
            player?.runCatching { if (isPlaying) pause() }
            isPlaying = false
        }
    }
    LaunchedEffect(gate.recording) { if (gate.recording && isPlaying) pauseThis() }
    DisposableEffect(attachment.id) {
        onDispose {
            coordinator.stopped(pauseThis)
            player?.runCatching { release() }
            player = null
        }
    }
    LaunchedEffect(isPlaying) {
        while (isPlaying) {
            if (!scrubbing) positionMs = (player?.currentPosition?.toLong() ?: positionMs)
            delay(100)
        }
    }
    // A new speed, from the chip or the menu, applies at once to what plays.
    LaunchedEffect(speed, isPlaying) {
        val active = player
        if (isPlaying && active != null) active.applySpeed(speed)
    }

    val seek: (Long) -> Unit = { target ->
        positionMs = target.coerceIn(0L, totalMs)
        player?.runCatching { seekTo(positionMs.toInt()) }
    }

    val waitsForTheRecording = stringResource(R.string.s_play_after_recording)
    val toggle: () -> Unit = toggle@{
        // Dimmed, not disabled: it says why (S1.7).
        if (gate.recording) {
            gate.explain()
            return@toggle
        }
        val active = player
        if (isPlaying && active != null) {
            active.pause()
            isPlaying = false
            coordinator.stopped(pauseThis)
            return@toggle
        }
        scope.launch {
            val ready = active ?: createVoicePlayer(context, attachment, streamUrl) {
                isPlaying = false
                underway = false
                positionMs = 0L
                coordinator.stopped(pauseThis)
            }
            if (ready == null) return@launch
            player = ready
            // Replaying after it ran to the end starts from the top; a seek
            // made before the first play starts where it was put.
            if (positionMs >= totalMs - 200) positionMs = 0L
            if (positionMs > 0L) ready.runCatching { seekTo(positionMs.toInt()) }
            ready.start()
            ready.applySpeed(currentSpeed)
            isPlaying = true
            underway = true
            coordinator.started(pauseThis)
            // Its dot goes the moment it STARTS, as on every other client
            // (the approved design: the dot is "you haven't played it yet").
            if (!isMine && acked) coordinator.playedVoice.markPlayed(attachment.id)
        }
    }

    val label = stringResource(R.string.s_voice_message_a11y, clock(totalMs))
    val notPlayed = stringResource(R.string.s_not_played)
    val playedWord = stringResource(R.string.s_played)
    // Moved from the start — playing, paused part-way, or put somewhere by a
    // seek: the clock and the accent bars say where; at rest, the length.
    val moved = isPlaying || underway || positionMs > 0L
    val shownMs = if (moved) positionMs else totalMs
    val inactive = waveformInactive(isMine)
    val quiet = ink.copy(alpha = 0.7f)

    val playWord = stringResource(if (isPlaying) R.string.s_pause else R.string.s_play)
    Row(
        modifier = modifier
            .width(min(VoiceDrawing.WIDTH, attachmentMaxWidth()))
            // TalkBack's double tap on the bubble — where the waveform's label,
            // value and seek action merge — plays or pauses it, as the ▶ does.
            // Semantics only, and ahead of combinedClickable so it is this
            // node's click: a finger's tap on the bubble still does nothing.
            .semantics { onClick(label = playWord) { toggle(); true } }
            .combinedClickable(
                interactionSource = null,
                indication = null,
                onClick = {},
                onLongClick = onLongPress,
                onDoubleClick = onDoubleTap,
            )
            .padding(start = 0.dp, end = 4.dp, top = 2.dp, bottom = 2.dp)
            .testTag("voice-message-${attachment.id}"),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        VoicePlayDisc(
            playing = isPlaying,
            onClick = toggle,
            dimmed = gate.recording,
            dimmedReason = waitsForTheRecording,
            modifier = Modifier.testTag("voice-play-${attachment.id}"),
        )
        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            val seekState by rememberUpdatedState(seek)
            WaveformBars(
                levels = levels,
                played = { bars -> if (moved) Waveform.playedBars(positionMs, totalMs, bars) else 0 },
                active = accent,
                inactive = inactive,
                decorative = false,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(VoiceDrawing.WAVE_HEIGHT)
                    .testTag("voice-wave-${attachment.id}")
                    // A tap seeks; a long press is still the bubble's menu.
                    .pointerInput(attachment.id, totalMs, rtl) {
                        detectTapGestures(
                            onTap = { at -> seekState(VoiceBubbleRules.seekTo(at.x, size.width.toFloat(), totalMs, rtl)) },
                            onLongPress = { onLongPress() },
                        )
                    }
                    // A drag scrubs, and lands where it is let go.
                    .pointerInput(attachment.id, totalMs, rtl) {
                        detectHorizontalDragGestures(
                            onDragStart = { scrubbing = true },
                            onDragEnd = { scrubbing = false },
                            onDragCancel = { scrubbing = false },
                        ) { change, _ ->
                            change.consume()
                            seekState(VoiceBubbleRules.seekTo(change.position.x, size.width.toFloat(), totalMs, rtl))
                        }
                    }
                    .semantics(mergeDescendants = false) {
                        contentDescription = label
                        // The value TalkBack adjusts: where it is, of how long.
                        progressBarRangeInfo = ProgressBarRangeInfo(
                            current = shownMs.coerceIn(0L, totalMs).toFloat(),
                            range = 0f..totalMs.toFloat(),
                        )
                        stateDescription = when {
                            gate.recording -> waitsForTheRecording
                            moved -> "${clock(positionMs)} / ${clock(totalMs)}"
                            showsDot -> notPlayed
                            played -> playedWord
                            else -> clock(totalMs)
                        }
                        setProgress { target ->
                            seekState(target.toLong())
                            true
                        }
                    },
            )
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                modifier = Modifier.fillMaxWidth().heightIn(min = 20.dp),
            ) {
                Text(
                    text = clock(shownMs),
                    style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"),
                    color = quiet,
                    maxLines = 1,
                    modifier = Modifier.semantics { hideFromAccessibility() },
                )
                if (showsDot) {
                    Box(
                        modifier = Modifier
                            .size(VoiceDrawing.DOT)
                            .clip(CircleShape)
                            .background(accent)
                            .testTag("voice-unplayed-${attachment.id}"),
                    )
                }
                Box(modifier = Modifier.weight(1f))
                if (underway || isPlaying) {
                    VoiceSpeedChip(
                        speed = speed,
                        onStep = { coordinator.voiceSpeed.step() },
                        modifier = Modifier.testTag("voice-speed-${attachment.id}"),
                    )
                }
            }
        }
    }
}

/**
 * The speed chip (the approved design): "1×", "1.5×" or "2×" in the accent on
 * a soft accent ground; a tap steps it, for every voice message on this
 * device. To TalkBack, "Playback speed, 1.5×".
 */
@Composable
fun VoiceSpeedChip(speed: Float, onStep: () -> Unit, modifier: Modifier = Modifier) {
    val accent = MaterialTheme.colorScheme.primary
    val name = stringResource(VoiceBubbleRules.speedLabel(speed))
    val label = stringResource(R.string.s_playback_speed_a11y, name)
    // Small to the eye; Compose widens the touch to the 48-unit minimum by
    // itself, so the row does not grow when the chip appears.
    Box(
        modifier = modifier
            .clickable(role = Role.Button, onClick = onStep)
            .clearAndSetSemantics {
                contentDescription = label
                role = Role.Button
                onClick { onStep(); true }
            },
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = name,
            style = MaterialTheme.typography.labelSmall.copy(fontWeight = FontWeight.Bold, fontFeatureSettings = "tnum"),
            color = accent,
            maxLines = 1,
            modifier = Modifier
                .clip(RoundedCornerShape(9.dp))
                .background(accent.copy(alpha = 0.14f))
                .padding(horizontal = 7.dp, vertical = 1.dp),
        )
    }
}

/** Speed through PlaybackParams; a player that refuses it plays at 1×. */
private fun MediaPlayer.applySpeed(speed: Float) {
    runCatching { playbackParams = playbackParams.setSpeed(speed) }
}

/**
 * A MediaPlayer pointed at the stream, with the auth header attached.
 * Returns null when it cannot be prepared — a bubble that will not play is
 * better than a crash.
 */
private suspend fun createVoicePlayer(
    context: android.content.Context,
    attachment: AttachmentDto,
    streamUrl: suspend (Long) -> Pair<String, Map<String, String>>?,
    onCompleted: () -> Unit,
): MediaPlayer? {
    val entry = streamUrl(attachment.id) ?: return null
    return withContext(Dispatchers.IO) {
        runCatching {
            MediaPlayer().apply {
                setDataSource(context, entry.first.toUri(), entry.second)
                setOnCompletionListener { onCompleted() }
                prepare()
            }
        }.getOrNull()
    }
}
