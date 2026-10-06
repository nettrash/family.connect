/*
 * WaveformVectorsTest.kt
 * Family Connect (Android)
 *
 * A voice note's waveform (#79; docs/protocol.md, "A voice note's
 * waveform") is the SHARED rules' — `fc_text::waveform`, printed by
 * `cargo run -- waveform` in win/tools/board-oracle into
 * `waveform-vectors.json`, byte-identical in every port's fixtures. Every
 * case in that file is run against [Waveform] here: the constants, a level
 * per peak (ties, one-ulp neighbours, NaN and the infinities, which JSON
 * spells as strings), the slices, parse, the placeholder, the bars that fit,
 * their heights, and how many are played.
 *
 * Plus [PeakLog], the sender's half: the first read after a start is
 * dropped, and the rest become the wire string.
 *
 * Plain JUnit: Waveform touches nothing from android.*.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Test

class WaveformVectorsTest {

    private val cases: List<JsonObject> = run {
        val stream = requireNotNull(javaClass.classLoader?.getResourceAsStream(VECTORS)) {
            "$VECTORS is missing from the test classpath (app/src/test/resources)"
        }
        val text = stream.use { it.readBytes().toString(Charsets.UTF_8) }
        Json.parseToJsonElement(text).jsonArray.map { it.jsonObject }
    }

    private fun JsonObject.function() = getValue("function").jsonPrimitive.content
    private fun JsonObject.name() = getValue("name").jsonPrimitive.content
    private fun JsonObject.input() = getValue("input").jsonObject
    private fun JsonObject.expected() = getValue("expected").jsonObject

    /** A peak as the file spells it: a number, or "NaN" / "Infinity" / "-Infinity". */
    private fun peak(element: JsonElement): Double {
        val primitive = element.jsonPrimitive
        return when {
            !primitive.isString -> primitive.content.toDouble()
            primitive.content == "NaN" -> Double.NaN
            primitive.content == "Infinity" -> Double.POSITIVE_INFINITY
            primitive.content == "-Infinity" -> Double.NEGATIVE_INFINITY
            else -> error("unknown peak spelling ${primitive.content}")
        }
    }

    private fun ints(element: JsonElement): List<Int> = (element as JsonArray).map { it.jsonPrimitive.int }

    @Test
    fun `every case in the shared vectors holds`() {
        val ran = mutableMapOf<String, Int>()
        for (case in cases) {
            val input = case.input()
            val expected = case.expected()
            val why = case.name()
            when (case.function()) {
                "constants" -> {
                    assertThat(Waveform.LEVELS).isEqualTo(expected.getValue("levels").jsonPrimitive.int)
                    assertThat(Waveform.MAX_LEVEL).isEqualTo(expected.getValue("max_level").jsonPrimitive.int)
                    assertThat(Waveform.FLOOR_DBFS)
                        .isEqualTo(expected.getValue("floor_dbfs").jsonPrimitive.content.toDouble())
                    assertThat(Waveform.DB_PER_LEVEL)
                        .isEqualTo(expected.getValue("db_per_level").jsonPrimitive.content.toDouble())
                    assertThat(Waveform.PLACEHOLDER_LEVEL)
                        .isEqualTo(expected.getValue("placeholder_level").jsonPrimitive.int)
                    assertThat(Waveform.encode(Waveform.PLACEHOLDER))
                        .isEqualTo(expected.getValue("placeholder").jsonPrimitive.content)
                }
                "level" -> assertWithMessage(why).that(Waveform.level(peak(input.getValue("dbfs"))))
                    .isEqualTo(expected.getValue("level").jsonPrimitive.int)
                "from_peaks" -> assertWithMessage(why).that(
                    Waveform.fromPeaks(
                        input.getValue("samples_dbfs").jsonArray.map(::peak),
                        input.getValue("levels").jsonPrimitive.int,
                    ),
                ).isEqualTo(expected.getValue("waveform").jsonPrimitive.content)
                "parse" -> {
                    val want = expected.getValue("levels")
                    val got = Waveform.parse(input.getValue("waveform").jsonPrimitive.content)
                    if (want is JsonNull) assertWithMessage(why).that(got).isNull() else assertWithMessage(why).that(got).isEqualTo(ints(want))
                }
                "levels_or_placeholder" -> {
                    val raw = input.getValue("waveform")
                    val text = if (raw is JsonNull) null else raw.jsonPrimitive.content
                    assertWithMessage(why).that(Waveform.levelsOrPlaceholder(text)).isEqualTo(ints(expected.getValue("levels")))
                }
                "bars" -> assertWithMessage(why).that(
                    Waveform.bars(ints(input.getValue("levels")), input.getValue("count").jsonPrimitive.int),
                ).isEqualTo(ints(expected.getValue("bars")))
                "bar_fraction" -> assertWithMessage(why).that(Waveform.barFraction(input.getValue("level").jsonPrimitive.int))
                    .isEqualTo(expected.getValue("fraction").jsonPrimitive.content.toDouble())
                "played_bars" -> assertWithMessage(why).that(
                    Waveform.playedBars(
                        input.getValue("position_ms").jsonPrimitive.content.toULong(),
                        input.getValue("duration_ms").jsonPrimitive.content.toULong(),
                        input.getValue("bars").jsonPrimitive.int,
                    ),
                ).isEqualTo(expected.getValue("played").jsonPrimitive.int)
                else -> error("a function this port does not know: ${case.function()} ($why)")
            }
            ran.merge(case.function(), 1, Int::plus)
        }
        // Every function the reference prints, and every case of it.
        assertThat(ran.keys).containsExactly(
            "constants", "level", "from_peaks", "parse", "levels_or_placeholder", "bars", "bar_fraction", "played_bars",
        )
        assertThat(ran.values.sum()).isEqualTo(cases.size)
    }

    @Test
    fun `the log drops the first read, which is always zero, and sends the rest`() {
        val log = PeakLog()
        assertThat(log.waveform()).isNull()
        log.record(0) // MediaRecorder's first answer after a start
        assertThat(log.waveform()).isNull()
        log.record(Waveform.FULL_SCALE)
        log.record(0)
        // Two peaks stretch over 48 slices: full scale, then silence.
        assertThat(log.waveform()).isEqualTo("f".repeat(24) + "0".repeat(24))
        log.clear()
        assertThat(log.waveform()).isNull()
    }

    @Test
    fun `a peak of the sixteen-bit meter is the decibels the levels are cut at`() {
        // −30 dBFS is 1 036.2 of 32 767: level 7.5 rounds up to 8.
        assertThat(Waveform.level(Waveform.dbfs(1_037))).isEqualTo(8)
        assertThat(Waveform.level(Waveform.dbfs(0))).isEqualTo(0)
        assertThat(Waveform.level(Waveform.dbfs(Waveform.FULL_SCALE))).isEqualTo(15)
    }

    private companion object {
        const val VECTORS = "waveform-vectors.json"
    }
}
