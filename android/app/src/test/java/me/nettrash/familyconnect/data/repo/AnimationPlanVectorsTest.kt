/*
 * AnimationPlanVectorsTest.kt
 * Family Connect (Android)
 *
 * AN ORACLE, NOT A READING (issue #96; docs/protocol.md, "Animations"). The
 * clip an animated picture becomes is exact arithmetic because four encoders
 * have to make the same one, so every case below was PRINTED by the reference
 * (`fc_text::animation`, `cargo run -- animation-plan` in
 * win/tools/board-oracle), and AnimationPlan has to give the same answer to
 * every one: the constants, each format's delay, the shortest-frame rule, and
 * whole plans — samples, ticks, size, duration and bitrate.
 *
 * The file is identical to the iOS and Windows copies and CI compares them
 * with a fresh print, so a hand edit here fails there. Mismatches are
 * collected, not thrown at the first, so a port sees every case it gets
 * wrong by name.
 *
 * Plain JUnit: AnimationPlan touches nothing from android.*.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import org.junit.Test

class AnimationPlanVectorsTest {

    private val cases: List<JsonObject> = run {
        val stream = requireNotNull(javaClass.classLoader?.getResourceAsStream(VECTORS)) {
            "$VECTORS is missing from the test classpath (app/src/test/resources)"
        }
        val text = stream.use { it.readBytes().toString(Charsets.UTF_8) }
        Json.parseToJsonElement(text).jsonArray.map { it.jsonObject }
    }

    @Test
    fun `every vector is answered exactly as the reference answers it`() {
        val mismatches = mutableListOf<String>()
        for (case in cases) {
            val name = case.getValue("name").jsonPrimitive.content
            val input = case.getValue("input").jsonObject
            val expected = normalised(case.getValue("expected"))
            val actual = answer(case.getValue("function").jsonPrimitive.content, input)
            if (actual != expected) {
                mismatches += "$name\n    expected $expected\n    actual   $actual"
            }
        }
        assertWithMessage(mismatches.joinToString("\n")).that(mismatches).isEmpty()
    }

    /**
     * The count is the reference's, so a truncated file does not pass by
     * having fewer cases to fail — and every function it prints is one this
     * port answers.
     */
    @Test
    fun `every case is read and every function is known`() {
        assertThat(cases).hasSize(EXPECTED_CASES)
        assertThat(cases.map { it.getValue("function").jsonPrimitive.content }.toSet()).containsExactly(
            "constants", "gif_delay_ms", "apng_delay_ms", "frame_duration_ms", "plan",
        )
    }

    /** The case's answer from AnimationPlan, shaped like the file's `expected`. */
    private fun answer(function: String, input: JsonObject): Any? = when (function) {
        "constants" -> mapOf(
            "background_rgb" to AnimationPlan.BACKGROUND_RGB.map(Int::toLong),
            "default_frame_ms" to AnimationPlan.DEFAULT_FRAME_MS.toLong(),
            "shortest_frame_ms" to AnimationPlan.SHORTEST_FRAME_MS.toLong(),
            "ticks_per_second" to AnimationPlan.TICKS_PER_SECOND.toLong(),
        )
        "gif_delay_ms" -> mapOf("ms" to AnimationPlan.gifDelayMs(input.long("centiseconds").toInt()))
        "apng_delay_ms" -> mapOf(
            "ms" to AnimationPlan.apngDelayMs(input.long("num").toInt(), input.long("den").toInt()),
        )
        "frame_duration_ms" -> mapOf("ms" to AnimationPlan.frameDurationMs(input.long("stated_ms")))
        "plan" -> AnimationPlan.plan(
            input.long("width"),
            input.long("height"),
            input.getValue("stated_ms").jsonArray.map { it.jsonPrimitive.long },
        )?.let { plan ->
            mapOf(
                "width" to plan.width.toLong(),
                "height" to plan.height.toLong(),
                "samples" to plan.samples.map { listOf(it.frame.toLong(), it.ticks.toLong()) },
                "total_ticks" to plan.totalTicks.toLong(),
                "duration_ms" to plan.durationMs,
                "video_bitrate" to plan.videoBitrate,
            )
        }
        else -> throw AssertionError("a function this port does not know: $function")
    }

    /** The file's `expected` as plain Kotlin values: objects, lists and Longs. */
    private fun normalised(element: JsonElement): Any? = when (element) {
        is JsonNull -> null
        is JsonObject -> element.mapValues { (_, value) -> normalised(value) }
        is JsonArray -> element.map(::normalised)
        else -> element.jsonPrimitive.long
    }

    private fun JsonObject.long(key: String): Long = getValue(key).jsonPrimitive.long

    private companion object {
        const val VECTORS = "animation-plan-vectors.json"

        /** What `cargo run -- animation-plan` printed on 2026-10-19. */
        const val EXPECTED_CASES = 46
    }
}
