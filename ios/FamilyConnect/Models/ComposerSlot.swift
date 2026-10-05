//
//  ComposerSlot.swift
//  FamilyConnect
//
//  The composer's trailing slot, the video button beside it and the round
//  video's arithmetic — the shared rules of #79 as Swift types
//  (docs/audio-video-messages-2026-10-04.md, S1.1–S1.4, S5.1–S5.2).
//
//  A PORT, NOT A READING. The reference is `web/text/src/record.rs`
//  (`fc_text::record`), and its answers are printed by `win/tools/board-oracle`
//  (`cargo run -- record`) into `record-vectors.json` — the same bytes Android
//  and Windows check, and CI compares the three copies. RecordVectorTests runs
//  every case of that file against these types, so a rule here that drifts
//  from the reference fails on the case that shows how.
//
//  Names are the reference's, in Swift's case: `composer_slot` is
//  `ComposerSlot.of`, `SlotInputs` is `ComposerSlot.Inputs`, `draft_blank` is
//  `draftBlank`. Words are kept TWICE on purpose: `labelKey`/`noticeKey` are
//  the reference's English source strings — what the vectors print and what the
//  catalogue is keyed by — and `label`/`notice` are the same sentences through
//  `String(localized:)`, written as literals so `check-strings.py` and Xcode's
//  extraction can see them. A test holds each pair to the same key.
//
//  Pure and nonisolated, beside `StickerDoor` and `NotSentSendDoor`: both
//  composers ask, and the answers can be pinned without a composer on screen.
//

import Foundation

// MARK: - S1.1, the constants

/// The numbers every client decides by (S1.1, S5.2), under the reference's
/// names. They are constants, tuned after one device session (the plan's
/// Blocked 3).
nonisolated enum RecordRules {
    /// The slot ignores activation this long after its OWN activation changed
    /// it — and only then. Typing, pasting and staging are never guarded.
    static let activationGuardMS: UInt64 = 600
    /// The hold threshold's floor: H = max(500 ms, the system long press).
    static let minHoldThresholdMS: UInt64 = 500
    /// A press that moves farther than this before H can no longer hold; it
    /// still taps if it lifts inside the button.
    static let tapSlop: Double = 20
    /// Upward, from where the press went down, in window coordinates: locked.
    static let lockDistance: Double = 60
    /// Toward the leading edge: cancel armed at 100, disarmed below 80.
    static let cancelArmDistance: Double = 100
    static let cancelDisarmDistance: Double = 80
    /// Nothing shorter is ever sent; a hold released sooner keeps recording.
    static let shortestRecordingMS: UInt64 = 1_000
    /// The grace after a release that sends.
    static let undoWindowMS: UInt64 = 5_000
    /// Five minutes, and "30 seconds left" at 4:30.
    static let voiceCapMS: UInt64 = 300_000
    static let voiceWarningMS: UInt64 = 270_000
    static let defaultMaxRoundVideoMS: UInt64 = 60_000
    static let roundCapMarginMS: UInt64 = 500
    static let roundWarningLeadMS: UInt64 = 10_000
    /// Digital silence — a muted microphone, not a quiet room. Apple reads
    /// `peakPower` against the first; the others are the other platforms'.
    static let silencePeakDBFS: Double = -60
    static let silenceMaxAmplitude: UInt32 = 32
    static let silenceSampleMagnitude: Double = 0.001
    /// "We can't hear anything. Is the microphone muted?" this long in.
    static let silenceWarningAfterMS: UInt64 = 3_000
    /// Deleting a recording this long or longer asks first.
    static let deleteAsksFromMS: UInt64 = 10_000
    /// "Still recording. Tap Send when you're done." stays this long.
    static let stillRecordingHintMS: UInt64 = 3_000
    static let previewIdleCloseMS: UInt64 = 60_000
    /// Send ↔ microphone; none under Reduce Motion.
    static let slotCrossfadeMS: UInt64 = 150
    static let recorderFadeMS: UInt64 = 200
    /// The least hit area on a coarse pointer, per platform.
    static let minTargetApplePT: UInt32 = 44
    static let minTargetAndroidDP: UInt32 = 48
    static let minTargetWindowsEPX: UInt32 = 44
    static let minTargetWebPX: UInt32 = 44
    /// A received circle (S5.2).
    static let roundDiameterCompact: UInt32 = 200
    static let roundDiameterRegular: UInt32 = 240
}

