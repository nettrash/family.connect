/*
 * ComposerSlot.kt
 * Family Connect (Android)
 *
 * Voice and video messages from the Send button (#79,
 * docs/audio-video-messages-2026-10-04.md — "the plan" below): the composer's
 * trailing slot, the video button inside the empty field, the round video's
 * arithmetic and the S1.1 constants, as the SHARED rules have them.
 *
 * The reference is `fc_text::record` in web/text/src/record.rs. This is a
 * port of it, not a reading of the plan: every type, field and branch here
 * is the reference's, under the same name in Kotlin's spelling, and
 * ComposerSlotVectorsTest holds it to the vectors `win/tools/board-oracle`
 * prints from the reference (`cargo run -- record`, copied to
 * app/src/test/resources/record-vectors.json) — "an oracle, not four
 * readings". The hold reducer is RecordGesture.kt's.
 *
 * Words are the apps' English source strings — the catalogue's keys (S10);
 * the screen says each in the reader's language through its own string
 * resource (RecordStrings), and a Robolectric test checks that the English
 * resource IS the key. Nothing here touches Android, so a plain JUnit test
 * pins it.
 *
 * The video button's rule came first (Phase 1), so that Phase 3 only had to
 * wire it: since Phase 3 this build records round video (VideoMessageRecorder,
 * CameraX), so it passes `recordsRoundVideo = true` and the door opens
 * wherever the server and the device allow (Decision 40).
 *
 * iOS counterpart: ios/FamilyConnect/Models/ComposerSlot.swift
 */

package me.nettrash.familyconnect.ui.chat

object ComposerSlot {

    // --- S1.1, the constants ---------------------------------------------

    /**
     * The slot ignores activation for this long only after its OWN activation
     * changed it — a send that empties the composer, a tap or hold that
     * starts a recording, a Send or a release that ends one or finds it too
     * short, a Stop in row 3 that stages the note. A change made by typing,
     * pasting or staging is never guarded.
     */
    const val ACTIVATION_GUARD_MS = 600L

    /** H's floor: H = max(500 ms, the system long-press duration). */
    const val MIN_HOLD_THRESHOLD_MS = 500L

    /** A press that moves farther than this before H can no longer become a hold. */
    const val TAP_SLOP = 20.0

    /** Upward from where the press went down: the hold locks hands-free. */
    const val LOCK_DISTANCE = 60.0

    /** Toward the leading edge: cancel arms at the first, disarms below the second. */
    const val CANCEL_ARM_DISTANCE = 100.0
    const val CANCEL_DISARM_DISTANCE = 80.0

    /** Nothing shorter is ever sent; a hold released sooner keeps recording. */
    const val SHORTEST_RECORDING_MS = 1_000L

    /** After a release that sends: the grace before anything leaves the device. */
    const val UNDO_WINDOW_MS = 5_000L

    /** A voice note's length, and where "30 seconds left" is shown and said. */
    const val VOICE_CAP_MS = 300_000L
    const val VOICE_WARNING_MS = 270_000L

    /** `max_round_video_ms` when a server has round video. */
    const val DEFAULT_MAX_ROUND_VIDEO_MS = 60_000L

    /** A round video stops this much before `max_round_video_ms`, and warns this much before it. */
    const val ROUND_CAP_MARGIN_MS = 500L
    const val ROUND_WARNING_LEAD_MS = 10_000L

    /**
     * Digital silence — a muted microphone, not a quiet room: no PEAK above
     * −60 dBFS. Android reads it as `getMaxAmplitude()` at or below 32.
     */
    const val SILENCE_PEAK_DBFS = -60.0
    const val SILENCE_MAX_AMPLITUDE = 32
    const val SILENCE_SAMPLE_MAGNITUDE = 0.001

    /** "We can't hear anything…" when nothing has risen above silence this long in. */
    const val SILENCE_WARNING_AFTER_MS = 3_000L

    /** Deleting a recording this long or longer asks first. */
    const val DELETE_ASKS_FROM_MS = 10_000L

    /** "Still recording. Tap Send when you're done." stays this long. */
    const val STILL_RECORDING_HINT_MS = 3_000L

    /** The video recorder's PREVIEW closes after this long with no control used. */
    const val PREVIEW_IDLE_CLOSE_MS = 60_000L

    /** The slot's Send ↔ microphone cross-fade and the recorder's fade. */
    const val SLOT_CROSSFADE_MS = 150L
    const val RECORDER_FADE_MS = 200L

    /** The least hit area on a coarse pointer, in each platform's unit. */
    const val MIN_TARGET_APPLE_PT = 44
    const val MIN_TARGET_ANDROID_DP = 48
    const val MIN_TARGET_WINDOWS_EPX = 44
    const val MIN_TARGET_WEB_PX = 44

    /** A received circle's diameter (S5.2): compact widths, and everything else. */
    const val ROUND_DIAMETER_COMPACT = 200
    const val ROUND_DIAMETER_REGULAR = 240

