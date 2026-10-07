/*
 * RecordSendButton.kt
 * Family Connect (Android)
 *
 * Voice in the Send slot (#79, docs/audio-video-messages-2026-10-04.md,
 * Phase 1): the composer's trailing slot and the row that takes the field's
 * place while a voice message is recorded.
 *
 * The slot is Send when there is something to send and a MICROPHONE when the
 * composer is empty (S1.3, ComposerSlot); it never changes size or place. Its
 * touch is handled here and only here, with `awaitEachGesture`:
 *
 *  - a FINGER, a STYLUS or a MOUSE's primary button is a click on release
 *    inside the button — whatever the slot is, and however long the press
 *    was held. On the microphone it records hands-free;
 *  - a MOUSE's SECONDARY button opens the microphone's menu (S1.6);
 *  - Enter or Space on the focused slot, and TalkBack's double tap, ACTIVATE
 *    it (S6).
 *
 * THERE IS NO HOLD (revised 2026-10-06). A long press on the microphone is
 * not a gesture: nothing records and nothing opens while the finger is down
 * — no DropdownMenu, no tooltip, no long-click action, no haptic — and when
 * it lifts inside it is an ordinary tap, so a slow or unsteady press is
 * never a dead button. Only a mouse's secondary click opens the menu.
 *
 * The activation guard (S1.1) is asked as a press GOES DOWN, not as it lifts:
 * a press that goes down while the guard runs is ignored whole, so a slow
 * second tap can neither send the recording the first tap started nor send
 * what a Stop just staged. Every press — and every Enter or Space — asks
 * [pressIgnored] at its down and remembers the answer until it comes up.
 *
 * A tap is a lift inside the button's 48-dp hit area wherever the press
 * wandered (S1.1). A lift that arrives already consumed is the system
 * cancelling the touch — an alert, the notification shade — and does
 * nothing.
 *
 * TalkBack (S6): "Record voice message", clicked as "start recording", the
 * dimmed reason as its state, "Stop and listen first" and "Delete recording"
 * while recording — and NO long-click action and NO tooltip.
 *
 * iOS counterpart: ios/FamilyConnect/Views/RecordSendButton.swift
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.snap
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.width
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import me.nettrash.familyconnect.data.repo.Waveform
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.indication
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.PressInteraction
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.VideoCameraFront
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.ripple
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.input.pointer.isSecondaryPressed
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.hideFromAccessibility
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.ui.chat.ComposerSlot.Slot

/** The orange of the 4:30 warning — words beside it too, never colour alone (S2.5). */
internal val VoiceWarningOrange = Color(0xFFE8710A)

/**
 * The composer's trailing slot (S1.3) — Send, the microphone, the Send
 * arrow, the Stop square, Save. 44-dp visual, 48-dp hit area (S1.1).
 */
