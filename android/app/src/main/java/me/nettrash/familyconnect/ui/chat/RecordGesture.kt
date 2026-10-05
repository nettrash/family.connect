/*
 * RecordGesture.kt
 * Family Connect (Android)
 *
 * What a press, a hold, a slide, a release, a timer, the length limit or an
 * interruption does to a voice recording in the Send slot (#79,
 * docs/audio-video-messages-2026-10-04.md, S2.1, S2.3, S2.5, S2.6) — as a
 * reducer: a state and an event go in, the next state and what to do come
 * out.
 *
 * A PORT of `fc_text::record::hold_step` (web/text/src/record.rs), branch for
 * branch and under the same names, held to the reference by
 * RecordGestureVectorsTest over every `hold_step` case in
 * record-vectors.json — so a finger on this phone does what a finger on the
 * iPhone does. Do not "improve" a branch here: change the reference, print
 * the vectors again, and port the change.
 *
 * It is only the DECISION. ChatViewModel feeds it — the microphone's touch
 * from RecordSendButton, the timers, the recorder's own endings, the
 * interruptions — and carries out what comes back: opening the microphone,
 * staging, parking, sending, the haptics, the words shown and spoken.
 *
 * The contract the port keeps (the reference's module notes):
 *  - TWO CLOCKS. `atMs` is one monotonic clock for the press, the guard and
 *    the Undo window. `recordedMs` is the recorder's own clock — the one the
 *    person sees, which with a screen reader starts once "Recording" has
 *    been spoken.
 *  - Down, Move, Up and SystemCancel are the MICROPHONE's touch and matter
 *    only from Idle. Every other activation of the slot is Activate. The
 *    lift of a press that went down on the microphone is always Up.
 *  - The hold begins only on a Tick at H, which the port sends from its own
 *    long-press timer.
 *  - After the slot's own Send or Save empties the composer the port sends
 *    Emptied, and it asks [HoldState.guarded] before acting on a row-5 Send.
 *  - The 4:30 warning, the 3-second silence warning and the level meter are
 *    display state the port derives from the constants (VoiceNoteRules).
 *
 * Nothing here touches Android, so a plain JUnit test pins it.
 *
 * iOS counterpart: ios/FamilyConnect/Models/RecordGesture.swift
 */

package me.nettrash.familyconnect.ui.chat

import me.nettrash.familyconnect.ui.chat.ComposerSlot.saturatingAdd
import me.nettrash.familyconnect.ui.chat.ComposerSlot.saturatingSub

object RecordGesture {

    /** The numbers [holdStep] decides by: S1.1's, with H for this system. */
    data class HoldConstants(
        val holdThresholdMs: Long = ComposerSlot.MIN_HOLD_THRESHOLD_MS,
        val tapSlop: Double = ComposerSlot.TAP_SLOP,
        val lockDistance: Double = ComposerSlot.LOCK_DISTANCE,
        val cancelArmDistance: Double = ComposerSlot.CANCEL_ARM_DISTANCE,
        val cancelDisarmDistance: Double = ComposerSlot.CANCEL_DISARM_DISTANCE,
        val shortestRecordingMs: Long = ComposerSlot.SHORTEST_RECORDING_MS,
        val undoWindowMs: Long = ComposerSlot.UNDO_WINDOW_MS,
        val activationGuardMs: Long = ComposerSlot.ACTIVATION_GUARD_MS,
        val deleteAsksFromMs: Long = ComposerSlot.DELETE_ASKS_FROM_MS,
    ) {
        companion object {
            /** S1.1's numbers, with H for a system whose long press takes [systemLongPressMs]. */
            fun forSystem(systemLongPressMs: Long): HoldConstants =
                HoldConstants(holdThresholdMs = ComposerSlot.holdThresholdMs(systemLongPressMs))
        }
    }

    /** The microphone permission, as the platform reads it. */
    enum class Permission {
        GRANTED,

        /** Activation raises the system's prompt. */
        NOT_ASKED,

        /** Refused: the denial notice, with Open Settings where the platform can. */
        DENIED,
    }

