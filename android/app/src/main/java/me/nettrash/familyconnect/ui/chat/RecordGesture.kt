/*
 * RecordGesture.kt
 * Family Connect (Android)
 *
 * What an activation of the slot, the menu or the shortcut, Stop, Delete, the
 * length limit or an interruption does to a voice recording in the Send slot
 * (#79, docs/audio-video-messages-2026-10-04.md, S2.1, S2.2, S2.5) — as a
 * reducer: a state and an event go in, the next state and what to do come
 * out.
 *
 * A PORT of `fc_text::record::hold_step` (web/text/src/record.rs), branch for
 * branch and under the same names, held to the reference by
 * RecordGestureVectorsTest over every `hold_step` case in
 * record-vectors.json — so a tap on this phone does what a tap on the iPhone
 * does. Do not "improve" a branch here: change the reference, print the
 * vectors again, and port the change.
 *
 * REVISED 2026-10-06: THERE IS NO HOLD. The first draft made the microphone a
 * walkie-talkie on touch (hold to talk, slide to cancel or to lock, let go to
 * send after a five-second Undo window); the owner removed it after testing
 * it. The microphone does ONE thing: its activation starts a hands-free
 * recording, which the slot's Send arrow sends, Stop keeps for review and
 * Delete deletes. A long press is not a gesture: nothing records and nothing
 * opens while a finger is down, and the press is an ordinary tap when it
 * lifts inside, however long it was held. The names (`holdStep`, HoldState,
 * HoldEvent, HoldEffect, HoldConstants) are the reference's, kept.
 *
 * It is only the DECISION. ChatViewModel feeds it — the slot's activation
 * from RecordSendButton, the recorder's own endings, the interruptions — and
 * carries out what comes back: opening the microphone, staging, parking,
 * sending, the haptics, the words shown and spoken.
 *
 * The contract the port keeps (the reference's module notes):
 *  - TWO CLOCKS. `atMs` is one monotonic clock for the guard. `recordedMs` is
 *    the recorder's own clock — the one the person sees, which with a screen
 *    reader starts once "Recording" has been spoken.
 *  - Every activation of the slot — a tap, a click, Enter, TalkBack's — is
 *    Activate. Nothing is sent for a press going down or being held.
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

object RecordGesture {

    /** The numbers [holdStep] decides by: S1.1's. */
    data class HoldConstants(
        val shortestRecordingMs: Long = ComposerSlot.SHORTEST_RECORDING_MS,
        val activationGuardMs: Long = ComposerSlot.ACTIVATION_GUARD_MS,
        val deleteAsksFromMs: Long = ComposerSlot.DELETE_ASKS_FROM_MS,
    )

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
     * granted microphone and nothing in the way.
     */
    data class Situation(
        val permission: Permission = Permission.GRANTED,
        /** The dimmed row the microphone is in (rows 7–9), if any. */
        val blocked: ComposerSlot.Dimmed? = null,
    )

    /** Where a recording that waits on the permission prompt came from; either way Allow records. */
    enum class Source {
        /** The slot's microphone: a tap, a click, Enter, TalkBack's activation. Its start is guarded. */
        TAP,

        /** "Record voice message" from the paperclip or the menu, or the shortcut. Never guarded. */
        MENU,
    }

    /** Where the slot's voice recording is. */
    sealed interface Phase {
        /** Nothing recording. */
        data object Idle : Phase

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

    /** The reducer's whole state: the phase and the activation guard. */
    data class HoldState(
        val phase: Phase = Phase.Idle,
        /** Activation of the slot before this moment is ignored whole. 0: none. */
        val guardUntilMs: Long = 0,
    ) {
        /** What [ComposerSlot.composerSlot] is told about this recording. */
        val recording: ComposerSlot.Recording
            get() = when (val phase = phase) {
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

        /** The recorder stopped itself at the five-minute limit. */
        data class Cap(override val atMs: Long) : HoldEvent

        /** Anything but the person stopped it (S4). */
        data class Interruption(override val atMs: Long, val recordedMs: Long) : HoldEvent

        /**
         * The slot was activated — the microphone, the Send arrow, the Stop
         * square — by a completed tap (however long it was held), a click,
         * Enter, or TalkBack.
         */
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

        /**
         * The person changed the composer — typed, deleted, pasted, staged or
         * took something off — or took any other action: outside a recording
         * it lifts the activation guard.
         */
        data class OtherAction(override val atMs: Long) : HoldEvent

        /** The slot's own Send or Save just emptied the composer. */
        data class Emptied(override val atMs: Long) : HoldEvent
    }

    /** S2.9's haptics, phones only. */
    enum class Haptic {
        /** Recording starts: `ToggleOn`. */
        LIGHT,

        /** Sent: `Confirm`. */
        SUCCESS,

        /** Too short; deleted: `Reject`. */
        WARNING,
    }

    /** A line the composer SHOWS — in the row or its notice line. */
    enum class Hint(val text: String) {
        STOPPED_AT_FIVE_MINUTES("Recording stopped at five minutes."),
        TOO_SHORT("That recording was too short."),
    }

    /** What is SPOKEN, politely, to a screen reader — state changes only. */
    sealed interface Announcement {
        /** The catalogue key; `%@` is [ReadyToReview]'s length. */
        val text: String

        data object Recording : Announcement {
            override val text = "Recording"
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
        /** Open the microphone and record: the recording row. */
        data object Start : HoldEffect

        /** Stop the recording if it runs, and delete it. */
        data object Delete : HoldEffect

        /** Stop it, and hand it to the outbox now, with the primed reply. */
        data object Send : HoldEffect

        /** Stop it, and stage it in the chip above the field (S2.7). */
        data object Review : HoldEffect

        /** Stop it, and keep it as the chat's "Voice message not sent" row (S2.8). */
        data object Park : HoldEffect

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
            when (event) {
                is HoldEvent.Cap ->
                    if (recording()) {
                        state = state.copy(phase = Phase.Idle)
                        effects += HoldEffect.Review
                        effects += HoldEffect.Hint(Hint.STOPPED_AT_FIVE_MINUTES)
                        effects += HoldEffect.Announce(Announcement.StoppedAtFiveMinutes)
                    }

                is HoldEvent.Interruption -> when (state.phase) {
                    is Phase.HandsFree -> {
                        state = state.copy(phase = Phase.Idle)
                        interrupted(event.recordedMs)
                    }
                    // The question's answer never came: kept, never lost and
                    // never sent, and the question goes with it.
                    is Phase.AskingDelete -> {
                        state = state.copy(phase = Phase.Idle)
                        effects += HoldEffect.Park
                    }
                    is Phase.AwaitingPermission -> state = state.copy(phase = Phase.Idle)
                    Phase.Idle -> Unit
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
                            is Phase.AskingDelete, is Phase.AwaitingPermission -> Unit
                        }
                    }

                is HoldEvent.Record -> when (state.phase) {
                    Phase.Idle -> activateMicrophone(at, event.situation, Source.MENU, event.besideDraft)
                    is Phase.HandsFree -> {
                        state = state.copy(phase = Phase.Idle)
                        stopIntoReview(event.recordedMs)
                    }
                    is Phase.AskingDelete, is Phase.AwaitingPermission -> Unit
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
                        if (event.granted) {
                            startHandsFree(at, phase.source, phase.besideDraft)
                        } else {
                            effects += HoldEffect.Denied
                        }
                    }
                }

                is HoldEvent.OtherAction ->
                    // The person changed the composer: the next activation is
                    // a decision of its own, not the second half of a double
                    // tap. While a recording runs the box is behind the row
                    // and nothing in it is the person's to change, so the
                    // guard that keeps a double tap on the microphone from
                    // sending stays.
                    if (state.phase == Phase.Idle) state = state.copy(guardUntilMs = 0)

                is HoldEvent.Emptied ->
                    state = state.copy(guardUntilMs = saturatingAdd(at, c.activationGuardMs))
            }
        }

        /** A voice recording runs. */
        private fun recording(): Boolean = state.phase is Phase.HandsFree

        /** What stands between the microphone's activation and a recording, in this order. */
        private fun refusal(situation: Situation): HoldEffect? {
            situation.blocked?.let { return HoldEffect.Explain(it) }
            return when (situation.permission) {
                Permission.GRANTED -> null
                Permission.DENIED -> HoldEffect.Denied
                Permission.NOT_ASKED -> HoldEffect.AskPermission
            }
        }

        /** The microphone's completed activation, from the slot or a menu. */
        private fun activateMicrophone(at: Long, situation: Situation, source: Source, besideDraft: Boolean) {
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
            effects += HoldEffect.Start
            effects += HoldEffect.Announce(Announcement.Recording)
            effects += HoldEffect.Haptic(Haptic.LIGHT)
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
    }
}
