/*
 * VoiceNoteRules.kt
 * Family Connect (Android)
 *
 * The numbers today's recorder lives by since it was made safe (#79,
 * docs/audio-video-messages-2026-10-04.md, Phase 0, S1.1), and — since
 * Phase 1 — the display state the shared reducer leaves to each port (S2.5,
 * S2.9): the level meter, the silence warning and the 4:30 warning, all read
 * from the one measure Android has, `getMaxAmplitude()`, a PEAK.
 *
 * The constants ARE the shared rules' (ComposerSlot, held to the printed
 * vectors), so a recording kept on this phone is one the iPhone would keep
 * too. Arithmetic only, free of Android, so a plain JUnit test pins it.
 */

package me.nettrash.familyconnect.ui.chat

import kotlin.math.log10

object VoiceNoteRules {

    /**
     * Nothing shorter is ever sent (S1.1). A Stop under it is "too short",
     * and an interruption under it keeps nothing: there is nothing worth
     * keeping (S4). This, not 1024 bytes, is the floor people see.
     */
    const val SHORTEST_RECORDING_MS = ComposerSlot.SHORTEST_RECORDING_MS

    /** Deleting a recording this long or longer asks first (S1.1, S2.8). */
    const val DELETE_ASKS_FROM_MS = ComposerSlot.DELETE_ASKS_FROM_MS

    /** Five minutes, then it stops into review — never sent (S2.5). */
    const val VOICE_CAP_MS = ComposerSlot.VOICE_CAP_MS

    /** Whether a recording that ended without being sent is kept at all. */
    fun isWorthKeeping(durationMs: Long): Boolean = durationMs >= SHORTEST_RECORDING_MS

    /** Whether deleting it asks "Delete this recording?" first. */
    fun deleteAsks(durationMs: Long): Boolean = durationMs >= DELETE_ASKS_FROM_MS

    /** The loudest a 16-bit sample can be: `getMaxAmplitude()`'s full scale. */
    const val FULL_SCALE = 32_767

    /**
     * Where the level meter's five bars light (S2.9): at −50, −40, −30, −20
     * and −10 dBFS of the PEAK — the silence check's own measure, so the bars
     * light alike on every client.
     */
    val LEVEL_BARS_DBFS: List<Double> = listOf(-50.0, -40.0, -30.0, -20.0, -10.0)

    /** The peak as dBFS; −∞ for silence. */
    fun dbfs(maxAmplitude: Int): Double =
        if (maxAmplitude <= 0) Double.NEGATIVE_INFINITY else 20.0 * log10(maxAmplitude.toDouble() / FULL_SCALE)

    /** How many of the five bars [maxAmplitude] lights. */
    fun levelBars(maxAmplitude: Int): Int {
        val level = dbfs(maxAmplitude)
        return LEVEL_BARS_DBFS.count { level >= it }
    }

    /**
     * Whether a peak rose above digital silence — a muted microphone, not a
     * quiet room (S1.1): Android's `getMaxAmplitude()` above 32.
     */
    fun isHeard(maxAmplitude: Int): Boolean = maxAmplitude > ComposerSlot.SILENCE_MAX_AMPLITUDE

    /**
     * "We can't hear anything. Is the microphone muted?" — three seconds into
     * a recording that has heard nothing; it goes when sound arrives (S2.9).
     */
    fun showsSilenceWarning(recordedMs: Long, heard: Boolean): Boolean =
        !heard && recordedMs >= ComposerSlot.SILENCE_WARNING_AFTER_MS

    /** "30 seconds left", and the timer turns orange: from 4:30 (S2.5). */
    fun showsTimeWarning(recordedMs: Long): Boolean = recordedMs >= ComposerSlot.VOICE_WARNING_MS

    /** `0:07`, `4:30` — m:ss, what the timer counts in and what a length reads as. */
    fun clock(ms: Long): String {
        val whole = (ms / 1000).coerceAtLeast(0)
        return "%d:%02d".format(whole / 60, whole % 60)
    }
}