    /**
     * The facts a decision reads at the moment it is made. The defaults are a
     * granted microphone, nothing in the way, no screen reader, a device that
     * has released before, and Review Before Sending off.
     */
    data class Situation(
        val permission: Permission = Permission.GRANTED,
        /** The dimmed row the microphone is in (rows 7–9), if any. */
        val blocked: ComposerSlot.Dimmed? = null,
        /** A screen reader or Switch Control runs: a held release goes to review. */
        val assistive: Boolean = false,
        /** This device has not yet had its first held release taught. */
        val firstRelease: Boolean = false,
        /** The per-device setting (S9): a held release goes to review. */
        val reviewBeforeSending: Boolean = false,
    )

    /** Where a recording that waits on the permission prompt came from. */
    enum class Source {
        /** A completed tap, a click, Enter or Space, a screen reader's activation. */
        TAP,

        /** The hold threshold: a prompt raised by a hold never records. */
        HOLD,

        /** "Record voice message" from the paperclip or the menu, or the shortcut. */
        MENU,
    }

    /** Where the slot's voice recording is. */
    sealed interface Phase {
        /** Nothing pressed, nothing recording. A released note may still be in its Undo window. */
        data object Idle : Phase

        /** A finger or pen is down on the microphone, before H; nothing records. */
        data class Pressed(
            val downAtMs: Long,
            /** Where it went down, in window coordinates. */
            val downX: Double,
            val downY: Double,
            /** The layout is right-to-left: the leading edge is on the right. */
            val rtl: Boolean,
            /** It can still become a hold. */
            val mayHold: Boolean,
        ) : Phase

        /** Recording, the finger still down: the hold row. */
        data class Holding(
            val downX: Double,
            val downY: Double,
            val rtl: Boolean,
            /** Slide-to-cancel is armed: "Release to cancel". */
            val armed: Boolean,
        ) : Phase

        /** Recording, hands-free: the recording row. */
        data class HandsFree(
            /** Started with words typed or items staged (row 3). */
            val besideDraft: Boolean,
        ) : Phase

        /** Stopped by Delete at 10 s or more; "Delete this recording?" is asked. */
        data class AskingDelete(val recordedMs: Long) : Phase

        /** The system's microphone prompt is up. */
        data class AwaitingPermission(val source: Source, val besideDraft: Boolean) : Phase
    }

    /** A released note waiting out its Undo window: nothing has left the device. */
    data class UndoNote(
        /** When the window runs out and the note goes to the outbox. */
        val untilMs: Long,
        val recordedMs: Long,
    )

    /** The reducer's whole state: the phase, the activation guard, a note in its Undo window. */
    data class HoldState(
        val phase: Phase = Phase.Idle,
        /** Activation of the slot before this moment is ignored whole. 0: none. */
        val guardUntilMs: Long = 0,
        val undo: UndoNote? = null,
    ) {
        /** What [ComposerSlot.composerSlot] is told about this recording. */
        val recording: ComposerSlot.Recording
            get() = when (val phase = phase) {
                is Phase.Holding -> ComposerSlot.Recording.HELD
                is Phase.HandsFree ->
                    if (phase.besideDraft) {
                        ComposerSlot.Recording.HANDS_FREE_BESIDE_DRAFT
                    } else {
                        ComposerSlot.Recording.HANDS_FREE
                    }
                else -> ComposerSlot.Recording.NONE
            }

        /** The slot ignores activation at [atMs] — a row-5 Send included. */
        fun guarded(atMs: Long): Boolean = atMs < guardUntilMs
    }

    /** What happened. Every event carries [atMs], on the same monotonic clock. */
    sealed interface HoldEvent {
        val atMs: Long

        /** A press went down on the microphone; [canHold] for a finger or pen. */
        data class Down(
            override val atMs: Long,
            val x: Double,
            val y: Double,
            val canHold: Boolean,
            val rtl: Boolean,
        ) : HoldEvent

        /** It moved, in window coordinates. */
        data class Move(override val atMs: Long, val x: Double, val y: Double) : HoldEvent

        /** It lifted; [inside] is the button's own hit test. Its position counts as a last move. */
        data class Up(
            override val atMs: Long,
            val x: Double,
            val y: Double,
            val inside: Boolean,
            val situation: Situation,
            val recordedMs: Long,
            /** Some peak since the recording started rose above the silence level. */
            val heard: Boolean,
        ) : HoldEvent

