/*
 * MediaPlanVectorsTest.kt
 * Family Connect (Android)
 *
 * AN ORACLE, NOT FOUR READINGS. docs/protocol.md, "Preparing media before
 * upload", is exact arithmetic because four codebases have to reach the
 * same answer for the same file — and four people reading the same
 * paragraph is how iOS and Android came to disagree about 1080p and 720p in
 * the first place. So the planner is not tested against what this file's
 * author thinks the protocol says: every case below was PRINTED by the
 * reference (`fc_text::media_plan`, via `cargo run -- media-plan` in
 * win/tools/board-oracle), and MediaPlan has to give the same answer to
 * every one, to the bit.
 *
 * The file is identical to the iOS and Windows copies; CI `cmp`s all three
 * against a fresh print, so a hand edit to this one fails there. Parsed, not
 * compared as text: its keys come out sorted, and a frame rate is written
 * as an integer when it is whole.
 *
 * Plain JUnit, no Robolectric: MediaPlan touches nothing from android.*.
 *
 * iOS counterpart: FamilyConnectTests/MediaPlanVectorTests.swift
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.double
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import me.nettrash.familyconnect.data.repo.MediaPlan.AudioPlan
import me.nettrash.familyconnect.data.repo.MediaPlan.VideoPlan
import org.junit.Test

class MediaPlanVectorsTest {

    private val cases: List<JsonObject> = run {
        val stream = requireNotNull(javaClass.classLoader?.getResourceAsStream(VECTORS)) {
            "$VECTORS is missing from the test classpath (app/src/test/resources)"
        }
        val text = stream.use { it.readBytes().toString(Charsets.UTF_8) }
        Json.parseToJsonElement(text).jsonArray.map { it.jsonObject }
    }

    /**
     * Every case, every field. Mismatches are COLLECTED rather than thrown at
     * the first, so a port that disagrees sees every case it disagrees on —
     * and the name of each, which says which rule it is about.
     */
    @Test
    fun `the planner answers every vector exactly as the reference does`() {
        val mismatches = mutableListOf<String>()
        for (case in cases) {
            val name = case.string("name")
            val input = case.getValue("input").jsonObject
            val expected = case.getValue("expected").jsonObject
            val actual = answer(case.string("function"), input)
            if (actual != normalised(expected)) {
                mismatches += "$name\n    expected $expected\n    actual   $actual"
            }
        }
        assertWithMessage(mismatches.joinToString("\n"))
            .that(mismatches)
            .isEmpty()
    }

    /**
     * The count is the reference's, so a truncated or half-copied file does
     * not pass by having fewer cases to fail — and every function the
     * reference prints is one this port answers.
     */
    @Test
    fun `every case is read and every function is known`() {
        assertThat(cases).hasSize(EXPECTED_CASES)
        assertThat(cases.map { it.string("function") }.toSet()).containsExactly(
            "target_size", "target_frame_rate", "profile_video_bitrate", "target_video_bitrate",
            "target_audio_bitrate", "estimated_bitrate", "plan_video", "plan_audio", "sendable",
            "on_failure", "keep_smaller",
        )
    }

    /** The case's answer from MediaPlan, shaped exactly like the file's `expected`. */
    private fun answer(function: String, input: JsonObject): Map<String, Any?> = when (function) {
        "target_size" -> {
            val (width, height) = MediaPlan.targetSize(input.long("width"), input.long("height"))
            mapOf("width" to width, "height" to height)
        }
        "target_frame_rate" ->
            mapOf("frame_rate" to MediaPlan.targetFrameRate(input.doubleOrNull("frame_rate")))
        "profile_video_bitrate" -> mapOf(
            "bitrate" to MediaPlan.profileVideoBitrate(
                input.long("width"),
                input.long("height"),
                input.double("frame_rate"),
            ),
        )
        "target_video_bitrate" -> mapOf(
            "bitrate" to MediaPlan.targetVideoBitrate(
                input.long("width"),
                input.long("height"),
                input.double("frame_rate"),
                input.longOrNull("source_bitrate"),
            ),
        )
        "target_audio_bitrate" -> mapOf(
            "bitrate" to MediaPlan.targetAudioBitrate(
                input.longOrNull("channels"),
                input.longOrNull("source_bitrate"),
            ),
        )
        "estimated_bitrate" -> mapOf(
            "bitrate" to MediaPlan.estimatedBitrate(
                input.long("size_bytes"),
                input.longOrNull("duration_ms"),
                input.long("audio_bitrate"),
            ),
        )
        "plan_video" -> {
            val source = videoSource(input)
            val target = MediaPlan.videoTarget(source)
            val plan = when (val planned = MediaPlan.planVideo(source)) {
                VideoPlan.Keep -> "keep"
                is VideoPlan.Transcode -> {
                    // What the reference asserts while printing: a transcode is TO the target.
                    check(planned.target == target) { "a transcode is to the target" }
                    "transcode"
                }
                VideoPlan.Fallback -> "fallback"
            }
            mapOf(
                "plan" to plan,
                "within_profile" to MediaPlan.withinProfile(source),
                "source_video_bitrate" to MediaPlan.sourceVideoBitrate(source),
                "target" to target?.let {
                    mapOf(
                        "width" to it.width,
                        "height" to it.height,
                        "frame_rate" to it.frameRate,
                        "video_bitrate" to it.videoBitrate,
                        "audio_bitrate" to it.audioBitrate,
                    )
                },
            )
        }
        "plan_audio" -> {
            val source = MediaPlan.AudioSource(
                container = input.string("container"),
                codec = input.string("codec"),
                channels = input.longOrNull("channels"),
                bitrate = input.longOrNull("bitrate"),
                sizeBytes = input.long("size_bytes"),
                durationMs = input.longOrNull("duration_ms"),
            )
            val bitrate = MediaPlan.sourceAudioBitrate(source)
            val target = MediaPlan.targetAudioBitrate(source.channels, bitrate)
            val plan = when (val planned = MediaPlan.planAudio(source)) {
                AudioPlan.Keep -> "keep"
                is AudioPlan.Transcode -> {
                    check(planned.bitrate == target) { "a transcode is to the target" }
                    "transcode"
                }
            }
            mapOf("plan" to plan, "source_bitrate" to bitrate, "target_bitrate" to target)
        }
        "sendable" -> mapOf(
            "sendable" to MediaPlan.sendable(
                input.string("kind"),
                input.string("container"),
                input.getValue("honest").jsonPrimitive.boolean,
                input.long("size_bytes"),
                input.long("ceiling_bytes"),
            ),
        )
        "on_failure" -> mapOf(
            "send" to when (MediaPlan.onFailure(input.getValue("source_sendable").jsonPrimitive.boolean)) {
                MediaPlan.OnFailure.ORIGINAL -> "original"
                MediaPlan.OnFailure.TODAYS_PATH -> "todays_path"
            },
        )
        "keep_smaller" -> mapOf(
            "upload" to when (
                MediaPlan.keepSmaller(
                    input.long("source_bytes"),
                    input.getValue("source_sendable").jsonPrimitive.boolean,
                    input.long("result_bytes"),
                )
            ) {
                MediaPlan.Upload.SOURCE -> "source"
                MediaPlan.Upload.RESULT -> "result"
            },
        )
        else -> throw AssertionError("a function this port does not know: $function")
    }

    private fun videoSource(input: JsonObject) = MediaPlan.VideoSource(
        width = input.long("width"),
        height = input.long("height"),
        frameRate = input.doubleOrNull("frame_rate"),
        container = input.string("container"),
        videoCodec = input.string("video_codec"),
        audioCodec = input.stringOrNull("audio_codec"),
        audioChannels = input.longOrNull("audio_channels"),
        videoBitrate = input.longOrNull("video_bitrate"),
        audioBitrate = input.longOrNull("audio_bitrate"),
        sizeBytes = input.long("size_bytes"),
        durationMs = input.longOrNull("duration_ms"),
    )

    /**
     * The file's `expected` as plain Kotlin values, so it compares with
     * [answer] by `==`. Every number is read as a Long unless it has a
     * fraction or is a frame rate — a whole frame rate is written `30`, and
     * 30L is not 30.0.
     */
    private fun normalised(element: JsonElement, key: String? = null): Any? = when {
        element is JsonNull -> null
        element is JsonObject -> element.mapValues { (name, value) -> normalised(value, name) }
        element.jsonPrimitive.isString -> element.jsonPrimitive.content
        element.jsonPrimitive.content == "true" || element.jsonPrimitive.content == "false" ->
            element.jsonPrimitive.boolean
        key == "frame_rate" -> element.jsonPrimitive.double
        else -> element.jsonPrimitive.long
    }

    private fun JsonObject.string(key: String): String = getValue(key).jsonPrimitive.content
    private fun JsonObject.stringOrNull(key: String): String? =
        get(key)?.takeUnless { it is JsonNull }?.jsonPrimitive?.content
    private fun JsonObject.long(key: String): Long = getValue(key).jsonPrimitive.long
    private fun JsonObject.longOrNull(key: String): Long? =
        get(key)?.takeUnless { it is JsonNull }?.jsonPrimitive?.long
    private fun JsonObject.double(key: String): Double = getValue(key).jsonPrimitive.double
    private fun JsonObject.doubleOrNull(key: String): Double? =
        get(key)?.takeUnless { it is JsonNull }?.jsonPrimitive?.double

    private companion object {
        const val VECTORS = "media-plan-vectors.json"

        /** What `cargo run -- media-plan` printed on 2026-09-28. */
        const val EXPECTED_CASES = 464
    }
}