// MARK: - S1.3, the trailing slot

/// What the composer's trailing slot shows and does — one row of S1.3 each,
/// the first matching row winning (`ComposerSlot.of`).
nonisolated enum ComposerSlot: Equatable, Sendable {
    /// Row 1: the video recorder owns the row.
    case recorder
    /// Row 2 while a finger holds it: the pressed microphone stays under the
    /// finger until the recording turns hands-free.
    case heldMicrophone
    /// Row 2: the Send arrow — stops and sends (S2.5).
    case sendVoice
    /// Row 3: the Stop square — stops; the note is staged beside the words.
    case stopRecording
    /// Row 4: today's Save, disabled while the field is blank. Never a
    /// microphone, even with the field cleared.
    case save(enabled: Bool)
    /// Row 5: Send, by today's rules.
    case send
    /// Row 6: Send, disabled — the assistant's chat, or nothing can record.
    case sendDisabled
    /// Rows 7–9: the microphone, dimmed; activating it says why.
    case dimmed(Dimmed)
    /// Row 10: the microphone — hands-free on activation, the walkie-talkie
    /// when held on a touch screen (S2.3).
    case microphone

    /// The voice recording the composer is showing, as far as the slot is
    /// concerned (`RecordGesture.HoldState.recording` says it of a state).
    nonisolated enum Recording: Equatable, Sendable {
        case none
        /// A finger or pen holds the microphone and it records.
        case held
        /// Hands-free, started with the composer empty: row 2.
        case handsFree
        /// Hands-free, started from the paperclip or the shortcut beside
        /// words or staged items: row 3.
        case handsFreeBesideDraft
    }

    /// Why the microphone is dimmed — rows 7, 8 and 9, in that order.
    /// Dimmed is not disabled: the control stays hittable and says why.
    nonisolated enum Dimmed: Equatable, Sendable {
        /// A call in any phase but idle or ended.
        case call
        /// The composer's attachment guard (`mediaState.blocksComposer`).
        case busy
        /// The chat holds a "Voice message not sent" row (S2.8).
        case notSent

        /// The reference's sentence, as the catalogue keys it.
        var noticeKey: String {
            switch self {
            case .call: "You can record a message after the call."
            case .busy: "Wait until the current attachment is done."
            case .notSent: "Send or delete the voice message that wasn't sent first."
            }
        }

        /// The sentence in the reader's language — said in the notice line,
        /// and to a screen reader as the control's value (S6).
        var notice: String {
            switch self {
            case .call: String(localized: "You can record a message after the call.")
            case .busy: String(localized: "Wait until the current attachment is done.")
            case .notSent: String(localized: "Send or delete the voice message that wasn't sent first.")
            }
        }
    }

    /// What the composer is, for the slot (S1.2). The defaults are an empty
    /// composer in a chat that can record, with nothing in the way.
    nonisolated struct Inputs: Equatable, Sendable {
        /// The video recorder is open (S3) — nothing opens it before Phase 3.
        var recorderOpen = false
        var recording: Recording = .none
        /// An edit is open.
        var editing = false
        /// The draft is blank after trimming whitespace.
        var draftBlank = true
        /// Anything is staged. A primed reply is not.
        var staged = false
        /// The assistant's chat (`kind = ai`).
        var assistantChat = false
        /// The platform can record sound at all.
        var canRecord = true
        /// A call in any phase but idle or ended.
        var call = false
        /// The composer's attachment guard.
        var busy = false
        /// The chat holds a not-sent voice message.
        var notSent = false

        /// S1.2's **empty**: the draft is blank AND nothing is staged.
        var isEmpty: Bool { draftBlank && !staged }

        /// The dimmed row a recording would meet here, if any — the same
        /// precedence as rows 7–9, asked whether or not the slot is a
        /// microphone right now: the paperclip's Record Voice Message beside
        /// words meets a call exactly as the microphone does.
        var blocked: Dimmed? {
            if call { return .call }
            if busy { return .busy }
            if notSent { return .notSent }
            return nil
        }
    }

    /// The composer's trailing slot (S1.3): the first matching row wins.
    static func of(_ inputs: Inputs) -> ComposerSlot {
        if inputs.recorderOpen { return .recorder }
        switch inputs.recording {
        case .held: return .heldMicrophone
        case .handsFree: return .sendVoice
        case .handsFreeBesideDraft: return .stopRecording
        case .none: break
        }
        if inputs.editing { return .save(enabled: !inputs.draftBlank) }
        if !inputs.isEmpty { return .send }
        if inputs.assistantChat || !inputs.canRecord { return .sendDisabled }
        if inputs.call { return .dimmed(.call) }
        if inputs.busy { return .dimmed(.busy) }
        if inputs.notSent { return .dimmed(.notSent) }
        return .microphone
    }

    /// The S1.3 row this is.
    var row: Int {
        switch self {
        case .recorder: 1
        case .heldMicrophone, .sendVoice: 2
        case .stopRecording: 3
        case .save: 4
        case .send: 5
        case .sendDisabled: 6
        case .dimmed(.call): 7
        case .dimmed(.busy): 8
        case .dimmed(.notSent): 9
        case .microphone: 10
        }
    }

    /// Its accessibility label as the catalogue keys it, or nil where the
    /// recorder owns the row.
    var labelKey: String? {
        switch self {
        case .recorder: nil
        case .heldMicrophone, .sendVoice: "Send voice message"
        case .stopRecording: "Stop recording"
        case .save: "Save"
        case .send, .sendDisabled: "Send"
        case .dimmed, .microphone: "Record voice message"
        }
    }

    /// Its accessibility label in the reader's language (S6).
    var label: String? {
        switch self {
        case .recorder: nil
        case .heldMicrophone, .sendVoice: String(localized: "Send voice message")
        case .stopRecording: String(localized: "Stop recording")
        case .save: String(localized: "Save")
        case .send, .sendDisabled: String(localized: "Send")
        case .dimmed, .microphone: String(localized: "Record voice message")
        }
    }

    /// What activating it says instead of acting: a dimmed microphone's
    /// reason, as the catalogue keys it.
    var noticeKey: String? {
        if case .dimmed(let reason) = self { return reason.noticeKey }
        return nil
    }

    /// The same, in the reader's language.
    var notice: String? {
        if case .dimmed(let reason) = self { return reason.notice }
        return nil
    }

    /// "The slot is a microphone" — rows 7 to 10, where the video button may
    /// show (S1.4) and where a press may become a hold (S2.3).
    var isMicrophone: Bool {
        switch self {
        case .dimmed, .microphone: true
        default: false
        }
    }

    /// Rows 2 to 5 — the states ⌘↩ and the Mac's Return activate. Never a
    /// microphone: Return in an empty field never records (S1.3).
    var takesReturn: Bool { (2...5).contains(row) }
}

