/*
 * RecordGestureVectorsTest.kt
 * Family Connect (Android)
 *
 * The recording's reducer against every `hold_step` case the reference
 * printed (#79, docs/audio-video-messages-2026-10-04.md, S2.1, S2.2, S2.5 —
 * revised 2026-10-06: there is no hold, and the vectors were printed again
 * without it): each case
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
import org.junit.Test

class RecordGestureVectorsTest {

    private val cases = RecordVectors.cases.filter {
        it.getValue("function").jsonPrimitive.content == "hold_step"
    }

    private fun JsonObject.str(key: String): String = getValue(key).jsonPrimitive.content
    private fun JsonObject.bool(key: String): Boolean = getValue(key).jsonPrimitive.boolean
    private fun JsonObject.long(key: String): Long = getValue(key).jsonPrimitive.long
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
                o.expectKeys("phase", "guard_until_ms")
                Phase.Idle
            }
            "hands_free" -> {
                o.expectKeys("phase", "guard_until_ms", "beside_draft")
                Phase.HandsFree(o.bool("beside_draft"))
            }
            "asking_delete" -> {
                o.expectKeys("phase", "guard_until_ms", "recorded_ms")
                Phase.AskingDelete(o.long("recorded_ms"))
            }
            "awaiting_permission" -> {
                o.expectKeys("phase", "guard_until_ms", "source", "beside_draft")
                val source = when (val s = o.str("source")) {
                    "tap" -> Source.TAP
                    "menu" -> Source.MENU
                    else -> error("source $s")
                }
                Phase.AwaitingPermission(source, o.bool("beside_draft"))
            }
            else -> error("phase $name")
        }
        return HoldState(phase = phase, guardUntilMs = o.long("guard_until_ms"))
    }

    private fun situation(o: JsonObject): Situation {
        o.expectKeys("permission", "blocked")
        return Situation(
            permission = when (val p = o.str("permission")) {
                "granted" -> Permission.GRANTED
                "not_asked" -> Permission.NOT_ASKED
                "denied" -> Permission.DENIED
                else -> error("permission $p")
            },
            blocked = o["blocked"].takeUnless(RecordVectors::isNull)?.jsonPrimitive?.content?.let(::dimmed),
        )
    }

    private fun event(o: JsonObject): HoldEvent {
        val at = o.long("at_ms")
        return when (val name = o.str("event")) {
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
        "explain" -> {
            o.expectKeys("effect", "reason", "text")
            val reason = dimmed(o.str("reason"))
            assertWithMessage("$name: explain text").that(reason.notice).isEqualTo(o.str("text"))
            HoldEffect.Explain(reason)
        }
        "hint" -> {
            o.expectKeys("effect", "hint", "text")
            val hint = when (val h = o.str("hint")) {
                "stopped_at_five_minutes" -> Hint.STOPPED_AT_FIVE_MINUTES
                "too_short" -> Hint.TOO_SHORT
                else -> error("hint $h")
            }
            assertWithMessage("$name: hint text").that(hint.text).isEqualTo(o.str("text"))
            HoldEffect.Hint(hint)
        }
        "announce" -> {
            val announcement = when (val a = o.str("announcement")) {
                "recording" -> Announcement.Recording
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
                    "success" -> Haptic.SUCCESS
                    "warning" -> Haptic.WARNING
                    else -> error("haptic $h")
                },
            )
        }
        else -> {
            o.expectKeys("effect")
            when (kind) {
                "start" -> HoldEffect.Start
                "delete" -> HoldEffect.Delete
                "send" -> HoldEffect.Send
                "review" -> HoldEffect.Review
                "park" -> HoldEffect.Park
                "ask_delete" -> HoldEffect.AskDelete
                "ask_permission" -> HoldEffect.AskPermission
                "denied" -> HoldEffect.Denied
                else -> error("effect $kind")
            }
        }
    }

    private fun ComposerSlot.Recording.wire(): String = when (this) {
        ComposerSlot.Recording.NONE -> "none"
        ComposerSlot.Recording.HANDS_FREE -> "hands_free"
        ComposerSlot.Recording.HANDS_FREE_BESIDE_DRAFT -> "hands_free_beside_draft"
    }

    @Test
    fun everyHoldStepCaseIsThisPortsStepToo() {
        // The reference's own count, printed again 2026-10-06 without the hold.
        assertThat(cases).hasSize(91)
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
            "cap", "interruption", "activate", "record", "stop", "delete", "answer",
            "permission_answer", "other_action", "emptied",
        )
        val effects = cases.flatMap { case ->
            case.obj("expected").getValue("effects").jsonArray.map { it.jsonObject.str("effect") }
        }.toSet()
        assertThat(effects).containsExactly(
            "start", "delete", "send", "review", "park", "ask_delete", "ask_permission", "denied",
            "explain", "hint", "announce", "haptic",
        )
    }

    /**
     * Nothing the hold had is left in what the reference printed: no press,
     * move, lift, tick or Undo event, no lock, arm, Undo window or lesson, no
     * `undo` in a state — the port and the vectors are the same revision.
     */
    @Test
    fun theVectorsCarryNothingOfTheHold() {
        val text = cases.joinToString("\n") { it.toString() }
        for (gone in listOf(
            "\"down\"", "\"up\"", "\"move\"", "\"tick\"", "\"system_cancel\"", "\"undo\"",
            "\"lock\"", "\"arm\"", "\"undo_window\"", "\"first_release_done\"", "\"held\"",
            "\"holding\"", "\"pressed\"", "first_release", "review_before_sending", "assistive",
        )) {
            assertWithMessage("vectors still say $gone").that(text).doesNotContain(gone)
        }
    }
}