    /**
     * H for a system whose own long press takes [systemLongPressMs] —
     * `ViewConfiguration.getLongPressTimeout()`, which follows the person's
     * "Touch & hold delay". Never shorter than [MIN_HOLD_THRESHOLD_MS].
     */
    fun holdThresholdMs(systemLongPressMs: Long): Long =
        maxOf(systemLongPressMs, MIN_HOLD_THRESHOLD_MS)

    // --- S1.3, the trailing slot -----------------------------------------

    /** The voice recording the composer is showing, as far as the slot is concerned. */
    enum class Recording {
        /** No voice recording runs. */
        NONE,

        /** A finger or pen holds the microphone and it records (S2.3). */
        HELD,

        /** Hands-free, started with the composer empty: row 2, the Send arrow. */
        HANDS_FREE,

        /** Hands-free, started beside words or staged items: row 3, the Stop square. */
        HANDS_FREE_BESIDE_DRAFT,
    }

    /**
     * Why the microphone is dimmed — rows 7, 8 and 9, in that order of
     * precedence. Dimmed is not disabled: the control stays focusable and
     * hittable and says why when activated.
     */
    enum class Dimmed(
        /** The sentence the composer's notice line says, and TalkBack hears with the control. */
        val notice: String,
    ) {
        CALL("You can record a message after the call."),
        BUSY("Wait until the current attachment is done."),
        NOT_SENT("Send or delete the voice message that wasn't sent first."),
    }

    /** What the composer is, for the slot (S1.2). Thread composers do not ask. */
    data class SlotInputs(
        /** The video recorder is open (S3) — it owns the row. */
        val recorderOpen: Boolean,
        val recording: Recording,
        /** An edit is open. */
        val editing: Boolean,
        /** The draft is blank after trimming whitespace — this client's own trim. */
        val draftBlank: Boolean,
        /** Anything is staged. A primed reply is not. */
        val staged: Boolean,
        /** The assistant's chat (`kind = ai`). */
        val assistantChat: Boolean,
        /** The platform can record sound at all. */
        val canRecord: Boolean,
        /** A call in any phase but idle or ended. */
        val call: Boolean,
        /** The composer's attachment guard (`mediaState.isBusy`). */
        val busy: Boolean,
        /** The chat holds a not-sent voice message. */
        val notSent: Boolean,
    ) {
        /** S1.2's **empty**: the draft is blank AND nothing is staged. */
        val empty: Boolean get() = draftBlank && !staged
    }

    /** What the slot shows and does — one row of S1.3 each. */
    sealed interface Slot {
        /** Row 1: the video recorder owns the row. */
        data object Recorder : Slot

        /** Row 2 while a finger holds it: the pressed microphone stays under the finger. */
        data object HeldMicrophone : Slot

        /** Row 2: the Send arrow — stops and sends (S2.5). */
        data object SendVoice : Slot

        /** Row 3: the Stop square — stops; the note is staged beside the words. */
        data object StopRecording : Slot

        /** Row 4: today's Save, disabled while the field is blank. Never a microphone. */
        data class Save(val enabled: Boolean) : Slot

        /** Row 5: Send, by today's rules. */
        data object Send : Slot

        /** Row 6: Send, disabled — the assistant's chat, or where nothing can record. */
        data object SendDisabled : Slot

        /** Rows 7–9: the microphone, dimmed; activating it says why. */
        data class Dimmed(val reason: ComposerSlot.Dimmed) : Slot

        /** Row 10: the microphone. */
        data object Microphone : Slot

        /** The S1.3 row this is. */
        val row: Int
            get() = when (this) {
                Recorder -> 1
                HeldMicrophone, SendVoice -> 2
                StopRecording -> 3
                is Save -> 4
                Send -> 5
                SendDisabled -> 6
                is Dimmed -> when (reason) {
                    ComposerSlot.Dimmed.CALL -> 7
                    ComposerSlot.Dimmed.BUSY -> 8
                    ComposerSlot.Dimmed.NOT_SENT -> 9
                }
                Microphone -> 10
            }

        /** Its accessibility label (S6), or null where the recorder owns the row. */
        val label: String?
            get() = when (this) {
                Recorder -> null
                HeldMicrophone, SendVoice -> "Send voice message"
                StopRecording -> "Stop recording"
                is Save -> "Save"
                Send, SendDisabled -> "Send"
                is Dimmed, Microphone -> "Record voice message"
            }

        /** What activating it says instead of acting: a dimmed microphone's reason. */
        val notice: String?
            get() = (this as? Dimmed)?.reason?.notice

        /** Rows 7 to 10 — where the video button may show (S1.4). */
        val isMicrophone: Boolean
            get() = this is Dimmed || this == Microphone
    }

    /** The composer's trailing slot (S1.3): the first matching row wins. */
    fun composerSlot(inputs: SlotInputs): Slot {
        if (inputs.recorderOpen) return Slot.Recorder
        when (inputs.recording) {
            Recording.HELD -> return Slot.HeldMicrophone
            Recording.HANDS_FREE -> return Slot.SendVoice
            Recording.HANDS_FREE_BESIDE_DRAFT -> return Slot.StopRecording
            Recording.NONE -> Unit
        }
        if (inputs.editing) return Slot.Save(enabled = !inputs.draftBlank)
        if (!inputs.empty) return Slot.Send
        if (inputs.assistantChat || !inputs.canRecord) return Slot.SendDisabled
        if (inputs.call) return Slot.Dimmed(Dimmed.CALL)
        if (inputs.busy) return Slot.Dimmed(Dimmed.BUSY)
        if (inputs.notSent) return Slot.Dimmed(Dimmed.NOT_SENT)
        return Slot.Microphone
    }

