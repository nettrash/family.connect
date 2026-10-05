/*
 * ComposerSlotVectorsTest.kt
 * Family Connect (Android)
 *
 * ComposerSlot against the shared rules' own vectors (#79,
 * docs/audio-video-messages-2026-10-04.md, "Shared rules"): every
 * `constants`, `hold_threshold_ms`, `composer_slot`, `video_door`,
 * `round_cap_ms`, `round_warning_ms`, `round_diameter` and `is_round` case in
 * record-vectors.json, which `win/tools/board-oracle` prints from the
 * reference (`fc_text::record`) and CI keeps identical across the ports. The
 * hold reducer's cases are RecordGestureVectorsTest's.
 *
 * Strict both ways: a field this port does not know, or a function the file
 * gained, fails here — so the reference cannot move on without the port
 * noticing.
 *
 * Plain JUnit: ComposerSlot touches nothing from android.*.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.double
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import me.nettrash.familyconnect.ui.chat.ComposerSlot.Dimmed
import me.nettrash.familyconnect.ui.chat.ComposerSlot.Door
import me.nettrash.familyconnect.ui.chat.ComposerSlot.Recording
import me.nettrash.familyconnect.ui.chat.ComposerSlot.Slot
import org.junit.Test

class ComposerSlotVectorsTest {

    private val cases: List<JsonObject> = RecordVectors.cases

    private fun casesOf(function: String): List<JsonObject> =
        cases.filter { it.getValue("function").jsonPrimitive.content == function }
            .also { assertWithMessage("no $function cases").that(it).isNotEmpty() }

    private fun JsonObject.str(key: String): String = getValue(key).jsonPrimitive.content
    private fun JsonObject.bool(key: String): Boolean = getValue(key).jsonPrimitive.boolean
    private fun JsonObject.long(key: String): Long = getValue(key).jsonPrimitive.long
    private fun JsonObject.obj(key: String): JsonObject = getValue(key).jsonObject

    /** Null for JSON null, else the string. */
    private fun JsonObject.optStr(key: String): String? =
        getValue(key).let { if (it is JsonNull) null else it.jsonPrimitive.content }

    @Test
    fun everyFunctionInTheFileIsOneAPortReads() {
        val known = setOf(
            "constants", "hold_threshold_ms", "composer_slot", "video_door", "round_cap_ms",
            "round_warning_ms", "round_diameter", "is_round", "hold_step",
        )
        val functions = cases.map { it.str("function") }.toSet()
        assertThat(functions).isEqualTo(known)
        // The printed file's own size, so a truncated copy cannot pass quietly.
        assertThat(cases).hasSize(398)
    }

    @Test
    fun theConstantsAreTheReferencesOwn() {
        val expected = casesOf("constants").single().obj("expected")
        val longs = mapOf(
            "activation_guard_ms" to ComposerSlot.ACTIVATION_GUARD_MS,
            "min_hold_threshold_ms" to ComposerSlot.MIN_HOLD_THRESHOLD_MS,
            "shortest_recording_ms" to ComposerSlot.SHORTEST_RECORDING_MS,
            "undo_window_ms" to ComposerSlot.UNDO_WINDOW_MS,
            "voice_cap_ms" to ComposerSlot.VOICE_CAP_MS,
            "voice_warning_ms" to ComposerSlot.VOICE_WARNING_MS,
            "default_max_round_video_ms" to ComposerSlot.DEFAULT_MAX_ROUND_VIDEO_MS,
            "round_cap_margin_ms" to ComposerSlot.ROUND_CAP_MARGIN_MS,
            "round_warning_lead_ms" to ComposerSlot.ROUND_WARNING_LEAD_MS,
            "silence_max_amplitude" to ComposerSlot.SILENCE_MAX_AMPLITUDE.toLong(),
            "silence_warning_after_ms" to ComposerSlot.SILENCE_WARNING_AFTER_MS,
            "delete_asks_from_ms" to ComposerSlot.DELETE_ASKS_FROM_MS,
            "still_recording_hint_ms" to ComposerSlot.STILL_RECORDING_HINT_MS,
            "preview_idle_close_ms" to ComposerSlot.PREVIEW_IDLE_CLOSE_MS,
            "slot_crossfade_ms" to ComposerSlot.SLOT_CROSSFADE_MS,
            "recorder_fade_ms" to ComposerSlot.RECORDER_FADE_MS,
            "min_target_apple_pt" to ComposerSlot.MIN_TARGET_APPLE_PT.toLong(),
            "min_target_android_dp" to ComposerSlot.MIN_TARGET_ANDROID_DP.toLong(),
            "min_target_windows_epx" to ComposerSlot.MIN_TARGET_WINDOWS_EPX.toLong(),
            "min_target_web_px" to ComposerSlot.MIN_TARGET_WEB_PX.toLong(),
            "round_diameter_compact" to ComposerSlot.ROUND_DIAMETER_COMPACT.toLong(),
            "round_diameter_regular" to ComposerSlot.ROUND_DIAMETER_REGULAR.toLong(),
        )
        val doubles = mapOf(
            "tap_slop" to ComposerSlot.TAP_SLOP,
            "lock_distance" to ComposerSlot.LOCK_DISTANCE,
            "cancel_arm_distance" to ComposerSlot.CANCEL_ARM_DISTANCE,
            "cancel_disarm_distance" to ComposerSlot.CANCEL_DISARM_DISTANCE,
            "silence_peak_dbfs" to ComposerSlot.SILENCE_PEAK_DBFS,
            "silence_sample_magnitude" to ComposerSlot.SILENCE_SAMPLE_MAGNITUDE,
        )
        val strings = mapOf(
            "video_door_label" to ComposerSlot.VIDEO_DOOR_LABEL,
            "video_door_tooltip" to ComposerSlot.VIDEO_DOOR_TOOLTIP,
        )
        // Every constant the reference prints is one this port carries.
        assertThat(expected.keys)
            .isEqualTo(longs.keys + doubles.keys + strings.keys + "default_hold_constants")
        for ((key, value) in longs) assertWithMessage(key).that(expected.long(key)).isEqualTo(value)
        for ((key, value) in doubles) {
            assertWithMessage(key).that(expected.getValue(key).jsonPrimitive.double).isEqualTo(value)
        }
        for ((key, value) in strings) assertWithMessage(key).that(expected.str(key)).isEqualTo(value)
        assertThat(RecordVectors.constants(expected.obj("default_hold_constants")))
            .isEqualTo(RecordGesture.HoldConstants())
    }

    @Test
    fun hTheHoldThresholdNeverFallsBelowTheFloor() {
        for (case in casesOf("hold_threshold_ms")) {
            val system = case.obj("input").long("system_long_press_ms")
            assertWithMessage(case.str("name"))
                .that(ComposerSlot.holdThresholdMs(system))
                .isEqualTo(case.obj("expected").long("hold_threshold_ms"))
            assertWithMessage(case.str("name"))
                .that(RecordGesture.HoldConstants.forSystem(system).holdThresholdMs)
                .isEqualTo(case.obj("expected").long("hold_threshold_ms"))
        }
    }

    private fun slotInputs(input: JsonObject): ComposerSlot.SlotInputs {
        assertThat(input.keys).isEqualTo(
            setOf(
                "recorder_open", "recording", "editing", "draft_blank", "staged",
                "assistant_chat", "can_record", "call", "busy", "not_sent",
            ),
        )
        return ComposerSlot.SlotInputs(
            recorderOpen = input.bool("recorder_open"),
            recording = when (val name = input.str("recording")) {
                "none" -> Recording.NONE
                "held" -> Recording.HELD
                "hands_free" -> Recording.HANDS_FREE
                "hands_free_beside_draft" -> Recording.HANDS_FREE_BESIDE_DRAFT
                else -> error("recording $name")
            },
            editing = input.bool("editing"),
            draftBlank = input.bool("draft_blank"),
            staged = input.bool("staged"),
            assistantChat = input.bool("assistant_chat"),
            canRecord = input.bool("can_record"),
            call = input.bool("call"),
            busy = input.bool("busy"),
            notSent = input.bool("not_sent"),
        )
    }

    private fun Dimmed.wire(): String = when (this) {
        Dimmed.CALL -> "call"
        Dimmed.BUSY -> "busy"
        Dimmed.NOT_SENT -> "not_sent"
    }

    @Test
    fun theSlotIsTheFirstMatchingRowEveryTime() {
        for (case in casesOf("composer_slot")) {
            val name = case.str("name")
            val slot = ComposerSlot.composerSlot(slotInputs(case.obj("input")))
            val expected = case.obj("expected")
            assertThat(expected.keys).isEqualTo(setOf("row", "slot", "enabled", "reason", "label", "notice"))
            val wireName = when (slot) {
                Slot.Recorder -> "recorder"
                Slot.HeldMicrophone -> "held_microphone"
                Slot.SendVoice -> "send_voice"
                Slot.StopRecording -> "stop_recording"
                is Slot.Save -> "save"
                Slot.Send -> "send"
                Slot.SendDisabled -> "send_disabled"
                is Slot.Dimmed -> "dimmed"
                Slot.Microphone -> "microphone"
            }
            assertWithMessage("$name: slot").that(wireName).isEqualTo(expected.str("slot"))
            assertWithMessage("$name: row").that(slot.row).isEqualTo(expected.getValue("row").jsonPrimitive.int)
            assertWithMessage("$name: enabled")
                .that((slot as? Slot.Save)?.enabled)
                .isEqualTo(expected.getValue("enabled").let { if (it is JsonNull) null else it.jsonPrimitive.boolean })
            assertWithMessage("$name: reason")
                .that((slot as? Slot.Dimmed)?.reason?.wire())
                .isEqualTo(expected.optStr("reason"))
            assertWithMessage("$name: label").that(slot.label).isEqualTo(expected.optStr("label"))
            assertWithMessage("$name: notice").that(slot.notice).isEqualTo(expected.optStr("notice"))
            assertWithMessage("$name: is a microphone")
                .that(slot.isMicrophone)
                .isEqualTo(slot.row in 7..10)
        }
    }

    @Test
    fun theVideoButtonIsHiddenDimmedOrShownAsTheReferenceSays() {
        for (case in casesOf("video_door")) {
            val name = case.str("name")
            val input = case.obj("input")
            assertThat(input.keys).isEqualTo(
                setOf(
                    "slot", "family_or_direct_chat", "undo_window", "server_offers_round",
                    "has_camera", "encoder_probe_passes", "records_round_video",
                ),
            )
            val door = ComposerSlot.videoDoor(
                ComposerSlot.DoorInputs(
                    slot = slotInputs(input.obj("slot")),
                    familyOrDirectChat = input.bool("family_or_direct_chat"),
                    undoWindow = input.bool("undo_window"),
                    serverOffersRound = input.bool("server_offers_round"),
                    hasCamera = input.bool("has_camera"),
                    encoderProbePasses = input.bool("encoder_probe_passes"),
                    recordsRoundVideo = input.bool("records_round_video"),
                ),
            )
            val expected = case.obj("expected")
            assertThat(expected.keys).isEqualTo(setOf("door", "reason", "label", "notice"))
            val wireName = when (door) {
                Door.Hidden -> "hidden"
                is Door.Dimmed -> "dimmed"
                Door.Shown -> "shown"
            }
            assertWithMessage("$name: door").that(wireName).isEqualTo(expected.str("door"))
            assertWithMessage("$name: reason")
                .that((door as? Door.Dimmed)?.reason?.wire())
                .isEqualTo(expected.optStr("reason"))
            assertWithMessage("$name: label").that(door.label).isEqualTo(expected.optStr("label"))
            assertWithMessage("$name: notice").that(door.notice).isEqualTo(expected.optStr("notice"))
        }
    }

    /**
     * Since Phase 3 this build records round video (Decision 40), so the door
     * opens where the server and the device allow — and a build that did
     * not would still keep it shut, whatever the rest says.
     */
    @Test
    fun theVideoEntryFollowsWhetherThisBuildRecordsRoundVideo() {
        assertThat(ComposerSlot.RECORDS_ROUND_VIDEO).isTrue()
        val microphone = ComposerSlot.SlotInputs(
            recorderOpen = false, recording = Recording.NONE, editing = false, draftBlank = true,
            staged = false, assistantChat = false, canRecord = true, call = false, busy = false,
            notSent = false,
        )
        fun door(records: Boolean) = ComposerSlot.videoDoor(
            ComposerSlot.DoorInputs(
                slot = microphone, familyOrDirectChat = true, undoWindow = false,
                serverOffersRound = true, hasCamera = true, encoderProbePasses = true,
                recordsRoundVideo = records,
            ),
        )
        assertThat(door(ComposerSlot.RECORDS_ROUND_VIDEO)).isEqualTo(Door.Shown)
        assertThat(door(false)).isEqualTo(Door.Hidden)
    }

    @Test
    fun theRoundVideosArithmetic() {
        for (case in casesOf("round_cap_ms")) {
            val max = case.obj("input").long("max_round_video_ms")
            assertWithMessage(case.str("name"))
                .that(ComposerSlot.roundCapMs(max))
                .isEqualTo(case.obj("expected").long("cap_ms"))
        }
        for (case in casesOf("round_warning_ms")) {
            val max = case.obj("input").long("max_round_video_ms")
            assertWithMessage(case.str("name"))
                .that(ComposerSlot.roundWarningMs(max))
                .isEqualTo(case.obj("expected").long("warning_ms"))
        }
        for (case in casesOf("round_diameter")) {
            val width = when (val name = case.obj("input").str("width_class")) {
                "compact" -> ComposerSlot.WidthClass.COMPACT
                "regular" -> ComposerSlot.WidthClass.REGULAR
                else -> error("width $name")
            }
            assertWithMessage(case.str("name"))
                .that(ComposerSlot.roundDiameter(width))
                .isEqualTo(case.obj("expected").getValue("diameter").jsonPrimitive.int)
        }
    }

    @Test
    fun aMessageIsRoundExactlyWhenTheReferenceSaysSo() {
        for (case in casesOf("is_round")) {
            val input = case.obj("input")
            val attachments = input.getValue("attachments").jsonArray.map { element ->
                val attachment = element.jsonObject
                ComposerSlot.AttachmentFlags(kind = attachment.str("kind"), round = attachment.bool("round"))
            }
            assertWithMessage(case.str("name"))
                .that(ComposerSlot.isRound(input.str("body"), attachments))
                .isEqualTo(case.obj("expected").bool("round"))
        }
    }

    /**
     * The THREAD's test — [roundOf], which draws the circle, quotes it and
     * withholds Edit — over the same cases, through a stored row: the vectors
     * pin what the thread does, not only the helper beside it.
     */
    @Test
    fun theThreadDrawsACircleExactlyWhenTheReferenceSaysSo() {
        for (case in casesOf("is_round")) {
            val input = case.obj("input")
            val attachments = input.getValue("attachments").jsonArray.mapIndexed { index, element ->
                val attachment = element.jsonObject
                AttachmentDto(
                    id = index + 1L,
                    kind = attachment.str("kind"),
                    mime = "video/mp4",
                    size = 1,
                    width = 480,
                    height = 480,
                    round = if (attachment.bool("round")) true else null,
                )
            }
            val row = MessageEntity(
                clientMsgId = "vector",
                serverId = 1,
                chatId = 42,
                senderId = 9,
                body = input.str("body"),
                createdAt = 1_700_000_000_000,
                status = MessageStatus.SENT,
                attachmentsJson = AttachmentsCodec.encode(attachments).takeIf { attachments.isNotEmpty() },
            )
            assertWithMessage(case.str("name"))
                .that(roundOf(row) != null)
                .isEqualTo(case.obj("expected").bool("round"))
        }
    }
}