        /** The system cancelled the touch; [background]: the app has gone there. */
        data class SystemCancel(
            override val atMs: Long,
            val background: Boolean,
            val recordedMs: Long,
        ) : HoldEvent

        /** Time passed: at H, and whenever the port likes. */
        data class Tick(override val atMs: Long, val situation: Situation) : HoldEvent

        /** The recorder stopped itself at the five-minute limit. */
        data class Cap(override val atMs: Long) : HoldEvent

        /** Anything but the person stopped it (S4). */
        data class Interruption(override val atMs: Long, val recordedMs: Long) : HoldEvent

        /** The slot was activated (not the microphone's own touch). */
        data class Activate(
            override val atMs: Long,
            val situation: Situation,
            val recordedMs: Long,
        ) : HoldEvent

        /** "Record voice message" from a menu, or the shortcut; during a recording it STOPS it. */
        data class Record(
            override val atMs: Long,
            val besideDraft: Boolean,
            val situation: Situation,
            val recordedMs: Long,
        ) : HoldEvent

        /** Stop, Back, "Stop and listen first". Never a start. */
        data class Stop(override val atMs: Long, val recordedMs: Long) : HoldEvent

        /** The recording row's Delete. */
        data class Delete(override val atMs: Long, val recordedMs: Long) : HoldEvent

        /** "Delete this recording?" answered: Delete, or Keep. */
        data class Answer(override val atMs: Long, val delete: Boolean) : HoldEvent

        /** The system's microphone prompt answered. */
        data class PermissionAnswer(override val atMs: Long, val granted: Boolean) : HoldEvent

        /** The Undo row's Undo. */
        data class Undo(override val atMs: Long) : HoldEvent

        /**
         * The person changed the composer — typed, deleted, pasted, staged or
         * took something off — or took any other action: it ends the Undo
         * window early, and outside a recording lifts the activation guard.
         */
        data class OtherAction(override val atMs: Long) : HoldEvent

        /** The slot's own Send or Save just emptied the composer. */
        data class Emptied(override val atMs: Long) : HoldEvent
    }

    /** S2.9's haptics, phones only. */
    enum class Haptic {
        /** Recording starts from a tap: `ToggleOn`. */
        LIGHT,

        /** Recording starts at H: `LongPress`. */
        MEDIUM,

        /** Lock; cancel armed: `GestureThresholdActivate`. */
        SELECTION,

        /** Sent: `Confirm`. */
        SUCCESS,

        /** Too short; deleted: `Reject`. */
        WARNING,
    }

    /** A line the composer SHOWS — in the row or its notice line. */
    enum class Hint(val text: String) {
        STILL_RECORDING("Still recording. Tap Send when you're done."),
        NEXT_TIME_SENDS("Next time, letting go will send it."),
        NOTHING_HEARD("We didn't hear anything."),
        STOPPED_AT_FIVE_MINUTES("Recording stopped at five minutes."),
        TOO_SHORT("That recording was too short."),
        CAN_RECORD_NOW("You can record now."),
    }

    /** What is SPOKEN, politely, to a screen reader — state changes only. */
    sealed interface Announcement {
        /** The catalogue key; `%@` is [ReadyToReview]'s length. */
        val text: String

        data object Recording : Announcement {
            override val text = "Recording"
        }

        data object RecordingLocked : Announcement {
            override val text = "Recording locked"
        }

        data object RecordingDeleted : Announcement {
            override val text = "Recording deleted"
        }

        data object VoiceMessageSent : Announcement {
            override val text = "Voice message sent"
        }

        /** "Ready to review, 0:42". */
        data class ReadyToReview(val recordedMs: Long) : Announcement {
            override val text = "Ready to review, %@"
        }

        data object TooShort : Announcement {
            override val text = "That recording was too short."
        }

        data object StoppedAtFiveMinutes : Announcement {
            override val text = "Recording stopped at five minutes."
        }
    }

    /** What a port does, in the order given: the thing, then what is shown, said and felt. */
    sealed interface HoldEffect {
        /** Open the microphone: the hold row when [held], otherwise the recording row. */
        data class Start(val held: Boolean) : HoldEffect

