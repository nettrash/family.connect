/*
 * RecordSendButton.kt
 * Family Connect (Android)
 *
 * Voice in the Send slot (#79, docs/audio-video-messages-2026-10-04.md,
 * Phase 1): the composer's trailing slot and the rows that take the field's
 * place while a voice message is recorded or waits out its Undo window.
 *
 * The slot is Send when there is something to send and a MICROPHONE when the
 * composer is empty (S1.3, ComposerSlot); it never changes size or place. Its
 * touch is handled here and only here, with `awaitEachGesture`, because no
 * stock button can tell a tap from a hold from a slide:
 *
 *  - a FINGER or a STYLUS on the microphone is the reducer's press — Down,
 *    Move and Up in WINDOW coordinates (positionInRoot), so a slide is
 *    measured from where it began, never from the button, and in DP, S1.1's
 *    units, so 20 / 60 / 100 mean the same on every screen density — and
 *    becomes a tap or, at H, the walkie-talkie (S2.3);
 *  - a MOUSE clicks on release (a press of any length), and its SECONDARY
 *    button opens the microphone's menu (S8.4);
 *  - anything that is not the microphone — the Send arrow, the Stop square,
 *    Send, Save — is a plain click on release inside the button;
 *  - Enter or Space on the focused slot, and TalkBack's double tap, ACTIVATE
 *    it (S6): the microphone then records hands-free.
 *
 * The activation guard (S1.1) is asked as a press GOES DOWN, not as it lifts:
 * a press that goes down while the guard runs is ignored whole, so a slow
 * second tap can neither send the recording the first tap started nor send
 * what a Stop just staged. The microphone's touch gets that from the
 * reducer's Down; every other press — and every Enter or Space — asks
 * [pressIgnored] at its down and remembers the answer until it comes up.
 *
 * A tap is a lift inside the button's 48-dp hit area wherever the press
 * wandered (S1.1), so an unsteady press is never a dead button. A lift that
 * arrives already consumed is the system cancelling the touch — an alert,
 * the notification shade — and a gesture torn down mid-press (the activity
 * rebuilt) is the same thing: both are the reducer's SystemCancel, which
 * LOCKS a hold rather than losing it.
 *
 * TalkBack (S6): "Record voice message", clicked as "start recording", the
 * dimmed reason as its state, "Stop and listen first" and "Delete recording"
 * while recording — and NO long-click action and NO tooltip, so neither can
 * fight the hold (a TooltipBox's long press would).
 *
 * iOS counterpart: ios/FamilyConnect/Views/RecordSendButton.swift
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.animation.Crossfade
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
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
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.VideoCameraFront
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.ripple
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
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
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
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
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties
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
     * A finger, a stylus or a mouse went down on the MICROPHONE, in window
     * coordinates and in dp — S1.1's units (20 slop, 60 lock, 100 cancel).
     */
    onMicDown: (x: Double, y: Double, canHold: Boolean, rtl: Boolean) -> Unit,
    /** It moved — window coordinates, dp. */
    onMicMove: (x: Double, y: Double) -> Unit,
    /** It lifted — window coordinates, dp; `inside` is this button's own hit test. */
    onMicUp: (x: Double, y: Double, inside: Boolean) -> Unit,
    /** The system cancelled the touch, or the gesture was torn down mid-press. */
    onMicCancel: () -> Unit,
    /** The slot was activated otherwise: Enter, Space, TalkBack, a click on the arrow or the square. */
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
     * Asked the moment a press goes DOWN on the slot that is not the
     * microphone's own touch — a finger, a stylus or a mouse on the Send
     * arrow, the Stop square, Send or Save; Enter or Space on any slot — with
     * the slot it went down on: true when it goes down while the activation
     * guard runs. Such a press is ignored WHOLE, however late it comes up
     * (S1.1): a slow second tap on the arrow must not send the recording the
     * first tap started. (The microphone's touch is the reducer's Down,
     * which asks the guard itself.)
     */
    pressIgnored: (Slot) -> Boolean = { false },
    /** "You can also hold the microphone while you talk." above the slot (S7.2). */
    coachMark: Boolean = false,
    onDismissCoachMark: () -> Unit = {},
) {
    val currentSlot by rememberUpdatedState(slot)
    val down by rememberUpdatedState(onMicDown)
    val move by rememberUpdatedState(onMicMove)
    val up by rememberUpdatedState(onMicUp)
    val cancel by rememberUpdatedState(onMicCancel)
    val activate by rememberUpdatedState(onActivate)
    val send by rememberUpdatedState(onSend)
    val stopAndListen by rememberUpdatedState(onStopAndListen)
    val deleteRecording by rememberUpdatedState(onDeleteRecording)
    val rtl = LocalLayoutDirection.current == LayoutDirection.Rtl
    val currentRtl by rememberUpdatedState(rtl)
    val minTouchPx = with(LocalDensity.current) { ComposerSlot.MIN_TARGET_ANDROID_DP.dp.toPx() }
    val ignoresPress by rememberUpdatedState(pressIgnored)
    val keyPress = remember { KeyPress() }

    var coordinates by remember { mutableStateOf<LayoutCoordinates?>(null) }
    val interactions = remember { MutableInteractionSource() }
    var menuOpen by remember { mutableStateOf(false) }

    val enabled = slot.acceptsActivation()
    val recording = slot == Slot.HeldMicrophone || slot == Slot.SendVoice || slot == Slot.StopRecording
    val label = RecordStrings.label(slot)?.let { stringResource(it) }
    val reason = (slot as? Slot.Dimmed)?.reason?.let { stringResource(RecordStrings.of(it)) }
    val startLabel = stringResource(R.string.s_start_recording_action)
    val stopLabel = stringResource(R.string.s_stop_and_listen_first)
    val deleteLabel = stringResource(R.string.s_delete_recording)
    val videoLabel = stringResource(R.string.s_record_video_message)
    val recordVideo by rememberUpdatedState(onRecordVideo)

    /** What activating the slot does, other than the microphone's own touch. */
    val click: () -> Unit = {
        when (currentSlot) {
            Slot.Send, is Slot.Save -> send()
            else -> activate()
        }
    }

    Box(
        modifier = modifier
            .size(44.dp)
            .onGloballyPositioned { coordinates = it }
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
                        // The microphone's secondary menu — never a touch hold (S1.6).
                        first.consume()
                        if (atDown.isMicrophone) menuOpen = true
                        waitForUpOrCancellation()?.consume()
                        return@awaitEachGesture
                    }
                    // A second finger while one holds, or a slot that takes nothing.
                    if (atDown == Slot.HeldMicrophone || !atDown.acceptsActivation()) return@awaitEachGesture
                    val onMicrophone = atDown.isMicrophone
                    val canHold = first.type == PointerType.Touch || first.type == PointerType.Stylus ||
                        first.type == PointerType.Eraser
                    // Anything but the microphone is a click on release — but
                    // whether this press can BE one is decided now, as it goes
                    // down: inside the activation guard it is ignored whole,
                    // however long after the guard it lifts (S1.1).
                    val ignored = !onMicrophone && ignoresPress(atDown)
                    // Window coordinates, read at every event: if anything
                    // moved the button mid-press, a slide is still measured
                    // from where the finger went down on the WINDOW (S1.1).
                    // And in DP: S1.1's distances are units, dp on Android,
                    // while the pointer speaks pixels — handed pixels, the
                    // reducer's 20-dp slop would be under 7 dp on a 3x phone
                    // and its 60-dp lock 20. (`density` is this pointer
                    // scope's: the button's own, kept current by Compose.)
                    fun window(at: Offset): Offset =
                        ((coordinates?.takeIf { it.isAttached }?.positionInRoot() ?: Offset.Zero) + at) / density
                    // The hit test stays in pixels: it is this button's own.
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
                        if (onMicrophone) {
                            val at = window(first.position)
                            down(at.x.toDouble(), at.y.toDouble(), canHold, currentRtl)
                        }
                        while (true) {
                            val event = awaitPointerEvent()
                            val change = event.changes.firstOrNull { it.id == first.id } ?: break
                            if (!change.pressed) {
                                if (change.isConsumed) {
                                    // A lift that arrives consumed is the system's cancel.
                                    if (onMicrophone) cancel()
                                    interactions.tryEmit(PressInteraction.Cancel(press))
                                } else {
                                    val lifted = inside(change.position)
                                    if (onMicrophone) {
                                        val at = window(change.position)
                                        up(at.x.toDouble(), at.y.toDouble(), lifted)
                                    } else if (lifted && !ignored) {
                                        click()
                                    }
                                    interactions.tryEmit(PressInteraction.Release(press))
                                }
                                change.consume()
                                finished = true
                                break
                            }
                            if (onMicrophone && change.positionChange() != Offset.Zero) {
                                val at = window(change.position)
                                move(at.x.toDouble(), at.y.toDouble())
                            }
                            change.consume()
                        }
                    } finally {
                        // Torn down mid-press — the activity rebuilt around the
                        // ViewModel, the button left composition: a hold LOCKS
                        // (S4), a press is let go. Compose normally delivers its
                        // own cancel first (the consumed lift above, which
                        // RecordSendButtonTest drives); this is the backstop for
                        // a gesture cancelled without one.
                        if (!finished) {
                            if (onMicrophone) cancel()
                            interactions.tryEmit(PressInteraction.Cancel(press))
                        }
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
        if (slot == Slot.HeldMicrophone) LockHint()
        if (coachMark) VoiceCoachMark(onDismiss = onDismissCoachMark)
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
    val held = slot == Slot.HeldMicrophone
    val live = when (slot) {
        Slot.Send, Slot.SendVoice, Slot.StopRecording, Slot.Microphone, Slot.HeldMicrophone -> true
        is Slot.Save -> slot.enabled
        else -> false
    }
    val colors = MaterialTheme.colorScheme
    val container by animateColorAsState(
        targetValue = when {
            held -> colors.error
            live -> colors.primary
            else -> colors.surfaceContainerHighest
        },
        animationSpec = tween(ComposerSlot.SLOT_CROSSFADE_MS.toInt()),
        label = "slotContainer",
    )
    val content by animateColorAsState(
        targetValue = when {
            held -> colors.onError
            live -> colors.onPrimary
            else -> colors.onSurfaceVariant
        },
        animationSpec = tween(ComposerSlot.SLOT_CROSSFADE_MS.toInt()),
        label = "slotContent",
    )
    // Draw-only, so nothing beside it moves: the held microphone swells
    // under the finger, a slot with nothing to do settles back.
    val scale by animateFloatAsState(
        targetValue = when {
            held -> 1.2f
            live -> 1f
            else -> 0.9f
        },
        animationSpec = tween(ComposerSlot.SLOT_CROSSFADE_MS.toInt()),
        label = "slotScale",
    )
    val glyph = when (slot) {
        Slot.StopRecording -> SlotGlyph.STOP
        Slot.Microphone, Slot.HeldMicrophone, is Slot.Dimmed -> SlotGlyph.MICROPHONE
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
            animationSpec = tween(ComposerSlot.SLOT_CROSSFADE_MS.toInt()),
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

/**
 * A lock glyph over an up chevron, 8 dp above the held microphone and
 * outside the bar, so the bar keeps its height (S2.3). Not focusable: it
 * must not take the keyboard or the touch.
 */
@Composable
private fun LockHint() {
    val gap = with(LocalDensity.current) { 8.dp.roundToPx() }
    Popup(
        popupPositionProvider = remember(gap) { AboveTheSlot(gap, alignTrailing = false) },
        properties = PopupProperties(focusable = false, clippingEnabled = false),
    ) {
        Surface(
            shape = RoundedCornerShape(20.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
            shadowElevation = 2.dp,
            modifier = Modifier.semantics { hideFromAccessibility() },
        ) {
            Column(
                modifier = Modifier.padding(horizontal = 8.dp, vertical = 8.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Icon(Icons.Filled.Lock, contentDescription = null, modifier = Modifier.size(18.dp))
                Icon(Icons.Filled.KeyboardArrowUp, contentDescription = null, modifier = Modifier.size(18.dp))
            }
        }
    }
}

/**
 * S7.2's one coach mark: a small bubble above the microphone, once per
 * device. A plain Popup, never a TooltipBox — whose long press would fight
 * the hold — and not focusable, because a focusable popup closes the
 * keyboard and shifts the list.
 */
@Composable
private fun VoiceCoachMark(onDismiss: () -> Unit) {
    val gap = with(LocalDensity.current) { 8.dp.roundToPx() }
    Popup(
        popupPositionProvider = remember(gap) { AboveTheSlot(gap, alignTrailing = true) },
        onDismissRequest = onDismiss,
        properties = PopupProperties(focusable = false),
    ) {
        Surface(
            onClick = onDismiss,
            shape = RoundedCornerShape(12.dp),
            color = MaterialTheme.colorScheme.inverseSurface,
            contentColor = MaterialTheme.colorScheme.inverseOnSurface,
            shadowElevation = 4.dp,
            modifier = Modifier.widthIn(max = 260.dp),
        ) {
            Text(
                text = stringResource(R.string.s_hold_the_microphone_coach),
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
            )
        }
    }
}

/** Places a popup [gapPx] above its anchor: centred on it, or at its trailing edge. */
private class AboveTheSlot(private val gapPx: Int, private val alignTrailing: Boolean) : PopupPositionProvider {
    override fun calculatePosition(
        anchorBounds: IntRect,
        windowSize: IntSize,
        layoutDirection: LayoutDirection,
        popupContentSize: IntSize,
    ): IntOffset {
        val x = when {
            !alignTrailing -> anchorBounds.left + (anchorBounds.width - popupContentSize.width) / 2
            layoutDirection == LayoutDirection.Rtl -> anchorBounds.left
            else -> anchorBounds.right - popupContentSize.width
        }
        val maxX = (windowSize.width - popupContentSize.width).coerceAtLeast(0)
        val y = (anchorBounds.top - gapPx - popupContentSize.height).coerceAtLeast(0)
        return IntOffset(x.coerceIn(0, maxX), y)
    }
}

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

/** 8 dp, pulsing between 100 % and 40 % once a second; steady without animations (S2.9). */
@Composable
private fun RecordingDot(steady: Boolean) {
    val alpha = if (steady) {
        1f
    } else {
        val pulse = rememberInfiniteTransition(label = "recordingDot")
        pulse.animateFloat(
            initialValue = 1f,
            targetValue = 0.4f,
            animationSpec = infiniteRepeatable(tween(500), RepeatMode.Reverse),
            label = "recordingDotAlpha",
        ).value
    }
    Box(
        modifier = Modifier
            .size(8.dp)
            .graphicsLayer { this.alpha = alpha }
            .background(MaterialTheme.colorScheme.error, CircleShape),
    )
}

/** Five bars, 3 × 16 dp, lit at −50…−10 dBFS of the peak (S2.9). Decoration to TalkBack. */
@Composable
private fun LevelMeter(level: Int) {
    val ink = MaterialTheme.colorScheme.onSurface
    Row(
        horizontalArrangement = Arrangement.spacedBy(3.dp),
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.semantics { hideFromAccessibility() },
    ) {
        repeat(VoiceNoteRules.LEVEL_BARS_DBFS.size) { bar ->
            Box(
                modifier = Modifier
                    .size(width = 3.dp, height = 16.dp)
                    .background(
                        if (bar < level) ink else ink.copy(alpha = 0.2f),
                        RoundedCornerShape(1.5.dp),
                    ),
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
        style = MaterialTheme.typography.bodyMedium.copy(fontFeatureSettings = "tnum"),
        color = if (warning) VoiceWarningOrange else MaterialTheme.colorScheme.onSurface,
        maxLines = 1,
    )
}

/**
 * The middle of a recording row: the level meter — or, in its place, the
 * line that matters more (S2.3, S2.5, S2.9). The meter goes first when the
 * row is too narrow for it.
 */
@Composable
private fun MiddleOfTheRow(level: Int, line: ChatViewModel.VoiceLine?, modifier: Modifier = Modifier) {
    BoxWithConstraints(modifier = modifier, contentAlignment = Alignment.CenterStart) {
        when (line) {
            ChatViewModel.VoiceLine.THIRTY_SECONDS_LEFT -> Text(
                text = stringResource(R.string.s_thirty_seconds_left),
                style = MaterialTheme.typography.bodySmall,
                color = VoiceWarningOrange,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            ChatViewModel.VoiceLine.STILL_RECORDING -> Text(
                text = stringResource(R.string.s_still_recording_tap_send),
                style = MaterialTheme.typography.bodySmall,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            ChatViewModel.VoiceLine.CANT_HEAR -> Text(
                text = stringResource(R.string.s_cant_hear_microphone_muted),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            null -> if (maxWidth >= 27.dp) LevelMeter(level)
        }
    }
}

/**
 * S2.3's hold row, in the field's place while a finger holds the
 * microphone: the red dot, the timer, the level meter and "‹ Slide to
 * cancel" — red, and "Release to cancel", once cancel is armed.
 */
@Composable
internal fun HoldRow(
    recordedMs: Long,
    level: Int,
    line: ChatViewModel.VoiceLine?,
    armed: Boolean,
    modifier: Modifier = Modifier,
) {
    val steady = animationsRemoved()
    val container by animateColorAsState(
        targetValue = if (armed) {
            MaterialTheme.colorScheme.errorContainer
        } else {
            MaterialTheme.colorScheme.surfaceContainerHigh
        },
        label = "holdRow",
    )
    Row(
        modifier = modifier
            .clip(RoundedCornerShape(24.dp))
            .background(container)
            .padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        RecordingDot(steady)
        RecordingClock(recordedMs)
        MiddleOfTheRow(level, line, Modifier.weight(1f))
        if (armed) {
            Text(
                text = stringResource(R.string.s_release_to_cancel),
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onErrorContainer,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        } else {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    imageVector = Icons.AutoMirrored.Filled.KeyboardArrowLeft,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(18.dp),
                )
                Text(
                    text = stringResource(R.string.s_slide_to_cancel),
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

/**
 * S2.4's recording row, hands-free: Delete leading, the dot, the timer and
 * the meter in the middle, Stop before the slot — no Stop when it started
 * beside words or staged items, where the SLOT is Stop (row 3).
 */
@Composable
internal fun RecordingRow(
    recordedMs: Long,
    level: Int,
    line: ChatViewModel.VoiceLine?,
    besideDraft: Boolean,
    onDelete: () -> Unit,
    onStop: () -> Unit,
    modifier: Modifier = Modifier,
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
            )
        }
        RecordingDot(steady)
        RecordingClock(recordedMs)
        MiddleOfTheRow(level, line, Modifier.weight(1f))
        if (!besideDraft) {
            IconButton(onClick = onStop) {
                Icon(
                    imageVector = Icons.Filled.Stop,
                    contentDescription = stringResource(R.string.s_stop_recording),
                )
            }
        }
    }
}

/**
 * S2.6's Undo row, in the FIELD's place only — the paperclip, stickers and
 * `@ai` stay usable beside it: "[Undo] Sending voice message · 0:12", with a
 * 2-dp line along its bottom emptying over the window; without animations,
 * "Sending in 5" instead, once a second and never announced.
 */
@Composable
internal fun UndoRow(
    recordedMs: Long,
    /** What is left of the window as this row appears. */
    windowMs: Long,
    onUndo: () -> Unit,
    modifier: Modifier = Modifier,
    steady: Boolean = animationsRemoved(),
) {
    Box(
        modifier = modifier
            .clip(RoundedCornerShape(24.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh),
    ) {
        Row(
            modifier = Modifier
                .fillMaxSize()
                .padding(start = 4.dp, end = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            TextButton(onClick = onUndo) { Text(stringResource(R.string.s_undo)) }
            Text(
                text = stringResource(R.string.s_sending_voice_message, VoiceNoteRules.clock(recordedMs)),
                style = MaterialTheme.typography.bodyMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            if (steady) {
                var secondsLeft by remember { mutableIntStateOf(((windowMs + 999) / 1000).toInt()) }
                LaunchedEffect(Unit) {
                    while (secondsLeft > 1) {
                        delay(1_000)
                        secondsLeft--
                    }
                }
                Text(
                    text = stringResource(R.string.s_sending_in, secondsLeft),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                )
            }
        }
        if (!steady) {
            val left = remember { Animatable(1f) }
            LaunchedEffect(Unit) {
                left.animateTo(0f, tween(windowMs.toInt().coerceAtLeast(0), easing = LinearEasing))
            }
            Box(
                modifier = Modifier
                    .align(Alignment.BottomStart)
                    .fillMaxWidth(left.value)
                    .height(2.dp)
                    .background(MaterialTheme.colorScheme.primary),
            )
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
 * The rows in the field's place keep a stray touch from reaching the field
 * hidden under them — which would focus it and raise the keyboard.
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
    RecordGesture.Haptic.MEDIUM -> HapticFeedbackType.LongPress
    RecordGesture.Haptic.SELECTION -> HapticFeedbackType.GestureThresholdActivate
    RecordGesture.Haptic.SUCCESS -> HapticFeedbackType.Confirm
    RecordGesture.Haptic.WARNING -> HapticFeedbackType.Reject
}

/**
 * Whether a cancelled touch on the microphone means the app has gone to the
 * BACKGROUND — the reducer then stops and keeps a hold instead of locking it
 * (S2.3). A configuration change tears the gesture down from an activity
 * already past STARTED, and is never the background: the hold locks (S4).
 */
internal fun touchCancelIsBackground(changingConfigurations: Boolean, started: Boolean): Boolean =
    !changingConfigurations && !started