// MARK: - S1.4, the video button

/// The video button inside the empty field (S1.4).
///
/// THE RULE ONLY, until Phase 3. No Apple build records round video yet, so
/// every composer passes `recordsRoundVideo: VideoDoor.thisBuildRecords`
/// (false) and the door stays hidden — Decision 40: a build that can only
/// receive circles shows no video entry at all. The rule is written here so
/// that the phase which records only has to wire it.
nonisolated enum VideoDoor: Equatable, Sendable {
    /// Not drawn — the field gets its width back.
    case hidden
    /// Drawn dimmed, saying the slot's own sentence: rows 7 and 8 only.
    case dimmed(ComposerSlot.Dimmed)
    /// Drawn; activating it opens the recorder (S3).
    case shown

    /// Whether THIS build records round video on this platform. Phase 3a
    /// turns it on for Apple.
    static let thisBuildRecords = false

    /// Its label and its pointer tooltip, as the catalogue will key them.
    /// No localized twin yet: nothing draws the button before Phase 3.
    static let labelKey = "Record video message"
    static let tooltipKey = "Record a video message"

    /// What the button needs to know besides the slot.
    nonisolated struct Inputs: Equatable, Sendable {
        var slot = ComposerSlot.Inputs()
        /// The chat's main composer, in a family or a direct chat.
        var familyOrDirectChat = true
        /// A released voice message is waiting out its Undo window: its row
        /// takes the field's place, and the button inside it goes too (S2.6).
        var undoWindow = false
        /// The server sends `max_round_video_ms` on `GET /families/mine`.
        var serverOffersRound = false
        /// The device has a camera.
        var hasCamera = false
        /// The web's encoder probe; true everywhere else.
        var encoderProbePasses = true
        /// This build records round video on this platform.
        var recordsRoundVideo = VideoDoor.thisBuildRecords

        /// S1.2's **round available**: all four.
        var roundAvailable: Bool {
            serverOffersRound && hasCamera && encoderProbePasses && recordsRoundVideo
        }
    }

    /// Shown when the slot is a microphone (rows 7–10) in a family or direct
    /// chat with round video available; dimmed, with the slot's sentence, in
    /// rows 7 and 8; usable in row 9, because the not-sent rule is about voice.
    static func of(_ inputs: Inputs) -> VideoDoor {
        guard inputs.familyOrDirectChat, !inputs.undoWindow, inputs.roundAvailable else {
            return .hidden
        }
        switch ComposerSlot.of(inputs.slot) {
        case .dimmed(.call): return .dimmed(.call)
        case .dimmed(.busy): return .dimmed(.busy)
        case .dimmed(.notSent), .microphone: return .shown
        default: return .hidden
        }
    }

    var labelKey: String? {
        switch self {
        case .hidden: nil
        case .dimmed, .shown: Self.labelKey
        }
    }

    var noticeKey: String? {
        if case .dimmed(let reason) = self { return reason.noticeKey }
        return nil
    }
}