        /** Hands-free from here; the finger's later lift does nothing. */
        data object Lock : HoldEffect

        /** Cancel armed: "Release to cancel". */
        data object Arm : HoldEffect

        /** "‹ Slide to cancel" again. */
        data object Disarm : HoldEffect

        /** Stop the recording if it runs, and delete it. */
        data object Delete : HoldEffect

        /** Stop it, and hand it to the outbox now, with the primed reply. */
        data object Send : HoldEffect

        /** Stop it, and stage it in the chip above the field (S2.7). */
        data object Review : HoldEffect

        /** Stop it, and keep it as the chat's "Voice message not sent" row (S2.8). */
        data object Park : HoldEffect

        /** Stop it, write it to the parked store marked "sending", show the Undo row. */
        data object UndoWindow : HoldEffect

        /** The note in its Undo window goes to the outbox now. */
        data object UndoSend : HoldEffect

        /** The note in its Undo window goes to review instead; nothing is sent. */
        data object UndoReview : HoldEffect

        /** Stop it, and ask "Delete this recording?" [Delete] [Keep]. */
        data object AskDelete : HoldEffect

        /** Raise the system's microphone prompt. */
        data object AskPermission : HoldEffect

        /** The denial notice, with Open Settings where the platform can open it. */
        data object Denied : HoldEffect

        /** Say the dimmed row's sentence instead of acting. */
        data class Explain(val reason: ComposerSlot.Dimmed) : HoldEffect

        data class Hint(val hint: RecordGesture.Hint) : HoldEffect

        data class Announce(val announcement: Announcement) : HoldEffect

        data class Haptic(val haptic: RecordGesture.Haptic) : HoldEffect

        /** Remember, on this device, that its first held release has been taught. */
        data object FirstReleaseDone : HoldEffect
    }

    /** One step: the state and an event in, the next state and what to do out. */
    fun holdStep(
        state: HoldState,
        event: HoldEvent,
        constants: HoldConstants,
    ): Pair<HoldState, List<HoldEffect>> {
        val step = Step(state, constants)
        step.run(event)
        return step.state to step.effects
    }

    /** The reference's `let mut s` and `let mut fx`, and its helpers over them. */
    private class Step(var state: HoldState, private val c: HoldConstants) {
        val effects = mutableListOf<HoldEffect>()

