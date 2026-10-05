/*
 * RecordGestureVectorsTest.kt
 * Family Connect (Android)
 *
 * The hold reducer against every `hold_step` case the reference printed (#79,
 * docs/audio-video-messages-2026-10-04.md, S2.1, S2.3, S2.5, S2.6): each case
 * is ONE step — a state, an event and the constants in; the next state, the
 * effects in order and what the slot is told out — taken from a named
 * scenario run through `fc_text::record::hold_step`, so this port meets every
 * transition in a state it can really be in, and checks it without replaying
 * anything.
 *
 * The words each effect carries (a hint's line, an announcement's key, a
 * dimmed row's sentence) are checked too: they are what RecordStrings maps to
 * this app's string resources.
 *
 * Plain JUnit: RecordGesture touches nothing from android.*.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.double
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import me.nettrash.familyconnect.ui.chat.RecordGesture.Announcement
import me.nettrash.familyconnect.ui.chat.RecordGesture.Haptic
import me.nettrash.familyconnect.ui.chat.RecordGesture.Hint
import me.nettrash.familyconnect.ui.chat.RecordGesture.HoldEffect
import me.nettrash.familyconnect.ui.chat.RecordGesture.HoldEvent
import me.nettrash.familyconnect.ui.chat.RecordGesture.HoldState
import me.nettrash.familyconnect.ui.chat.RecordGesture.Permission
import me.nettrash.familyconnect.ui.chat.RecordGesture.Phase
import me.nettrash.familyconnect.ui.chat.RecordGesture.Situation
import me.nettrash.familyconnect.ui.chat.RecordGesture.Source
import me.nettrash.familyconnect.ui.chat.RecordGesture.UndoNote
import org.junit.Test

class RecordGestureVectorsTest {

    private val cases = RecordVectors.cases.filter {
        it.getValue("function").jsonPrimitive.content == "hold_step"
    }

    private fun JsonObject.str(key: String): String = getValue(key).jsonPrimitive.content
    private fun JsonObject.bool(key: String): Boolean = getValue(key).jsonPrimitive.boolean
    private fun JsonObject.long(key: String): Long = getValue(key).jsonPrimitive.long
    private fun JsonObject.double(key: String): Double = getValue(key).jsonPrimitive.double
    private fun JsonObject.obj(key: String): JsonObject = getValue(key).jsonObject

    private fun JsonObject.expectKeys(vararg keys: String) {
        assertWithMessage("keys of $this").that(this.keys).isEqualTo(keys.toSet())
    }

    private fun dimmed(name: String): ComposerSlot.Dimmed = when (name) {
        "call" -> ComposerSlot.Dimmed.CALL
        "busy" -> ComposerSlot.Dimmed.BUSY
        "not_sent" -> ComposerSlot.Dimmed.NOT_SENT
        else -> error("dimmed $name")
    }

    private fun state(o: JsonObject): HoldState {
        val phase = when (val name = o.str("phase")) {
            "idle" -> {
                o.expectKeys("phase", "guard_until_ms", "undo")
                Phase.Idle
            }
            "pressed" -> {
                o.expectKeys("phase", "guard_until_ms", "undo", "down_at_ms", "down_x", "down_y", "rtl", "may_hold")
                Phase.Pressed(o.long("down_at_ms"), o.double("down_x"), o.double("down_y"), o.bool("rtl"), o.bool("may_hold"))
            }
            "holding" -> {
                o.expectKeys("phase", "guard_until_ms", "undo", "down_x", "down_y", "rtl", "armed")
                Phase.Holding(o.double("down_x"), o.double("down_y"), o.bool("rtl"), o.bool("armed"))
            }
            "hands_free" -> {
                o.expectKeys("phase", "guard_until_ms", "undo", "beside_draft")
                Phase.HandsFree(o.bool("beside_draft"))
            }
            "asking_delete" -> {
                o.expectKeys("phase", "guard_until_ms", "undo", "recorded_ms")
                Phase.AskingDelete(o.long("recorded_ms"))
            }
            "awaiting_permission" -> {
                o.expectKeys("phase", "guard_until_ms", "undo", "source", "beside_draft")
                val source = when (val s = o.str("source")) {
                    "tap" -> Source.TAP
                    "hold" -> Source.HOLD
                    "menu" -> Source.MENU
                    else -> error("source $s")
                }
                Phase.AwaitingPermission(source, o.bool("beside_draft"))
            }
            else -> error("phase $name")
        }
        val undo = o["undo"].takeUnless(RecordVectors::isNull)?.jsonObject?.let { note ->
            note.expectKeys("until_ms", "recorded_ms")
            UndoNote(untilMs = note.long("until_ms"), recordedMs = note.long("recorded_ms"))
        }
        return HoldState(phase = phase, guardUntilMs = o.long("guard_until_ms"), undo = undo)
    }

    private fun situation(o: JsonObject): Situation {
        o.expectKeys("permission", "blocked", "assistive", "first_release", "review_before_sending")
        return Situation(
            permission = when (val p = o.str("permission")) {
                "granted" -> Permission.GRANTED
                "not_asked" -> Permission.NOT_ASKED
                "denied" -> Permission.DENIED
                else -> error("permission $p")
            },
            blocked = o["blocked"].takeUnless(RecordVectors::isNull)?.jsonPrimitive?.content?.let(::dimmed),
            assistive = o.bool("assistive"),
            firstRelease = o.bool("first_release"),
            reviewBeforeSending = o.bool("review_before_sending"),
        )
    }

    private fun event(o: JsonObject): HoldEvent {
        val at = o.long("at_ms")
        return when (val name = o.str("event")) {
            "down" -> {
                o.expectKeys("event", "at_ms", "x", "y", "can_hold", "rtl")
                HoldEvent.Down(at, o.double("x"), o.double("y"), o.bool("can_hold"), o.bool("rtl"))
            }
            "move" -> {
                o.expectKeys("event", "at_ms", "x", "y")
                HoldEvent.Move(at, o.double("x"), o.double("y"))
            }
            "up" -> {
                o.expectKeys("event", "at_ms", "x", "y", "inside", "situation", "recorded_ms", "heard")
                HoldEvent.Up(
                    at, o.double("x"), o.double("y"), o.bool("inside"), situation(o.obj("situation")),
                    o.long("recorded_ms"), o.bool("heard"),
                )
            }
            "system_cancel" -> {
                o.expectKeys("event", "at_ms", "background", "recorded_ms")
                HoldEvent.SystemCancel(at, o.bool("background"), o.long("recorded_ms"))
            }
            "tick" -> {
                o.expectKeys("event", "at_ms", "situation")
                HoldEvent.Tick(at, situation(o.obj("situation")))
            }
            "cap" -> {
                o.expectKeys("event", "at_ms")
                HoldEvent.Cap(at)
            }
            "interruption" -> {
                o.expectKeys("event", "at_ms", "recorded_ms")
                HoldEvent.Interruption(at, o.long("recorded_ms"))
            }
            "activate" -> {
                o.expectKeys("event", "at_ms", "situation", "recorded_ms")
                HoldEvent.Activate(at, situation(o.obj("situation")), o.long("recorded_ms"))
            }
            "record" -> {
                o.expectKeys("event", "at_ms", "beside_draft", "situation", "recorded_ms")
                HoldEvent.Record(at, o.bool("beside_draft"), situation(o.obj("situation")), o.long("recorded_ms"))
            }
            "stop" -> {
                o.expectKeys("event", "at_ms", "recorded_ms")
                HoldEvent.Stop(at, o.long("recorded_ms"))
            }
            "delete" -> {
                o.expectKeys("event", "at_ms", "recorded_ms")
                HoldEvent.Delete(at, o.long("recorded_ms"))
            }
            "answer" -> {
                o.expectKeys("event", "at_ms", "delete")
                HoldEvent.Answer(at, o.bool("delete"))
            }
            "permission_answer" -> {
                o.expectKeys("event", "at_ms", "granted")
                HoldEvent.PermissionAnswer(at, o.bool("granted"))
            }
            "undo" -> {
                o.expectKeys("event", "at_ms")
                HoldEvent.Undo(at)
            }
            "other_action" -> {
                o.expectKeys("event", "at_ms")
                HoldEvent.OtherAction(at)
            }
            "emptied" -> {
                o.expectKeys("event", "at_ms")
                HoldEvent.Emptied(at)
            }
            else -> error("event $name")
        }
    }

    /** One effect as the reference printed it — and the words it carries checked against this port's. */
    private fun effect(o: JsonObject, name: String): HoldEffect = when (val kind = o.str("effect")) {
        "start" -> {
            o.expectKeys("effect", "held")
            HoldEffect.Start(o.bool("held"))
        }
        "explain" -> {
            o.expectKeys("effect", "reason", "text")
            val reason = dimmed(o.str("reason"))
            assertWithMessage("$name: explain text").that(reason.notice).isEqualTo(o.str("text"))
            HoldEffect.Explain(reason)
        }
        "hint" -> {
            o.expectKeys("effect", "hint", "text")
            val hint = when (val h = o.str("hint")) {
                "still_recording" -> Hint.STILL_RECORDING
                "next_time_sends" -> Hint.NEXT_TIME_SENDS
                "nothing_heard" -> Hint.NOTHING_HEARD
                "stopped_at_five_minutes" -> Hint.STOPPED_AT_FIVE_MINUTES
                "too_short" -> Hint.TOO_SHORT
                "can_record_now" -> Hint.CAN_RECORD_NOW
                else -> error("hint $h")
            }
            assertWithMessage("$name: hint text").that(hint.text).isEqualTo(o.str("text"))
            HoldEffect.Hint(hint)
        }
        "announce" -> {
            val announcement = when (val a = o.str("announcement")) {
                "recording" -> Announcement.Recording
                "recording_locked" -> Announcement.RecordingLocked
                "recording_deleted" -> Announcement.RecordingDeleted
                "voice_message_sent" -> Announcement.VoiceMessageSent
                "ready_to_review" -> Announcement.ReadyToReview(o.long("recorded_ms"))
                "too_short" -> Announcement.TooShort
                "stopped_at_five_minutes" -> Announcement.StoppedAtFiveMinutes
                else -> error("announcement $a")
            }
            if (announcement is Announcement.ReadyToReview) {
                o.expectKeys("effect", "announcement", "text", "recorded_ms")
            } else {
                o.expectKeys("effect", "announcement", "text")
            }
            assertWithMessage("$name: announcement text").that(announcement.text).isEqualTo(o.str("text"))
            HoldEffect.Announce(announcement)
        }
        "haptic" -> {
            o.expectKeys("effect", "haptic")
            HoldEffect.Haptic(
                when (val h = o.str("haptic")) {
                    "light" -> Haptic.LIGHT
                    "medium" -> Haptic.MEDIUM
                    "selection" -> Haptic.SELECTION
                    "success" -> Haptic.SUCCESS
                    "warning" -> Haptic.WARNING
                    else -> error("haptic $h")
                },
            )
        }
        else -> {
            o.expectKeys("effect")
            when (kind) {
                "lock" -> HoldEffect.Lock
                "arm" -> HoldEffect.Arm
                "disarm" -> HoldEffect.Disarm
                "delete" -> HoldEffect.Delete
                "send" -> HoldEffect.Send
                "review" -> HoldEffect.Review
                "park" -> HoldEffect.Park
                "undo_window" -> HoldEffect.UndoWindow
                "undo_send" -> HoldEffect.UndoSend
                "undo_review" -> HoldEffect.UndoReview
                "ask_delete" -> HoldEffect.AskDelete
                "ask_permission" -> HoldEffect.AskPermission
                "denied" -> HoldEffect.Denied
                "first_release_done" -> HoldEffect.FirstReleaseDone
                else -> error("effect $kind")
            }
        }
    }

    private fun ComposerSlot.Recording.wire(): String = when (this) {
        ComposerSlot.Recording.NONE -> "none"
        ComposerSlot.Recording.HELD -> "held"
        ComposerSlot.Recording.HANDS_FREE -> "hands_free"
        ComposerSlot.Recording.HANDS_FREE_BESIDE_DRAFT -> "hands_free_beside_draft"
    }

    @Test
    fun everyHoldStepCaseIsThisPortsStepToo() {
        // The reference's own count: 71 scenarios' steps and 24 out-of-place events.
        assertThat(cases).hasSize(262)
        for (case in cases) {
            val name = case.str("name")
            val input = case.obj("input")
            input.expectKeys("state", "event", "constants")
            val expected = case.obj("expected")
            expected.expectKeys("state", "effects", "recording")

            val (next, effects) = RecordGesture.holdStep(
                state = state(input.obj("state")),
                event = event(input.obj("event")),
                constants = RecordVectors.constants(input.obj("constants")),
            )

            assertWithMessage("$name: state").that(next).isEqualTo(state(expected.obj("state")))
            val expectedEffects = expected.getValue("effects").jsonArray.map { effect(it.jsonObject, name) }
            assertWithMessage("$name: effects").that(effects).isEqualTo(expectedEffects)
            assertWithMessage("$name: recording").that(next.recording.wire()).isEqualTo(expected.str("recording"))
        }
    }

    /** Every event and every effect the reducer knows is met by at least one case. */
    @Test
    fun theVectorsReachEveryEventAndEveryEffect() {
        val events = cases.map { it.obj("input").obj("event").str("event") }.toSet()
        assertThat(events).containsExactly(
            "down", "move", "up", "system_cancel", "tick", "cap", "interruption", "activate",
            "record", "stop", "delete", "answer", "permission_answer", "undo", "other_action", "emptied",
        )
        val effects = cases.flatMap { case ->
            case.obj("expected").getValue("effects").jsonArray.map { it.jsonObject.str("effect") }
        }.toSet()
        assertThat(effects).containsExactly(
            "start", "lock", "arm", "disarm", "delete", "send", "review", "park", "undo_window",
            "undo_send", "undo_review", "ask_delete", "ask_permission", "denied", "explain", "hint",
            "announce", "haptic", "first_release_done",
        )
    }
}