@Composable
internal fun RecordSendButton(
    slot: Slot,
    /**
     * The slot was activated — the microphone, the Send arrow, the Stop
     * square — by a completed tap or click, Enter, Space or TalkBack.
     */
    onActivate: () -> Unit,
    /** Rows 4 and 5: Save, or Send. */
    onSend: () -> Unit,
    /** TalkBack's "Stop and listen first" while recording (S6). */
    onStopAndListen: () -> Unit,
    /** TalkBack's "Delete recording" while recording (S6). */
    onDeleteRecording: () -> Unit,
    /** The secondary click's menu: "Record voice message" (S1.6). */
    onRecordFromMenu: () -> Unit,
    /**
     * "Record video message" (#79, S1.6): the menu's second item and
     * TalkBack's action on the microphone — only when S1.2's **round
     * available** holds; null offers neither. Dimmed rows say why.
     */
    onRecordVideo: (() -> Unit)? = null,
    focusRequester: FocusRequester,
    modifier: Modifier = Modifier,
    /**
     * Asked the moment a press goes DOWN on the slot — a finger, a stylus or
     * a mouse; Enter or Space — with the slot it went down on: true when it
     * goes down while the activation guard runs. Such a press is ignored
     * WHOLE, however late it comes up (S1.1): a slow second tap on the arrow
     * must not send the recording the first tap started.
     */
    pressIgnored: (Slot) -> Boolean = { false },
) {
    val currentSlot by rememberUpdatedState(slot)
    val activate by rememberUpdatedState(onActivate)
    val send by rememberUpdatedState(onSend)
    val stopAndListen by rememberUpdatedState(onStopAndListen)
    val deleteRecording by rememberUpdatedState(onDeleteRecording)
    val minTouchPx = with(LocalDensity.current) { ComposerSlot.MIN_TARGET_ANDROID_DP.dp.toPx() }
    val ignoresPress by rememberUpdatedState(pressIgnored)
    val keyPress = remember { KeyPress() }

    val interactions = remember { MutableInteractionSource() }
    var menuOpen by remember { mutableStateOf(false) }

    val enabled = slot.acceptsActivation()
    val recording = slot == Slot.SendVoice || slot == Slot.StopRecording
    val label = RecordStrings.label(slot)?.let { stringResource(it) }
    val reason = (slot as? Slot.Dimmed)?.reason?.let { stringResource(RecordStrings.of(it)) }
    val startLabel = stringResource(R.string.s_start_recording_action)
    val stopLabel = stringResource(R.string.s_stop_and_listen_first)
    val deleteLabel = stringResource(R.string.s_delete_recording)
    val videoLabel = stringResource(R.string.s_record_video_message)
    val recordVideo by rememberUpdatedState(onRecordVideo)

    /** What activating the slot does. */
    val click: () -> Unit = {
        when (currentSlot) {
            Slot.Send, is Slot.Save -> send()
            else -> activate()
        }
    }

    Box(
        modifier = modifier
            .size(44.dp)
            .focusRequester(focusRequester)
            .onKeyEvent { event ->
                val activation = event.key == Key.Enter || event.key == Key.NumPadEnter ||
                    event.key == Key.Spacebar || event.key == Key.DirectionCenter
                if (!activation) return@onKeyEvent false
                when (event.type) {
                    KeyEventType.KeyDown -> {
                        if (!currentSlot.acceptsActivation()) return@onKeyEvent false
                        // The key's press goes down now, and the guard is asked
                        // now (S1.1). A held key repeats: it is still one press.
                        if (event.nativeKeyEvent.repeatCount == 0) keyPress.ignored = ignoresPress(currentSlot)
                        true
                    }
                    KeyEventType.KeyUp -> {
                        val ignored = keyPress.ignored
                        keyPress.ignored = null
                        if (!currentSlot.acceptsActivation()) return@onKeyEvent false
                        // On the key's release, as a click's: activation
                        // completes then (S2.2) — unless its press went down
                        // inside the guard, or went down somewhere else.
                        if (ignored == false) click()
                        true
                    }
                    else -> false
                }
            }
            // Dimmed rows stay focusable (they say why); the two that take
            // nothing — today's disabled Send and a blank Save — do not.
            .focusable(enabled = enabled, interactionSource = interactions)
            .clearAndSetSemantics {
                role = Role.Button
                if (label != null) contentDescription = label
                // Dimmed is not disabled: it stays focusable and says why (S1.3).
                if (reason != null) stateDescription = reason
                if (!enabled) disabled()
                onClick(label = if (slot.isMicrophone) startLabel else null) {
                    click()
                    true
                }
                if (recording) {
                    customActions = listOf(
                        CustomAccessibilityAction(stopLabel) {
                            stopAndListen()
                            true
                        },
                        CustomAccessibilityAction(deleteLabel) {
                            deleteRecording()
                            true
                        },
                    )
                } else if (slot.isMicrophone && onRecordVideo != null) {
                    // S6: TalkBack's "Record video message" on the microphone.
                    customActions = listOf(
                        CustomAccessibilityAction(videoLabel) {
                            recordVideo?.invoke()
                            true
                        },
                    )
                }
            }
            .pointerInput(Unit) {
                awaitEachGesture {
                    val first = awaitFirstDown(requireUnconsumed = false)
                    val atDown = currentSlot
                    if (first.type == PointerType.Mouse && currentEvent.buttons.isSecondaryPressed) {
                        // The microphone's secondary menu — a mouse's only,
                        // never a touch's long press (S1.6).
                        first.consume()
                        if (atDown.isMicrophone) menuOpen = true
                        waitForUpOrCancellation()?.consume()
                        return@awaitEachGesture
                    }
                    if (!atDown.acceptsActivation()) return@awaitEachGesture
                    // A click on release — but whether this press can BE one
                    // is decided now, as it goes down: inside the activation
                    // guard it is ignored whole, however long after the
                    // guard it lifts (S1.1). Nothing else is decided while
                    // it is down: a long press is not a gesture.
                    val ignored = ignoresPress(atDown)
                    // The hit test, in pixels: this button's own, grown to 48 dp.
                    fun inside(at: Offset): Boolean {
                        val extraX = ((minTouchPx - size.width) / 2f).coerceAtLeast(0f)
                        val extraY = ((minTouchPx - size.height) / 2f).coerceAtLeast(0f)
                        return at.x >= -extraX && at.x <= size.width + extraX &&
                            at.y >= -extraY && at.y <= size.height + extraY
                    }
                    first.consume()
                    val press = PressInteraction.Press(first.position)
                    interactions.tryEmit(press)
                    var finished = false
                    try {
                        while (true) {
                            val event = awaitPointerEvent()
                            val change = event.changes.firstOrNull { it.id == first.id } ?: break
                            if (!change.pressed) {
                                if (change.isConsumed) {
                                    // A lift that arrives consumed is the system's cancel.
                                    interactions.tryEmit(PressInteraction.Cancel(press))
                                } else {
                                    if (inside(change.position) && !ignored) click()
                                    interactions.tryEmit(PressInteraction.Release(press))
                                }
                                change.consume()
                                finished = true
                                break
                            }
                            change.consume()
                        }
                    } finally {
                        // Torn down mid-press (the button left composition):
                        // the press is let go, and nothing is activated.
                        if (!finished) interactions.tryEmit(PressInteraction.Cancel(press))
                    }
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        SlotFace(slot = slot, interactions = interactions)
        DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
            DropdownMenuItem(
                text = { Text(stringResource(R.string.s_record_voice_message)) },
                leadingIcon = { Icon(Icons.Filled.Mic, contentDescription = null) },
                onClick = {
                    menuOpen = false
                    onRecordFromMenu()
                },
            )
            if (onRecordVideo != null) {
                DropdownMenuItem(
                    text = { Text(videoLabel) },
                    leadingIcon = { Icon(Icons.Outlined.VideoCameraFront, contentDescription = null) },
                    onClick = {
                        menuOpen = false
                        onRecordVideo()
                    },
                )
            }
        }
    }
}

/** Whether activating the slot does anything at all (row 6 and a blank Save do not). */
internal fun Slot.acceptsActivation(): Boolean = when (this) {
    Slot.Recorder, Slot.SendDisabled -> false
    is Slot.Save -> enabled
    else -> true
}

/**
 * The Enter or Space press the slot is in the middle of: whether it went
 * down inside the activation guard (S1.1) — null when no press went down
 * here, and then its release activates nothing. Plain fields, not state:
 * nothing is drawn from it.
 */
private class KeyPress {
    var ignored: Boolean? = null
}

/** What the slot looks like: a 44-dp disc, the glyph cross-fading in 150 ms (S1.3). */
@Composable
private fun SlotFace(slot: Slot, interactions: MutableInteractionSource) {
    val live = when (slot) {
        Slot.Send, Slot.SendVoice, Slot.StopRecording, Slot.Microphone -> true
        is Slot.Save -> slot.enabled
        else -> false
    }
    val colors = MaterialTheme.colorScheme
    // 150 ms cross-fades (S1.3, the approved design); none without animations.
    val steady = animationsRemoved()
    val fade = if (steady) snap<Color>() else tween(ComposerSlot.SLOT_CROSSFADE_MS.toInt())
    val container by animateColorAsState(
        targetValue = if (live) colors.primary else colors.surfaceContainerHighest,
        animationSpec = fade,
        label = "slotContainer",
    )
    val content by animateColorAsState(
        targetValue = if (live) colors.onPrimary else colors.onSurfaceVariant,
        animationSpec = fade,
        label = "slotContent",
    )
    // Draw-only, so nothing beside it moves: a slot with nothing to do
    // settles back. Nothing swells under a finger — there is no hold — and
    // none of it moves without animations: it simply is the size it ends at.
    val motion = if (steady) snap<Float>() else tween(ComposerSlot.SLOT_CROSSFADE_MS.toInt())
    val scale by animateFloatAsState(
        targetValue = if (live) 1f else 0.9f,
        animationSpec = motion,
        label = "slotScale",
    )
    val glyph = when (slot) {
        Slot.StopRecording -> SlotGlyph.STOP
        Slot.Microphone, is Slot.Dimmed -> SlotGlyph.MICROPHONE
        else -> SlotGlyph.SEND
    }
    Box(
        modifier = Modifier
            .size(44.dp)
            .graphicsLayer {
                scaleX = scale
                scaleY = scale
            }
            .clip(CircleShape)
            .background(container)
            .indication(interactions, ripple()),
        contentAlignment = Alignment.Center,
    ) {
        Crossfade(
            targetState = glyph,
            animationSpec = if (steady) snap() else tween(ComposerSlot.SLOT_CROSSFADE_MS.toInt()),
            label = "slotGlyph",
        ) { shown ->
            Icon(
                imageVector = when (shown) {
                    SlotGlyph.SEND -> Icons.AutoMirrored.Filled.Send
                    SlotGlyph.MICROPHONE -> Icons.Filled.Mic
                    SlotGlyph.STOP -> Icons.Filled.Stop
                },
                // The slot's own semantics carry the label (clearAndSetSemantics).
                contentDescription = null,
                tint = content,
            )
        }
    }
}

private enum class SlotGlyph { SEND, MICROPHONE, STOP }

/** Whether the person asked Android to remove animations — S1.1's Reduce Motion. */
@Composable
internal fun animationsRemoved(): Boolean {
    val context = androidx.compose.ui.platform.LocalContext.current
    return remember(context) {
        android.provider.Settings.Global.getFloat(
            context.contentResolver,
            android.provider.Settings.Global.ANIMATOR_DURATION_SCALE,
            1f,
        ) == 0f
    }
}

/** The recording dot, pulsing between 100 % and 35 % once a second; steady without animations (S2.9). */
@Composable
internal fun RecordingDot(
    steady: Boolean,
    size: androidx.compose.ui.unit.Dp = 10.dp,
    color: Color = MaterialTheme.colorScheme.error,
) {
    val alpha = if (steady) {
        1f
    } else {
        val pulse = rememberInfiniteTransition(label = "recordingDot")
        pulse.animateFloat(
            initialValue = 1f,
            targetValue = 0.35f,
            animationSpec = infiniteRepeatable(tween(1_000, easing = FastOutSlowInEasing), RepeatMode.Reverse),
            label = "recordingDotAlpha",
        ).value
    }
    Box(
        modifier = Modifier
            .size(size)
            .graphicsLayer { this.alpha = alpha }
            .background(color, CircleShape)
            .testTag("voice-recording-dot"),
    )
}

/**
 * The LIVE waveform (the approved design): the meter's newest peaks as bars,
 * 3 × up to 24 units, 2 apart, in the recording red, scrolling in from the
 * trailing edge as each tick adds one. Without animations it does not scroll:
 * it steps once a second, as the circles' rings do (S6). Decoration to
 * TalkBack — the timer and the announcements carry the meaning.
 */
@Composable
internal fun LiveWaveform(levels: List<Int>, modifier: Modifier = Modifier, steady: Boolean = animationsRemoved()) {
    val red = MaterialTheme.colorScheme.error
    val latest by rememberUpdatedState(levels)
    var stepped by remember { mutableStateOf(levels) }
    if (steady) {
        LaunchedEffect(Unit) {
            while (true) {
                stepped = latest
                delay(1_000)
            }
        }
    }
    val shown = if (steady) stepped else levels
    val rtl = LocalLayoutDirection.current == LayoutDirection.Rtl
    Canvas(
        modifier = modifier
            .height(24.dp)
            .semantics { hideFromAccessibility() }
            .testTag("voice-live-waveform"),
    ) {
        val bar = 3.dp.toPx()
        val gap = 2.dp.toPx()
        val fits = ((size.width + gap) / (bar + gap)).toInt()
        val recent = shown.takeLast(fits)
        // Newest at the trailing edge; the rest run back from it.
        recent.asReversed().forEachIndexed { back, level ->
            val height = (size.height * Waveform.barFraction(level)).toFloat()
            val fromEnd = size.width - bar - back * (bar + gap)
            val x = if (rtl) size.width - bar - fromEnd else fromEnd
            drawRoundRect(
                color = red.copy(alpha = 0.8f),
                topLeft = Offset(x, (size.height - height) / 2f),
                size = Size(bar, height),
                cornerRadius = CornerRadius(bar / 2f, bar / 2f),
            )
        }
    }
}

/** The timer — m:ss in tabular digits, orange from 4:30 (S2.5, S2.9). Never announced. */
@Composable
private fun RecordingClock(recordedMs: Long) {
    val warning = VoiceNoteRules.showsTimeWarning(recordedMs)
    Text(
        text = VoiceNoteRules.clock(recordedMs),
        style = MaterialTheme.typography.bodyMedium.copy(fontFeatureSettings = "tnum", fontFamily = FontFamily.Monospace),
        color = if (warning) voiceWarning() else MaterialTheme.colorScheme.onSurface,
        maxLines = 1,
    )
}

/**
 * The orange of a warning, readable on the composer in either theme: the
 * design's #C2620C on a light surface, #F5A524 on a dark one — words beside it
 * too, never colour alone (S2.5).
 */
@Composable
internal fun voiceWarning(): Color =
    if (MaterialTheme.colorScheme.surface.luminance() < 0.5f) Color(0xFFF5A524) else Color(0xFFC2620C)

/**
 * The middle of a recording row: the live waveform — or, in its place, the
 * line that matters more (S2.5, S2.9). The waveform goes first when the
 * row is too narrow for it.
 */
@Composable
private fun MiddleOfTheRow(levels: List<Int>, line: ChatViewModel.VoiceLine?, modifier: Modifier = Modifier) {
    BoxWithConstraints(modifier = modifier, contentAlignment = Alignment.CenterStart) {
        when (line) {
            null -> if (maxWidth >= 27.dp) LiveWaveform(levels, Modifier.fillMaxWidth())
            else -> VoiceLineText(line)
        }
    }
}

/** One of the lines that take the waveform's place. */
@Composable
private fun VoiceLineText(line: ChatViewModel.VoiceLine) {
    when (line) {
        ChatViewModel.VoiceLine.THIRTY_SECONDS_LEFT -> Text(
            text = stringResource(R.string.s_thirty_seconds_left),
            style = MaterialTheme.typography.bodySmall,
            color = voiceWarning(),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        ChatViewModel.VoiceLine.CANT_HEAR -> Text(
            text = stringResource(R.string.s_cant_hear_microphone_muted),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.error,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/**
 * S2.4's recording row, hands-free (the approved design): Delete leading,
 * the pulsing dot, the timer and the live waveform in the middle, Stop
 * before the slot — which is the Send arrow — and no Stop when it started
 * beside words or staged items, where the SLOT is Stop (row 3).
 */
@Composable
internal fun RecordingRow(
    recordedMs: Long,
    @Suppress("UNUSED_PARAMETER") level: Int,
    line: ChatViewModel.VoiceLine?,
    besideDraft: Boolean,
    onDelete: () -> Unit,
    onStop: () -> Unit,
    modifier: Modifier = Modifier,
    /** The meter's newest peaks as waveform levels, oldest first (ChatViewModel.voiceLevels). */
    levels: List<Int> = emptyList(),
) {
    val steady = animationsRemoved()
    Row(
        modifier = modifier
            .clip(RoundedCornerShape(24.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .padding(horizontal = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        IconButton(onClick = onDelete) {
            Icon(
                imageVector = Icons.Outlined.Delete,
                contentDescription = stringResource(R.string.s_delete_recording),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        RecordingDot(steady)
        RecordingClock(recordedMs)
        MiddleOfTheRow(levels, line, Modifier.weight(1f))
        if (!besideDraft) {
            IconButton(onClick = onStop) {
                Icon(
                    imageVector = Icons.Filled.Stop,
                    contentDescription = stringResource(R.string.s_stop_recording),
                    tint = MaterialTheme.colorScheme.onSurface,
                )
            }
        }
    }
}

/**
 * The composer's polite live region (S6): state changes only — never the
 * ticking clock. Each announcement is held a few seconds and then cleared,
 * so swiping through the composer later does not read a stale one.
 */
@Composable
internal fun VoiceAnnouncer(announcement: ChatViewModel.VoiceAnnouncement?) {
    var spoken by remember { mutableStateOf("") }
    // Kept across a rotation, so a rebuilt screen does not say the last
    // thing again.
    var lastSerial by rememberSaveable { mutableLongStateOf(-1L) }
    LaunchedEffect(announcement) {
        val next = announcement ?: return@LaunchedEffect
        if (next.serial == lastSerial) return@LaunchedEffect
        lastSerial = next.serial
        // Cleared for a frame first, so the same words said twice in a row are
        // a change TalkBack hears.
        spoken = ""
        withFrameNanos { }
        spoken = next.text
        delay(ANNOUNCEMENT_HELD_MS)
        spoken = ""
    }
    Box(
        modifier = Modifier
            .size(1.dp)
            .semantics {
                liveRegion = LiveRegionMode.Polite
                if (spoken.isNotEmpty()) contentDescription = spoken
            },
    )
}

private const val ANNOUNCEMENT_HELD_MS = 4_000L

/**
 * The row in the field's place keeps a stray touch from reaching the field
 * hidden under it — which would focus it and raise the keyboard.
 */
internal fun Modifier.keepsTouchesFromTheField(): Modifier = pointerInput(Unit) {
    awaitEachGesture {
        awaitFirstDown(requireUnconsumed = false).consume()
        waitForUpOrCancellation()?.consume()
    }
}

/**
 * Whether S2.9's haptics play here: on phones only — none on a tablet, as
 * none on an iPad. A tablet is a smallest width of 600 dp or more — the
 * line S8.5 draws, where Android also stops honouring an orientation lock —
 * so an unfolded foldable counts as one and a phone turned sideways does not.
 */
internal fun voiceHapticsOn(smallestScreenWidthDp: Int): Boolean = smallestScreenWidthDp < 600

/** S2.9's haptics as Android names them. */
internal fun RecordGesture.Haptic.feedback(): HapticFeedbackType = when (this) {
    RecordGesture.Haptic.LIGHT -> HapticFeedbackType.ToggleOn
    RecordGesture.Haptic.SUCCESS -> HapticFeedbackType.Confirm
    RecordGesture.Haptic.WARNING -> HapticFeedbackType.Reject
}
