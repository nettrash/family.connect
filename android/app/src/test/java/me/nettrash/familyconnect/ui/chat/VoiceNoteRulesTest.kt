/*
 * VoiceNoteRulesTest.kt
 * Family Connect (Android)
 *
 * The numbers today's recorder lives by since it was made safe (#79,
 * Phase 0) are the SHARED rules' numbers: every port checks the same
 * `record-vectors.json`, printed by the reference (`fc_text::record`, via
 * `cargo run -- record` in win/tools/board-oracle). This holds the ones
 * Phase 0 uses to its constants case — the full set, and every rule, are
 * Phase 1's to port (ComposerSlot, RecordGesture).
 *
 * Plain JUnit: VoiceNoteRules touches nothing from android.*.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import me.nettrash.familyconnect.data.repo.VoiceRecorder
import org.junit.Test

class VoiceNoteRulesTest {

    private val constants = run {
        val stream = requireNotNull(javaClass.classLoader?.getResourceAsStream(VECTORS)) {
            "$VECTORS is missing from the test classpath (app/src/test/resources)"
        }
        val text = stream.use { it.readBytes().toString(Charsets.UTF_8) }
        Json.parseToJsonElement(text).jsonArray
            .map { it.jsonObject }
            .single { it.getValue("function").jsonPrimitive.content == "constants" }
            .getValue("expected").jsonObject
    }

    private fun constant(name: String): Long = constants.getValue(name).jsonPrimitive.long

    @Test
    fun `the constants are the shared rules' own`() {
        assertThat(VoiceNoteRules.SHORTEST_RECORDING_MS).isEqualTo(constant("shortest_recording_ms"))
        assertThat(VoiceNoteRules.DELETE_ASKS_FROM_MS).isEqualTo(constant("delete_asks_from_ms"))
        assertThat(VoiceNoteRules.VOICE_CAP_MS).isEqualTo(constant("voice_cap_ms"))
        // The cap the recorder enforces is the one the rules name.
        assertThat(VoiceRecorder.MAX_DURATION_MS.toLong()).isEqualTo(constant("voice_cap_ms"))
    }

    @Test
    fun `a second is kept, a moment less is not`() {
        assertThat(VoiceNoteRules.isWorthKeeping(999)).isFalse()
        assertThat(VoiceNoteRules.isWorthKeeping(1_000)).isTrue()
        assertThat(VoiceNoteRules.isWorthKeeping(0)).isFalse()
    }

    @Test
    fun `ten seconds or more asks before it is deleted`() {
        assertThat(VoiceNoteRules.deleteAsks(9_999)).isFalse()
        assertThat(VoiceNoteRules.deleteAsks(10_000)).isTrue()
        assertThat(VoiceNoteRules.deleteAsks(300_000)).isTrue()
    }

    private companion object {
        const val VECTORS = "record-vectors.json"
    }

    // -- Phase 1: what the reducer leaves to the port (S2.5, S2.9) ----------

    /** The meter's five bars light at −50, −40, −30, −20 and −10 dBFS of the peak. */
    @Test
    fun `the level meter lights a bar at each ten decibels from minus fifty`() {
        assertThat(VoiceNoteRules.levelBars(0)).isEqualTo(0)
        // −50 dBFS is 103.6 of 32 767: 103 stays dark, 104 lights the first bar.
        assertThat(VoiceNoteRules.levelBars(103)).isEqualTo(0)
        assertThat(VoiceNoteRules.levelBars(104)).isEqualTo(1)
        // −40 dBFS is 327.7; −30 is 1 036.2; −20 is 3 276.7; −10 is 10 361.9.
        assertThat(VoiceNoteRules.levelBars(327)).isEqualTo(1)
        assertThat(VoiceNoteRules.levelBars(328)).isEqualTo(2)
        assertThat(VoiceNoteRules.levelBars(1_037)).isEqualTo(3)
        assertThat(VoiceNoteRules.levelBars(3_277)).isEqualTo(4)
        assertThat(VoiceNoteRules.levelBars(10_361)).isEqualTo(4)
        assertThat(VoiceNoteRules.levelBars(10_362)).isEqualTo(5)
        assertThat(VoiceNoteRules.levelBars(VoiceNoteRules.FULL_SCALE)).isEqualTo(5)
    }

    /** Silence is a muted microphone — getMaxAmplitude() at or below 32 — not a quiet room. */
    @Test
    fun `heard means a peak above thirty two, the shared rules' own number`() {
        assertThat(constant("silence_max_amplitude")).isEqualTo(32L)
        assertThat(VoiceNoteRules.isHeard(32)).isFalse()
        assertThat(VoiceNoteRules.isHeard(33)).isTrue()
        assertThat(VoiceNoteRules.isHeard(0)).isFalse()
        // 32 is the −60 dBFS the rules name, to a tenth of a decibel.
        assertThat(VoiceNoteRules.dbfs(32)).isWithin(0.25).of(-60.0)
    }

    /** "We can't hear anything…" three seconds into a recording that has heard nothing — never after sound. */
    @Test
    fun `the silence warning waits three seconds and goes with sound`() {
        val after = constant("silence_warning_after_ms")
        assertThat(VoiceNoteRules.showsSilenceWarning(after - 1, heard = false)).isFalse()
        assertThat(VoiceNoteRules.showsSilenceWarning(after, heard = false)).isTrue()
        assertThat(VoiceNoteRules.showsSilenceWarning(after * 10, heard = true)).isFalse()
    }

    /** "30 seconds left" from 4:30. */
    @Test
    fun `the time warning starts at four thirty`() {
        val warning = constant("voice_warning_ms")
        assertThat(VoiceNoteRules.showsTimeWarning(warning - 1)).isFalse()
        assertThat(VoiceNoteRules.showsTimeWarning(warning)).isTrue()
    }

    /** m:ss, whole seconds, never negative. */
    @Test
    fun `the clock reads minutes and seconds`() {
        assertThat(VoiceNoteRules.clock(0)).isEqualTo("0:00")
        assertThat(VoiceNoteRules.clock(42_999)).isEqualTo("0:42")
        assertThat(VoiceNoteRules.clock(270_000)).isEqualTo("4:30")
        assertThat(VoiceNoteRules.clock(-5)).isEqualTo("0:00")
    }
}