/** record-vectors.json, read once, and the decoders both vector tests share. */
internal object RecordVectors {

    private const val FILE = "record-vectors.json"

    val cases: List<JsonObject> by lazy {
        val stream = requireNotNull(javaClass.classLoader?.getResourceAsStream(FILE)) {
            "$FILE is missing from the test classpath (app/src/test/resources)"
        }
        val text = stream.use { it.readBytes().toString(Charsets.UTF_8) }
        Json.parseToJsonElement(text).jsonArray.map { it.jsonObject }
    }

    fun constants(o: JsonObject): RecordGesture.HoldConstants {
        check(
            o.keys == setOf(
                "hold_threshold_ms", "tap_slop", "lock_distance", "cancel_arm_distance",
                "cancel_disarm_distance", "shortest_recording_ms", "undo_window_ms",
                "activation_guard_ms", "delete_asks_from_ms",
            ),
        ) { "constants keys ${o.keys}" }
        fun l(key: String) = o.getValue(key).jsonPrimitive.long
        fun d(key: String) = o.getValue(key).jsonPrimitive.double
        return RecordGesture.HoldConstants(
            holdThresholdMs = l("hold_threshold_ms"),
            tapSlop = d("tap_slop"),
            lockDistance = d("lock_distance"),
            cancelArmDistance = d("cancel_arm_distance"),
            cancelDisarmDistance = d("cancel_disarm_distance"),
            shortestRecordingMs = l("shortest_recording_ms"),
            undoWindowMs = l("undo_window_ms"),
            activationGuardMs = l("activation_guard_ms"),
            deleteAsksFromMs = l("delete_asks_from_ms"),
        )
    }

    fun isNull(element: JsonElement?): Boolean = element == null || element is JsonNull
}