        fun run(event: HoldEvent) {
            val at = event.atMs

            // The Undo window runs out on its own clock: a late timer must
            // not keep a note waiting, nor let a late Undo take it back.
            val note = state.undo
            if (note != null && at >= note.untilMs) undoSend()

            when (event) {
                is HoldEvent.Down ->
                    if (state.phase == Phase.Idle && !state.guarded(at)) {
                        state = state.copy(
                            phase = Phase.Pressed(
                                downAtMs = at,
                                downX = event.x,
                                downY = event.y,
                                rtl = event.rtl,
                                mayHold = event.canHold,
                            ),
                        )
                    }

                is HoldEvent.Move -> when (val phase = state.phase) {
                    is Phase.Pressed -> if (phase.mayHold) {
                        val dx = event.x - phase.downX
                        val dy = event.y - phase.downY
                        if (dx * dx + dy * dy > c.tapSlop * c.tapSlop) {
                            state = state.copy(phase = phase.copy(mayHold = false))
                        }
                    }
                    is Phase.Holding -> slide(event.x, event.y)
                    else -> Unit
                }

                is HoldEvent.Up -> when (state.phase) {
                    is Phase.Pressed ->
                        if (event.inside) {
                            activateMicrophone(at, event.situation, Source.TAP, besideDraft = false)
                        } else {
                            state = state.copy(phase = Phase.Idle)
                        }
                    is Phase.Holding -> {
                        slide(event.x, event.y)
                        val after = state.phase
                        if (after is Phase.Holding) {
                            state = state.copy(guardUntilMs = saturatingAdd(at, c.activationGuardMs))
                            if (after.armed) {
                                state = state.copy(phase = Phase.Idle)
                                deleted()
                            } else {
                                release(at, event.situation, event.recordedMs, event.heard)
                            }
                        }
                    }
                    else -> Unit
                }

                is HoldEvent.SystemCancel -> when (val phase = state.phase) {
                    is Phase.Pressed -> state = state.copy(phase = Phase.Idle)
                    is Phase.Holding -> when {
                        phase.armed -> {
                            state = state.copy(phase = Phase.Idle)
                            deleted()
                        }
                        event.background -> {
                            state = state.copy(phase = Phase.Idle)
                            interrupted(event.recordedMs)
                        }
                        else -> {
                            state = state.copy(phase = Phase.HandsFree(besideDraft = false))
                            effects += HoldEffect.Lock
                            effects += HoldEffect.Announce(Announcement.RecordingLocked)
                        }
                    }
                    else -> Unit
                }

                is HoldEvent.Tick -> {
                    val phase = state.phase
                    if (phase is Phase.Pressed && phase.mayHold &&
                        saturatingSub(at, phase.downAtMs) >= c.holdThresholdMs
                    ) {
                        sendWaiting()
                        when (val refused = refusal(event.situation)) {
                            HoldEffect.AskPermission -> {
                                state = state.copy(
                                    phase = Phase.AwaitingPermission(Source.HOLD, besideDraft = false),
                                )
                                effects += HoldEffect.AskPermission
                            }
                            null -> {
                                state = state.copy(
                                    phase = Phase.Holding(phase.downX, phase.downY, phase.rtl, armed = false),
                                    guardUntilMs = saturatingAdd(at, c.activationGuardMs),
                                )
                                effects += HoldEffect.Start(held = true)
                                effects += HoldEffect.Announce(Announcement.Recording)
                                effects += HoldEffect.Haptic(Haptic.MEDIUM)
                            }
                            else -> {
                                state = state.copy(phase = Phase.Idle)
                                effects += refused
                            }
                        }
                    }
                }

                is HoldEvent.Cap ->
                    if (recording()) {
                        state = state.copy(phase = Phase.Idle)
                        effects += HoldEffect.Review
                        effects += HoldEffect.Hint(Hint.STOPPED_AT_FIVE_MINUTES)
                        effects += HoldEffect.Announce(Announcement.StoppedAtFiveMinutes)
                    }

                is HoldEvent.Interruption -> {
                    if (state.undo != null) undoSend()
                    when (state.phase) {
                        is Phase.Holding, is Phase.HandsFree -> {
                            state = state.copy(phase = Phase.Idle)
                            interrupted(event.recordedMs)
                        }
                        // The question's answer never came: kept, never lost
                        // and never sent, and the question goes with it.
                        is Phase.AskingDelete -> {
                            state = state.copy(phase = Phase.Idle)
                            effects += HoldEffect.Park
                        }
                        is Phase.Pressed, is Phase.AwaitingPermission -> state = state.copy(phase = Phase.Idle)
                        Phase.Idle -> Unit
                    }
                }

                is HoldEvent.Activate ->
                    if (!state.guarded(at)) {
                        when (val phase = state.phase) {
                            Phase.Idle -> activateMicrophone(at, event.situation, Source.TAP, besideDraft = false)
                            is Phase.HandsFree -> {
                                state = state.copy(
                                    phase = Phase.Idle,
                                    guardUntilMs = saturatingAdd(at, c.activationGuardMs),
                                )
                                if (phase.besideDraft) {
                                    stopIntoReview(event.recordedMs)
                                } else {
                                    sendNow(event.recordedMs)
                                }
                            }
                            else -> Unit
                        }
                    }

                is HoldEvent.Record -> when (state.phase) {
                    Phase.Idle -> activateMicrophone(at, event.situation, Source.MENU, event.besideDraft)
                    is Phase.Holding, is Phase.HandsFree -> {
                        state = state.copy(phase = Phase.Idle)
                        stopIntoReview(event.recordedMs)
                    }
                    else -> Unit
                }

                is HoldEvent.Stop ->
                    if (recording()) {
                        state = state.copy(phase = Phase.Idle)
                        stopIntoReview(event.recordedMs)
                    }

                is HoldEvent.Delete ->
                    if (recording()) {
                        if (event.recordedMs < c.deleteAsksFromMs) {
                            state = state.copy(phase = Phase.Idle)
                            deleted()
                        } else {
                            state = state.copy(phase = Phase.AskingDelete(event.recordedMs))
                            effects += HoldEffect.AskDelete
                        }
                    }

                is HoldEvent.Answer -> {
                    val phase = state.phase
                    if (phase is Phase.AskingDelete) {
                        state = state.copy(phase = Phase.Idle)
                        if (event.delete) {
                            deleted()
                        } else {
                            effects += HoldEffect.Review
                            effects += HoldEffect.Announce(Announcement.ReadyToReview(phase.recordedMs))
                        }
                    }
                }

                is HoldEvent.PermissionAnswer -> {
                    val phase = state.phase
                    if (phase is Phase.AwaitingPermission) {
                        state = state.copy(phase = Phase.Idle)
                        when {
                            !event.granted -> effects += HoldEffect.Denied
                            phase.source == Source.HOLD -> effects += HoldEffect.Hint(Hint.CAN_RECORD_NOW)
                            else -> startHandsFree(at, phase.source, phase.besideDraft)
                        }
                    }
                }

                is HoldEvent.Undo -> {
                    val waiting = state.undo
                    if (waiting != null) {
                        state = state.copy(undo = null)
                        effects += HoldEffect.UndoReview
                        effects += HoldEffect.Announce(Announcement.ReadyToReview(waiting.recordedMs))
                    }
                }

                is HoldEvent.OtherAction -> {
                    if (state.undo != null) undoSend()
                    // The person changed the composer: the next press is a
                    // decision of its own, not the second half of a double
                    // tap. While a recording runs the box is behind the row
                    // and nothing in it is the person's to change, so the
                    // guard that keeps a double tap on the microphone from
                    // sending stays.
                    if (state.phase == Phase.Idle) state = state.copy(guardUntilMs = 0)
                }

                is HoldEvent.Emptied -> {
                    // A text Send is an action like any other: a released note
                    // still waiting goes first.
                    if (state.undo != null) undoSend()
                    state = state.copy(guardUntilMs = saturatingAdd(at, c.activationGuardMs))
                }
            }
        }

