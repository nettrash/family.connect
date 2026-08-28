/*
 * RingbackTone.kt
 * Family Connect (Android)
 *
 * The sound the CALLER hears while the far side rings: a synthesised
 * ringback tone in the cadence of the caller's own country, so placing a
 * call in the app sounds the way placing one on the phone does. Nothing
 * on the wire — the server's `call_ringing` frame is what starts it, and
 * the answer (or any end) is what stops it (CallManager).
 *
 * Pure Kotlin, no Android classes: the cadence table and the synthesis
 * are pinned on the JVM (RingbackToneTest), and AndroidCallAudio only
 * loops the one cycle this produces in an AudioTrack. The table is
 * shared BY VALUE with iOS — same names, same constants, in the same
 * order — so the two files can be diffed side by side.
 *
 * Four cadences cover the world well enough for a family app: the North
 * American one (ANSI), the British double ring (also used across the
 * Commonwealth and in Hong Kong / Singapore), Japan's, and the CEPT
 * single 425 Hz burst that most of Europe, Russia and China share — the
 * default for any country not listed. The region comes from the
 * device's locale, passed in so tests can choose.
 *
 * iOS counterpart: ios/FamilyConnect/Core/Calls/RingbackTone.swift
 */

package me.nettrash.familyconnect.calls

import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.min
import kotlin.math.roundToInt
import kotlin.math.sin

object RingbackTone {

    /** Telephone bandwidth; every tone in the table sits well under 4 kHz. */
    const val SAMPLE_RATE = 8000

    /**
     * Per-tone amplitude, about -12 dBFS. Two summed tones peak near
     * -6 dBFS, so nothing here can ever clip — and the tone sits under
     * the far side's voice once the call connects, the way a network's
     * ringback does.
     */
    const val TONE_AMPLITUDE = 0.25

    /** Raised-cosine ramp at every on/off edge, so a burst never clicks. */
    const val RAMP_SECONDS = 0.008

    /**
     * One cadence: the tones summed, and the on/off pattern in seconds,
     * ALTERNATING starting with "on" — so a pattern always has an even
     * number of entries and one cycle is its sum.
     */
    enum class Cadence(val frequenciesHz: List<Double>, val pattern: List<Double>) {
        /** US, Canada and the rest of the North American Numbering Plan. */
        ANSI(listOf(440.0, 480.0), listOf(2.0, 4.0)),

        /** The British double ring: UK, Ireland, Australia, New Zealand, Hong Kong, Singapore. */
        UK(listOf(400.0, 450.0), listOf(0.4, 0.2, 0.4, 2.0)),

        JAPAN(listOf(400.0), listOf(1.0, 2.0)),

        /** The European default — Serbia, Germany, France, Spain, Russia, China… */
        CEPT(listOf(425.0), listOf(1.0, 4.0)),
        ;

        val cycleSeconds: Double get() = pattern.sum()
    }

    val ANSI_COUNTRIES: Set<String> = setOf(
        "US", "CA",
        "AG", "AI", "AS", "BB", "BM", "BS", "DM", "DO", "GD", "GU", "JM", "KN",
        "KY", "LC", "MP", "MS", "PR", "SX", "TC", "TT", "VC", "VG", "VI",
    )

    val UK_COUNTRIES: Set<String> = setOf("GB", "IE", "AU", "NZ", "HK", "SG")

    val JAPAN_COUNTRIES: Set<String> = setOf("JP")

    /**
     * The cadence for an ISO 3166 country code, as `Locale.getDefault().country`
     * gives it. Case does not matter; an empty or unknown code is CEPT.
     */
    fun cadence(country: String): Cadence = when (country.uppercase()) {
        in ANSI_COUNTRIES -> Cadence.ANSI
        in UK_COUNTRIES -> Cadence.UK
        in JAPAN_COUNTRIES -> Cadence.JAPAN
        else -> Cadence.CEPT
    }

    /**
     * One full cycle of [cadence] as 16-bit mono PCM at [sampleRate]:
     * the tones summed through the "on" segments, ramped in and out at
     * each edge, and EXACT silence through the "off" ones. Looped end to
     * end it rings for as long as the player lets it — the seam lands in
     * the trailing silence, so no phase jump is ever heard.
     */
    fun cycle(cadence: Cadence, sampleRate: Int = SAMPLE_RATE): ShortArray {
        val frames = (cadence.cycleSeconds * sampleRate).roundToInt()
        val pcm = ShortArray(frames)
        val ramp = (RAMP_SECONDS * sampleRate).roundToInt()
        var cursor = 0.0
        cadence.pattern.forEachIndexed { index, seconds ->
            val start = (cursor * sampleRate).roundToInt()
            cursor += seconds
            val end = min((cursor * sampleRate).roundToInt(), frames)
            if (index % 2 != 0) return@forEachIndexed // an "off" segment: the zeros already there
            for (n in start until end) {
                val t = n.toDouble() / sampleRate
                var sample = 0.0
                for (hz in cadence.frequenciesHz) sample += TONE_AMPLITUDE * sin(2.0 * PI * hz * t)
                val envelope = min(edge(n - start, ramp), edge(end - 1 - n, ramp))
                // Clamped so a louder table could never wrap here (toShort()
                // of an out-of-range value is a full-scale buzz, not a crash).
                val value = (sample * envelope).coerceIn(-1.0, 1.0) * Short.MAX_VALUE
                pcm[n] = value.roundToInt().toShort()
            }
        }
        return pcm
    }

    /** The envelope [distance] frames from a segment's edge: 0 at the edge, 1 past [ramp], a half-cosine between. */
    private fun edge(distance: Int, ramp: Int): Double =
        if (distance >= ramp) 1.0 else 0.5 * (1.0 - cos(PI * distance / ramp))
}