    // --- S1.4, the video button ------------------------------------------

    /** The video button's label, and its desktop and pointer tooltip. */
    const val VIDEO_DOOR_LABEL = "Record video message"
    const val VIDEO_DOOR_TOOLTIP = "Record a video message"

    /**
     * Whether THIS build records round video on Android — its Phase 3, which
     * it does (VideoMessageRecorder). A build that could only receive circles
     * would draw no video entry anywhere (Decision 40).
     */
    const val RECORDS_ROUND_VIDEO = true

    /** What the video button needs to know besides the slot. */
    data class DoorInputs(
        val slot: SlotInputs,
        /** The main composer of a family or a direct chat. */
        val familyOrDirectChat: Boolean,
        /** A released voice message waits out its Undo window (S2.6). */
        val undoWindow: Boolean,
        /** The server sends `max_round_video_ms` on `GET /families/mine`. */
        val serverOffersRound: Boolean,
        /** The device has a camera. */
        val hasCamera: Boolean,
        /** The web's encoder probe; `true` everywhere else. */
        val encoderProbePasses: Boolean,
        /** This build records round video on this platform ([RECORDS_ROUND_VIDEO]). */
        val recordsRoundVideo: Boolean,
    ) {
        /** S1.2's **round available**: all four. */
        val roundAvailable: Boolean
            get() = serverOffersRound && hasCamera && encoderProbePasses && recordsRoundVideo
    }

    /** The video button inside the empty field (S1.4). */
    sealed interface Door {
        /** Not drawn — the field gets its width back. */
        data object Hidden : Door

        /** Drawn dimmed, saying the slot's own sentence: rows 7 and 8 only. */
        data class Dimmed(val reason: ComposerSlot.Dimmed) : Door

        /** Drawn; activating it opens the recorder (S3). */
        data object Shown : Door

        val label: String?
            get() = if (this == Hidden) null else VIDEO_DOOR_LABEL

        val notice: String?
            get() = (this as? Dimmed)?.reason?.notice
    }

    /** Whether the video button is hidden, dimmed or shown (S1.4). */
    fun videoDoor(inputs: DoorInputs): Door {
        if (!inputs.familyOrDirectChat || inputs.undoWindow || !inputs.roundAvailable) return Door.Hidden
        return when (val slot = composerSlot(inputs.slot)) {
            is Slot.Dimmed -> when (slot.reason) {
                Dimmed.CALL, Dimmed.BUSY -> Door.Dimmed(slot.reason)
                Dimmed.NOT_SENT -> Door.Shown
            }
            Slot.Microphone -> Door.Shown
            else -> Door.Hidden
        }
    }

    // --- The round video's arithmetic ------------------------------------

    /** Where a round video stops: the limit − 500 ms; never below 0. */
    fun roundCapMs(maxRoundVideoMs: Long): Long = saturatingSub(maxRoundVideoMs, ROUND_CAP_MARGIN_MS)

    /** Where "10 seconds left" is shown: the limit − 10 000 ms; never below 0. */
    fun roundWarningMs(maxRoundVideoMs: Long): Long = saturatingSub(maxRoundVideoMs, ROUND_WARNING_LEAD_MS)

    /** How wide a window draws a received circle (S5.2). */
    enum class WidthClass {
        /** An Android window under 600 dp. */
        COMPACT,

        /** 600 dp and wider. */
        REGULAR,
    }

    /** A received circle's diameter — a RECOMMENDATION nothing on the wire carries. */
    fun roundDiameter(width: WidthClass): Int = when (width) {
        WidthClass.COMPACT -> ROUND_DIAMETER_COMPACT
        WidthClass.REGULAR -> ROUND_DIAMETER_REGULAR
    }

    /** What [isRound] reads of one attachment. */
    data class AttachmentFlags(
        /** The attachment's `kind`, as the wire spells it. */
        val kind: String,
        /** Its `round` — absent on the wire is false. */
        val round: Boolean,
    )

    /**
     * The drawing test (S5.1): exactly one attachment, `kind = video`,
     * carrying `round: true`, and no body — compared EXACTLY, as the
     * reference does, so no port's idea of whitespace can make two clients
     * disagree.
     */
    fun isRound(body: String, attachments: List<AttachmentFlags>): Boolean =
        body.isEmpty() && attachments.size == 1 && attachments[0].kind == "video" && attachments[0].round

    /** u64's `saturating_sub` for the non-negative values the rules use. */
    internal fun saturatingSub(a: Long, b: Long): Long = if (a <= b) 0L else a - b

    /** u64's `saturating_add`, capped at Long.MAX_VALUE. */
    internal fun saturatingAdd(a: Long, b: Long): Long = if (a > Long.MAX_VALUE - b) Long.MAX_VALUE else a + b
}