        /** A voice recording runs. */
        private fun recording(): Boolean = state.phase is Phase.Holding || state.phase is Phase.HandsFree

        /** What stands between the microphone's activation and a recording, in this order. */
        private fun refusal(situation: Situation): HoldEffect? {
            situation.blocked?.let { return HoldEffect.Explain(it) }
            return when (situation.permission) {
                Permission.GRANTED -> null
                Permission.DENIED -> HoldEffect.Denied
                Permission.NOT_ASKED -> HoldEffect.AskPermission
            }
        }

        /** The microphone's completed activation from a tap or a menu (H has its own path). */
        private fun activateMicrophone(at: Long, situation: Situation, source: Source, besideDraft: Boolean) {
            sendWaiting()
            when (val refused = refusal(situation)) {
                HoldEffect.AskPermission -> {
                    state = state.copy(phase = Phase.AwaitingPermission(source, besideDraft))
                    effects += HoldEffect.AskPermission
                }
                null -> startHandsFree(at, source, besideDraft)
                else -> {
                    state = state.copy(phase = Phase.Idle)
                    effects += refused
                }
            }
        }

        /** A hands-free recording starts. Only the slot's OWN activation guards it. */
        private fun startHandsFree(at: Long, source: Source, besideDraft: Boolean) {
            state = state.copy(phase = Phase.HandsFree(besideDraft))
            if (source == Source.TAP) {
                state = state.copy(guardUntilMs = saturatingAdd(at, c.activationGuardMs))
            }
            effects += HoldEffect.Start(held = false)
            effects += HoldEffect.Announce(Announcement.Recording)
            effects += HoldEffect.Haptic(Haptic.LIGHT)
        }

