/*
 * RingbackToneTest.kt
 * Family Connect (Android)
 *
 * The cadence table and the synthesis, pinned: which country hears which
 * ring, that a cycle is exactly as long as its pattern says, that the
 * bursts sound and the gaps are silent, and that nothing clips. iOS pins
 * the same numbers (RingbackToneTests.swift) — the table is shared by
 * value.
 */

package me.nettrash.familyconnect.calls

import com.google.common.truth.Truth.assertThat
import kotlin.math.abs
import org.junit.Test

class RingbackToneTest {

    @Test
    fun theCountryPicksTheCadenceAndUnknownIsCept() {
        assertThat(RingbackTone.cadence("US")).isEqualTo(RingbackTone.Cadence.ANSI)
        assertThat(RingbackTone.cadence("CA")).isEqualTo(RingbackTone.Cadence.ANSI)
        assertThat(RingbackTone.cadence("PR")).isEqualTo(RingbackTone.Cadence.ANSI)
        assertThat(RingbackTone.cadence("GB")).isEqualTo(RingbackTone.Cadence.UK)
        assertThat(RingbackTone.cadence("AU")).isEqualTo(RingbackTone.Cadence.UK)
        assertThat(RingbackTone.cadence("JP")).isEqualTo(RingbackTone.Cadence.JAPAN)
        assertThat(RingbackTone.cadence("RS")).isEqualTo(RingbackTone.Cadence.CEPT)
        assertThat(RingbackTone.cadence("DE")).isEqualTo(RingbackTone.Cadence.CEPT)
        assertThat(RingbackTone.cadence("CN")).isEqualTo(RingbackTone.Cadence.CEPT)
        assertThat(RingbackTone.cadence("ZZ")).isEqualTo(RingbackTone.Cadence.CEPT)
        assertThat(RingbackTone.cadence("")).isEqualTo(RingbackTone.Cadence.CEPT)
        // Case is the locale's problem, not the table's.
        assertThat(RingbackTone.cadence("us")).isEqualTo(RingbackTone.Cadence.ANSI)
        assertThat(RingbackTone.cadence("gb")).isEqualTo(RingbackTone.Cadence.UK)
    }

    @Test
    fun theFourCadencesCarryTheStandardTonesAndPatterns() {
        assertThat(RingbackTone.Cadence.ANSI.frequenciesHz).containsExactly(440.0, 480.0).inOrder()
        assertThat(RingbackTone.Cadence.ANSI.pattern).containsExactly(2.0, 4.0).inOrder()
        assertThat(RingbackTone.Cadence.UK.frequenciesHz).containsExactly(400.0, 450.0).inOrder()
        assertThat(RingbackTone.Cadence.UK.pattern).containsExactly(0.4, 0.2, 0.4, 2.0).inOrder()
        assertThat(RingbackTone.Cadence.JAPAN.frequenciesHz).containsExactly(400.0)
        assertThat(RingbackTone.Cadence.JAPAN.pattern).containsExactly(1.0, 2.0).inOrder()
        assertThat(RingbackTone.Cadence.CEPT.frequenciesHz).containsExactly(425.0)
        assertThat(RingbackTone.Cadence.CEPT.pattern).containsExactly(1.0, 4.0).inOrder()
        for (cadence in RingbackTone.Cadence.entries) {
            assertThat(cadence.pattern.size % 2).isEqualTo(0)
            assertThat(cadence.pattern.all { it > 0 }).isTrue()
        }
        assertThat(RingbackTone.Cadence.UK.cycleSeconds).isEqualTo(3.0)
        assertThat(RingbackTone.Cadence.CEPT.cycleSeconds).isEqualTo(5.0)
    }

    @Test
    fun aCycleIsExactlyAsLongAsItsPattern() {
        for (cadence in RingbackTone.Cadence.entries) {
            assertThat(RingbackTone.cycle(cadence).size).isEqualTo(Math.round(cadence.cycleSeconds * 8000).toInt())
        }
        assertThat(RingbackTone.cycle(RingbackTone.Cadence.ANSI).size).isEqualTo(48_000)
        assertThat(RingbackTone.cycle(RingbackTone.Cadence.UK).size).isEqualTo(24_000)
        assertThat(RingbackTone.cycle(RingbackTone.Cadence.JAPAN).size).isEqualTo(24_000)
        assertThat(RingbackTone.cycle(RingbackTone.Cadence.CEPT).size).isEqualTo(40_000)
        assertThat(RingbackTone.cycle(RingbackTone.Cadence.CEPT, sampleRate = 16_000).size).isEqualTo(80_000)
    }

    @Test
    fun ceptByLiteralSampleIndex() {
        val pcm = RingbackTone.cycle(RingbackTone.Cadence.CEPT)
        assertThat(pcm.size).isEqualTo(40_000)
        assertThat(pcm[0]).isEqualTo(0.toShort())
        // Not 4000: at 0.5 s a 425 Hz tone is exactly at a zero crossing.
        assertThat(abs(pcm[4004].toInt())).isGreaterThan(3000)
        assertThat(abs(pcm[7999].toInt())).isLessThan(100)
        assertThat(pcm[8000]).isEqualTo(0.toShort())
        assertThat(pcm.copyOfRange(8000, 40_000).all { it == 0.toShort() }).isTrue()
        assertThat(pcm.copyOfRange(64, 7936).any { abs(it.toInt()) > 3000 }).isTrue()
    }

    @Test
    fun burstsSoundGapsAreSilentAndEdgesAreRamped() {
        for (cadence in RingbackTone.Cadence.entries) {
            val rate = RingbackTone.SAMPLE_RATE
            val pcm = RingbackTone.cycle(cadence)
            var cursor = 0.0
            cadence.pattern.forEachIndexed { index, seconds ->
                val start = Math.round(cursor * rate).toInt()
                cursor += seconds
                val end = Math.round(cursor * rate).toInt()
                val segment = pcm.copyOfRange(start, end)
                if (index % 2 == 0) {
                    val peak = segment.maxOf { abs(it.toInt()) }
                    assertThat(peak).isGreaterThan(3000)
                    // The first and last samples of a burst are at the foot
                    // of the ramp — a click would be a full-scale sample.
                    assertThat(abs(segment.first().toInt())).isLessThan(100)
                    assertThat(abs(segment.last().toInt())).isLessThan(100)
                } else {
                    assertThat(segment.all { it == 0.toShort() }).isTrue()
                }
            }
        }
    }

    @Test
    fun twoSummedTonesStayUnderMinusSixDbfsAndOneUnderMinusTwelve() {
        val twoTones = RingbackTone.cycle(RingbackTone.Cadence.ANSI).maxOf { abs(it.toInt()) }
        val oneTone = RingbackTone.cycle(RingbackTone.Cadence.CEPT).maxOf { abs(it.toInt()) }
        assertThat(twoTones).isAtMost((Short.MAX_VALUE * 0.5).toInt() + 1)
        assertThat(oneTone).isAtMost((Short.MAX_VALUE * 0.25).toInt() + 1)
        assertThat(twoTones).isGreaterThan(oneTone)
    }
}