// MARK: - The round video's arithmetic

/// S1.1's video length and S5's drawing test and sizes — for Phases 2 and 3,
/// held to the vectors from today.
nonisolated enum RoundVideo {
    /// Where a recording stops: `max_round_video_ms` − 500 ms. A limit
    /// shorter than the margin is 0, never a wrapped-around eternity.
    static func capMS(maxRoundVideoMS: UInt64) -> UInt64 {
        RecordGesture.saturatingSub(maxRoundVideoMS, RecordRules.roundCapMarginMS)
    }

    /// Where "10 seconds left" is shown: `max_round_video_ms` − 10 000 ms.
    static func warningMS(maxRoundVideoMS: UInt64) -> UInt64 {
        RecordGesture.saturatingSub(maxRoundVideoMS, RecordRules.roundWarningLeadMS)
    }

    /// How wide a window draws a received circle (S5.2).
    nonisolated enum WidthClass: Equatable, Sendable {
        /// A compact horizontal size class.
        case compact
        /// Everything else.
        case regular
    }

    /// A received circle's diameter, in points — a recommendation nothing on
    /// the wire carries.
    static func diameter(_ width: WidthClass) -> UInt32 {
        switch width {
        case .compact: RecordRules.roundDiameterCompact
        case .regular: RecordRules.roundDiameterRegular
        }
    }

    /// What `isRound` reads of one attachment.
    nonisolated struct AttachmentFlags: Equatable, Sendable {
        /// The attachment's `kind`, as the wire spells it.
        let kind: String
        /// Its `round` — absent on the wire is false.
        let round: Bool
    }

    /// The drawing test (S5.1): exactly one attachment, `kind = video`,
    /// carrying `round: true`, and no body. The body is compared EXACTLY:
    /// the server stores a blank attachment body as `""`, so no platform's
    /// idea of whitespace can make two clients disagree.
    static func isRound(body: String, attachments: [AttachmentFlags]) -> Bool {
        guard body.isEmpty, attachments.count == 1, let only = attachments.first else { return false }
        return only.kind == "video" && only.round
    }
}