        /** A held finger moved (or lifted): arm or disarm cancel, then lock — never while armed. */
        private fun slide(x: Double, y: Double) {
            val phase = state.phase as? Phase.Holding ?: return
            val towardLeading = if (phase.rtl) x - phase.downX else phase.downX - x
            val up = phase.downY - y
            var nowArmed = phase.armed
            if (!phase.armed && towardLeading >= c.cancelArmDistance) {
                nowArmed = true
                effects += HoldEffect.Arm
                effects += HoldEffect.Haptic(Haptic.SELECTION)
            } else if (phase.armed && towardLeading < c.cancelDisarmDistance) {
                nowArmed = false
                effects += HoldEffect.Disarm
            }
            if (!nowArmed && up >= c.lockDistance) {
                state = state.copy(phase = Phase.HandsFree(besideDraft = false))
                effects += HoldEffect.Lock
                effects += HoldEffect.Announce(Announcement.RecordingLocked)
                effects += HoldEffect.Haptic(Haptic.SELECTION)
            } else {
                state = state.copy(phase = phase.copy(armed = nowArmed))
            }
        }

        /** A hold let go with cancel not armed (S2.3), in the plan's order. */
        private fun release(at: Long, situation: Situation, recordedMs: Long, heard: Boolean) {
            if (recordedMs < c.shortestRecordingMs) {
                state = state.copy(phase = Phase.HandsFree(besideDraft = false))
                effects += HoldEffect.Lock
                effects += HoldEffect.Hint(Hint.STILL_RECORDING)
                return
            }
            state = state.copy(phase = Phase.Idle)
            if (!heard) {
                effects += HoldEffect.Review
                effects += HoldEffect.Hint(Hint.NOTHING_HEARD)
                effects += HoldEffect.Announce(Announcement.ReadyToReview(recordedMs))
                return
            }
            if (situation.firstRelease || situation.reviewBeforeSending || situation.assistive) {
                // "Next time, letting go will send it" only when it is true.
                val teach = situation.firstRelease && !situation.reviewBeforeSending && !situation.assistive
                effects += HoldEffect.Review
                if (teach) effects += HoldEffect.Hint(Hint.NEXT_TIME_SENDS)
                effects += HoldEffect.Announce(Announcement.ReadyToReview(recordedMs))
                if (teach) effects += HoldEffect.FirstReleaseDone
                return
            }
            state = state.copy(undo = UndoNote(untilMs = saturatingAdd(at, c.undoWindowMs), recordedMs = recordedMs))
            effects += HoldEffect.UndoWindow
        }

        /** The person deleted it: said and felt. */
        private fun deleted() {
            effects += HoldEffect.Delete
            effects += HoldEffect.Announce(Announcement.RecordingDeleted)
            effects += HoldEffect.Haptic(Haptic.WARNING)
        }

        /** Under a second: discarded, with the sentence shown and said. */
        private fun tooShort() {
            effects += HoldEffect.Delete
            effects += HoldEffect.Hint(Hint.TOO_SHORT)
            effects += HoldEffect.Announce(Announcement.TooShort)
            effects += HoldEffect.Haptic(Haptic.WARNING)
        }

        /** The Send arrow (S2.5). */
        private fun sendNow(recordedMs: Long) {
            if (recordedMs < c.shortestRecordingMs) {
                tooShort()
            } else {
                effects += HoldEffect.Send
                effects += HoldEffect.Announce(Announcement.VoiceMessageSent)
                effects += HoldEffect.Haptic(Haptic.SUCCESS)
            }
        }

        /** Stop, the Stop square, the shortcut (S2.5). */
        private fun stopIntoReview(recordedMs: Long) {
            if (recordedMs < c.shortestRecordingMs) {
                tooShort()
            } else {
                effects += HoldEffect.Review
                effects += HoldEffect.Announce(Announcement.ReadyToReview(recordedMs))
            }
        }

        /** Stopped by something other than the person: not sent, or deleted without a word. */
        private fun interrupted(recordedMs: Long) {
            effects += if (recordedMs < c.shortestRecordingMs) HoldEffect.Delete else HoldEffect.Park
        }

        /** The microphone's activation ends the Undo window by sending. */
        private fun sendWaiting() {
            if (state.undo != null) undoSend()
        }

        private fun undoSend() {
            state = state.copy(undo = null)
            effects += HoldEffect.UndoSend
            effects += HoldEffect.Announce(Announcement.VoiceMessageSent)
            effects += HoldEffect.Haptic(Haptic.SUCCESS)
        }
    }
}
