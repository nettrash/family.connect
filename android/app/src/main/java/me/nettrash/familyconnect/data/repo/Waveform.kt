/*
 * Waveform.kt
 * Family Connect (Android)
 *
 * A voice note's waveform (#79; docs/protocol.md, "A voice note's waveform"):
 * 48 levels of 0–15, one per equal slice of the recording, sent with the
 * upload as 48 lowercase hex digits and echoed on the attachment, so a voice
 * bubble draws its shape before anything is downloaded.
 *
 * The rules are the SHARED ones (`fc_text::waveform` in web/text), held to
 * the printed `waveform-vectors.json` by WaveformVectorsTest: a level is the
 * peak in dBFS clamped to −60…0 and divided into 4 dB steps, rounded half up
 * — no logarithm, double arithmetic — so every platform draws the same bars
 * from the same recording. Arithmetic only, free of Android.
 *
 * [PeakLog] is the sender's half: the recorder's peaks, one per meter tick,
 * turned into the wire string when the recording stops.
 */

package me.nettrash.familyconnect.data.repo

import java.math.BigInteger
import kotlin.math.floor
import kotlin.math.log10

object Waveform {
    /** How many levels the wire carries. */
    const val LEVELS = 48

    /** The loudest level: `f`. */
    const val MAX_LEVEL = 15

    /** Level 0: the silence floor, the same −60 dBFS the silence check uses. */
    const val FLOOR_DBFS = -60.0

    /** Each level is this many decibels. */
    const val DB_PER_LEVEL = 4.0

    /** What a bubble draws when there is no waveform, or it cannot be read. */
    const val PLACEHOLDER_LEVEL = 4

    val PLACEHOLDER: List<Int> = List(LEVELS) { PLACEHOLDER_LEVEL }

    /** A peak in dBFS as a level, 0…15 — NaN is silence, +∞ full scale. */
    fun level(dbfs: Double): Int {
        if (dbfs.isNaN()) return 0
        val clamped = dbfs.coerceIn(FLOOR_DBFS, 0.0)
        val x = (clamped - FLOOR_DBFS) / DB_PER_LEVEL
        val whole = floor(x)
        val rounded = if (x - whole >= 0.5) whole + 1.0 else whole
        return rounded.toInt().coerceAtMost(MAX_LEVEL)
    }

    /**
     * [levels] reduced (or stretched) to [count]: slice i covers
     * `⌊i·n/count⌋` up to `max(start+1, ⌊(i+1)·n/count⌋)` and takes its
     * highest level. Nothing at all is [count] zeros.
     */
    fun reduce(levels: List<Int>, count: Int): List<Int> {
        if (count <= 0) return emptyList()
        if (levels.isEmpty()) return List(count) { 0 }
        val n = levels.size.toLong()
        return List(count) { i ->
            val start = (i.toLong() * n / count).toInt()
            val end = maxOf(start + 1, ((i + 1).toLong() * n / count).toInt())
            var top = 0
            for (k in start until end) if (levels[k] > top) top = levels[k]
            top.coerceIn(0, MAX_LEVEL)
        }
    }

    /** Levels as the wire spells them: one lowercase hex digit each. */
    fun encode(levels: List<Int>): String =
        levels.joinToString("") { level -> Character.forDigit(level.coerceIn(0, MAX_LEVEL), 16).toString() }

    /** The sender's string: every peak a level, reduced to [levels] slices. */
    fun fromPeaks(samplesDbfs: List<Double>, levels: Int = LEVELS): String =
        encode(reduce(samplesDbfs.map(::level), levels))

    /** Exactly 48 of `0-9a-f`, or null — uppercase, a space, a stray digit of another script are all refused. */
    fun parse(waveform: String?): List<Int>? {
        if (waveform == null || waveform.length != LEVELS) return null
        val out = ArrayList<Int>(LEVELS)
        for (ch in waveform) {
            out += when (ch) {
                in '0'..'9' -> ch - '0'
                in 'a'..'f' -> ch - 'a' + 10
                else -> return null
            }
        }
        return out
    }

    /** What a bubble draws: the waveform, or the flat placeholder. */
    fun levelsOrPlaceholder(waveform: String?): List<Int> = parse(waveform) ?: PLACEHOLDER

    /** The bars that fit — the same slice rule. */
    fun bars(levels: List<Int>, count: Int): List<Int> = reduce(levels, count)

    /** A bar's height as a fraction of the full height: (2 + level) / 17. */
    fun barFraction(level: Int): Double = (2 + level.coerceIn(0, MAX_LEVEL)) / 17.0

    /** How many of [bars] are played at [positionMs] of [durationMs], never more than [bars]. */
    fun playedBars(positionMs: Long, durationMs: Long, bars: Int): Int =
        playedBars(positionMs.coerceAtLeast(0L).toULong(), durationMs.coerceAtLeast(0L).toULong(), bars)

    /** The wire's unsigned arithmetic: ⌊position·bars/duration⌋, capped at [bars]; 0 for no duration. */
    fun playedBars(positionMs: ULong, durationMs: ULong, bars: Int): Int {
        if (durationMs == 0uL || bars <= 0) return 0
        val played = BigInteger(positionMs.toString())
            .multiply(BigInteger.valueOf(bars.toLong()))
            .divide(BigInteger(durationMs.toString()))
        return played.min(BigInteger.valueOf(bars.toLong())).toInt()
    }

    /** `MediaRecorder.getMaxAmplitude()` — a 16-bit peak — as dBFS; −∞ for silence. */
    fun dbfs(maxAmplitude: Int): Double =
        if (maxAmplitude <= 0) Double.NEGATIVE_INFINITY else 20.0 * log10(maxAmplitude.toDouble() / FULL_SCALE)

    /** `getMaxAmplitude()`'s full scale. */
    const val FULL_SCALE = 32_767
}

/**
 * The recorder's peaks while it records, one per read of the meter (S2.9's
 * 200 ms tick), turned into the wire waveform at the end.
 *
 * The FIRST read after a start is dropped: `getMaxAmplitude()` answers 0 to
 * it whatever was heard, and that would draw a silent bar at the start of
 * every note. Bounded, so a five-minute note keeps every tick and nothing
 * longer grows without end.
 */
class PeakLog {
    private val peaks = ArrayList<Double>()
    private var first = true

    fun record(maxAmplitude: Int) {
        if (first) {
            first = false
            return
        }
        if (peaks.size < CAPACITY) peaks += Waveform.dbfs(maxAmplitude)
    }

    /** Nothing read but the dropped first: null, the waveform is left out. */
    fun waveform(): String? = if (peaks.isEmpty()) null else Waveform.fromPeaks(peaks)

    fun clear() {
        peaks.clear()
        first = true
    }

    private companion object {
        /** Five minutes of 200 ms ticks, and room to spare. */
        const val CAPACITY = 4_000
    }
}
