//
//  RecordGesture.swift
//  FamilyConnect
//
//  The slot's voice recording as a reducer: an activation of the slot or a
//  menu, Stop, Delete, the length limit or an interruption goes in with the
//  state; the next state and what to do come out (#79,
//  docs/audio-video-messages-2026-10-04.md, S2.1, S2.2, S2.5).
//
//  A PORT of `hold_step` in `web/text/src/record.rs`, checked case by case
//  against `record-vectors.json` (RecordVectorTests) — see ComposerSlot.swift
//  for why the reference is an oracle and not a document to read. Every
//  branch below is the reference's, in its order; where Swift needs a
//  different spelling (saturating arithmetic on `UInt64`, a `switch` for a
//  `match`) the meaning is the same for every input the reference takes.
//
//  REVISED 2026-10-06: THERE IS NO HOLD. The first draft made the microphone
//  a walkie-talkie on touch — hold to talk, slide to cancel or to lock, let
//  go to send after a five-second Undo window. The owner removed it after
//  testing it on his iPhone, with everything that existed only for it. The
//  microphone does ONE thing: its activation starts a hands-free recording,
//  which the slot's Send arrow sends, Stop keeps for review and Delete
//  deletes. A long press is not a gesture: nothing records and nothing opens
//  while a finger is down, and the press is the button's ordinary tap when it
//  lifts inside, however long it was held. The names (`HoldState`,
//  `HoldEvent`, `HoldEffect`, `HoldConstants`) are the reference's, kept so
//  that the four ports' types keep theirs.
//
//  It is only the DECISION. Opening the microphone, staging, parking,
//  sending, speaking and buzzing are the composer's (VoiceComposer), which
//  turns each `HoldEffect` into the thing it names, in the order given.
//
//  Two clocks, as the reference says: `atMS` is one monotonic clock for the
//  activation guard; `recordedMS` is the recorder's own clock — the one the
//  person watches, which with VoiceOver running starts only once "Recording"
//  has been spoken.
//

import Foundation

nonisolated enum RecordGesture {

    // MARK: - The numbers

    /// The numbers `step` decides by: S1.1's.
    nonisolated struct HoldConstants: Equatable, Sendable {
        var shortestRecordingMS: UInt64
        var activationGuardMS: UInt64
        var deleteAsksFromMS: UInt64

        /// S1.1's numbers — the reference's `Default`.
        static let standard = HoldConstants(
            shortestRecordingMS: RecordRules.shortestRecordingMS,
            activationGuardMS: RecordRules.activationGuardMS,
            deleteAsksFromMS: RecordRules.deleteAsksFromMS)
    }

    // MARK: - What a decision reads

    /// The microphone permission, as the platform reads it.
    nonisolated enum Permission: Equatable, Sendable {
        case granted
        /// Never asked: activation raises the system's prompt.
        case notAsked
        /// Refused: the denial notice, with Open Settings.
        case denied
    }

    /// The facts a decision reads at the moment it is made. The defaults are
    /// a granted microphone and nothing in the way.
    nonisolated struct Situation: Equatable, Sendable {
        var permission: Permission = .granted
        /// The dimmed row the microphone is in (rows 7–9), if any.
        var blocked: ComposerSlot.Dimmed?
    }

    /// Where a recording waiting on the permission prompt came from. Either
    /// way Allow starts recording: the activation meant "record".
    nonisolated enum Source: Equatable, Sendable {
        /// The slot's microphone: a tap, a click, Enter or Space, a screen
        /// reader's activation. Its start is guarded.
        case tap
        /// Record Voice Message from a menu, or ⌥⌘R: beside the draft if
        /// there is one, and never guarded.
        case menu
    }

    // MARK: - The state

    /// Where the slot's voice recording is.
    nonisolated enum Phase: Equatable, Sendable {
        /// Nothing recording.
        case idle
        /// Recording, hands-free: the recording row.
        case handsFree(besideDraft: Bool)
        /// Stopped by Delete at 10 s or more; "Delete this recording?".
        case askingDelete(recordedMS: UInt64)
        /// The system's microphone prompt is up.
        case awaitingPermission(source: Source, besideDraft: Bool)
    }

    /// The reducer's whole state: the phase and the activation guard.
    nonisolated struct HoldState: Equatable, Sendable {
        var phase: Phase = .idle
        /// Activation of the slot before this moment is ignored whole.
        var guardUntilMS: UInt64 = 0

        /// What `ComposerSlot.of` is told about this recording.
        var recording: ComposerSlot.Recording {
            switch phase {
            case .handsFree(besideDraft: false): .handsFree
            case .handsFree(besideDraft: true): .handsFreeBesideDraft
            default: .none
            }
        }

        /// The slot ignores activation at `atMS` — a row-5 Send included,
        /// which the composer asks here before sending: a double tap on the
        /// Stop square must not send what it staged.
        func guarded(atMS: UInt64) -> Bool {
            atMS < guardUntilMS
        }
    }

    // MARK: - What happened

    /// What happened. Every activation of the slot — the microphone, the
    /// Send arrow, the Stop square — is `activate`, whatever activated it:
    /// the button's own completed tap (a press that lifts inside, however
    /// long it was held), a click, Return, a screen reader's. Nothing is
    /// said for a press going down or being held: a long press is not a
    /// gesture.
    nonisolated enum HoldEvent: Equatable, Sendable {
        /// The recorder stopped itself at five minutes.
        case cap(atMS: UInt64)
        /// Anything but the person stopped it (S4).
        case interruption(atMS: UInt64, recordedMS: UInt64)
        /// The slot was activated.
        case activate(atMS: UInt64, situation: Situation, recordedMS: UInt64)
        /// Record Voice Message from a menu, or ⌥⌘R. During a recording it
        /// STOPS it into review: a shortcut never sends.
        case record(atMS: UInt64, besideDraft: Bool, situation: Situation, recordedMS: UInt64)
        /// Stop, Esc, VoiceOver's escape, Magic Tap, "Stop and listen first".
        case stop(atMS: UInt64, recordedMS: UInt64)
        /// The recording row's Delete.
        case delete(atMS: UInt64, recordedMS: UInt64)
        /// "Delete this recording?" answered.
        case answer(atMS: UInt64, delete: Bool)
        /// The system's microphone prompt answered.
        case permissionAnswer(atMS: UInt64, granted: Bool)
        /// Any other action — a character typed or deleted, a paste, a
        /// pick, a sticker: when idle it lifts the activation guard (S1.1:
        /// typing is never guarded).
        case otherAction(atMS: UInt64)
        /// The slot's own Send or Save just emptied the composer.
        case emptied(atMS: UInt64)

        var atMS: UInt64 {
            switch self {
            case .cap(let at), .interruption(let at, _), .activate(let at, _, _),
                 .record(let at, _, _, _), .stop(let at, _), .delete(let at, _),
                 .answer(let at, _), .permissionAnswer(let at, _), .otherAction(let at),
                 .emptied(let at):
                at
            }
        }
    }

    // MARK: - What to do

    /// S2.9's haptics — phones only; a composer drops them on an iPad.
    nonisolated enum Haptic: Equatable, Sendable {
        /// Recording starts: `.impact(weight: .light)`.
        case light
        /// Sent: `.success`.
        case success
        /// Too short; deleted: `.warning`.
        case warning
    }

    /// A line the composer SHOWS — in the row or its notice line.
    nonisolated enum Hint: Equatable, Sendable {
        case stoppedAtFiveMinutes
        case tooShort

        var key: String {
            switch self {
            case .stoppedAtFiveMinutes: "Recording stopped at five minutes."
            case .tooShort: "That recording was too short."
            }
        }

        var text: String {
            switch self {
            case .stoppedAtFiveMinutes: String(localized: "Recording stopped at five minutes.")
            case .tooShort: String(localized: "That recording was too short.")
            }
        }
    }

    /// What is SPOKEN, politely, to a screen reader (S6) — state changes
    /// only, never the ticking clock.
    nonisolated enum Announcement: Equatable, Sendable {
        case recording
        case recordingDeleted
        case voiceMessageSent
        /// "Ready to review, 0:42".
        case readyToReview(recordedMS: UInt64)
        case tooShort
        case stoppedAtFiveMinutes

        /// The catalogue key; `%@` is `readyToReview`'s length.
        var key: String {
            switch self {
            case .recording: "Recording"
            case .recordingDeleted: "Recording deleted"
            case .voiceMessageSent: "Voice message sent"
            case .readyToReview: "Ready to review, %@"
            case .tooShort: "That recording was too short."
            case .stoppedAtFiveMinutes: "Recording stopped at five minutes."
            }
        }

        var text: String {
            switch self {
            case .recording: String(localized: "Recording")
            case .recordingDeleted: String(localized: "Recording deleted")
            case .voiceMessageSent: String(localized: "Voice message sent")
            case .readyToReview(let recordedMS):
                String(localized: "Ready to review, \(AudioRecorder.timeLabel(Double(recordedMS) / 1000))")
            case .tooShort: String(localized: "That recording was too short.")
            case .stoppedAtFiveMinutes: String(localized: "Recording stopped at five minutes.")
            }
        }
    }

    /// What the composer does, in the order given: the thing first, then
    /// what is shown, said and felt.
    nonisolated enum HoldEffect: Equatable, Sendable {
        /// Open the microphone: the recording row. Keep the screen awake;
        /// pause anything playing; move focus to the slot.
        case start
        /// Stop the recording if it runs, and delete it.
        case delete
        /// Stop it and hand it to the outbox now, with the primed reply.
        case send
        /// Stop it and stage it in the chip above the field (S2.7).
        case review
        /// Stop it and keep it as the chat's "not sent" row (S2.8).
        case park
        /// Stop it and ask "Delete this recording?".
        case askDelete
        /// Raise the system's microphone prompt.
        case askPermission
        /// The denial notice, with Open Settings.
        case denied
        /// Say the dimmed row's sentence instead of acting.
        case explain(ComposerSlot.Dimmed)
        case hint(Hint)
        case announce(Announcement)
        case haptic(Haptic)
    }

    // MARK: - The step

    /// One step of the slot's voice recording: the state and an event in,
    /// the next state and what to do out. The reference's rules, in its
    /// order — see `hold_step` for the prose.
    static func step(
        _ state: HoldState,
        _ event: HoldEvent,
        _ c: HoldConstants
    ) -> (HoldState, [HoldEffect]) {
        var s = state
        var fx: [HoldEffect] = []
        let at = event.atMS

        switch event {
        case .cap:
            if isRecording(s) {
                s.phase = .idle
                fx.append(.review)
                fx.append(.hint(.stoppedAtFiveMinutes))
                fx.append(.announce(.stoppedAtFiveMinutes))
            }

        case let .interruption(_, recordedMS):
            switch s.phase {
            case .handsFree:
                s.phase = .idle
                interrupted(&fx, recordedMS: recordedMS, c)
            case .askingDelete:
                // The question's answer never came: kept, never lost, never sent.
                s.phase = .idle
                fx.append(.park)
            case .awaitingPermission:
                s.phase = .idle
            case .idle:
                break
            }

        case let .activate(_, situation, recordedMS):
            if !s.guarded(atMS: at) {
                switch s.phase {
                case .idle:
                    activateMicrophone(&s, &fx, at: at, situation, source: .tap, besideDraft: false, c)
                case let .handsFree(besideDraft):
                    s.phase = .idle
                    s.guardUntilMS = saturatingAdd(at, c.activationGuardMS)
                    if besideDraft {
                        stopIntoReview(&fx, recordedMS: recordedMS, c)
                    } else {
                        sendNow(&fx, recordedMS: recordedMS, c)
                    }
                case .askingDelete, .awaitingPermission:
                    break
                }
            }

        case let .record(_, besideDraft, situation, recordedMS):
            switch s.phase {
            case .idle:
                activateMicrophone(&s, &fx, at: at, situation, source: .menu, besideDraft: besideDraft, c)
            case .handsFree:
                s.phase = .idle
                stopIntoReview(&fx, recordedMS: recordedMS, c)
            case .askingDelete, .awaitingPermission:
                break
            }

        case let .stop(_, recordedMS):
            if isRecording(s) {
                s.phase = .idle
                stopIntoReview(&fx, recordedMS: recordedMS, c)
            }

        case let .delete(_, recordedMS):
            if isRecording(s) {
                if recordedMS < c.deleteAsksFromMS {
                    s.phase = .idle
                    deleted(&fx)
                } else {
                    s.phase = .askingDelete(recordedMS: recordedMS)
                    fx.append(.askDelete)
                }
            }

        case let .answer(_, delete):
            if case let .askingDelete(recordedMS) = s.phase {
                s.phase = .idle
                if delete {
                    deleted(&fx)
                } else {
                    fx.append(.review)
                    fx.append(.announce(.readyToReview(recordedMS: recordedMS)))
                }
            }

        case let .permissionAnswer(_, granted):
            if case let .awaitingPermission(source, besideDraft) = s.phase {
                s.phase = .idle
                if granted {
                    startHandsFree(&s, &fx, at: at, source: source, besideDraft: besideDraft, c)
                } else {
                    fx.append(.denied)
                }
            }

        case .otherAction:
            // The person changed the composer — typed, pasted, deleted,
            // staged: the next activation is a decision of its own, not the
            // second half of a double tap, so it lifts the guard. While a
            // recording runs the field is behind the row and nothing in it is
            // the person's to change, so the guard on its Send stays.
            if s.phase == .idle {
                s.guardUntilMS = 0
            }

        case .emptied:
            s.guardUntilMS = saturatingAdd(at, c.activationGuardMS)
        }
        return (s, fx)
    }

    // MARK: - The reference's helpers, in its order

    /// A voice recording runs.
    private static func isRecording(_ s: HoldState) -> Bool {
        if case .handsFree = s.phase { return true }
        return false
    }

    /// What stands between the microphone's activation and a recording, in
    /// this order: a dimmed row's sentence, the denial notice, the prompt.
    private static func refusal(_ situation: Situation) -> HoldEffect? {
        if let reason = situation.blocked {
            return .explain(reason)
        }
        switch situation.permission {
        case .granted: return nil
        case .denied: return .denied
        case .notAsked: return .askPermission
        }
    }

    /// The microphone's completed activation, from the slot or a menu.
    private static func activateMicrophone(
        _ s: inout HoldState,
        _ fx: inout [HoldEffect],
        at: UInt64,
        _ situation: Situation,
        source: Source,
        besideDraft: Bool,
        _ c: HoldConstants
    ) {
        switch refusal(situation) {
        case .askPermission?:
            s.phase = .awaitingPermission(source: source, besideDraft: besideDraft)
            fx.append(.askPermission)
        case let effect?:
            s.phase = .idle
            fx.append(effect)
        case nil:
            startHandsFree(&s, &fx, at: at, source: source, besideDraft: besideDraft, c)
        }
    }

    /// A hands-free recording starts. Only the slot's OWN activation guards it.
    private static func startHandsFree(
        _ s: inout HoldState,
        _ fx: inout [HoldEffect],
        at: UInt64,
        source: Source,
        besideDraft: Bool,
        _ c: HoldConstants
    ) {
        s.phase = .handsFree(besideDraft: besideDraft)
        if source == .tap {
            s.guardUntilMS = saturatingAdd(at, c.activationGuardMS)
        }
        fx.append(.start)
        fx.append(.announce(.recording))
        fx.append(.haptic(.light))
    }

    /// The person deleted it: said and felt.
    private static func deleted(_ fx: inout [HoldEffect]) {
        fx.append(.delete)
        fx.append(.announce(.recordingDeleted))
        fx.append(.haptic(.warning))
    }

    /// Under a second: discarded, with the sentence shown and said.
    private static func tooShort(_ fx: inout [HoldEffect]) {
        fx.append(.delete)
        fx.append(.hint(.tooShort))
        fx.append(.announce(.tooShort))
        fx.append(.haptic(.warning))
    }

    /// The Send arrow (S2.5).
    private static func sendNow(_ fx: inout [HoldEffect], recordedMS: UInt64, _ c: HoldConstants) {
        if recordedMS < c.shortestRecordingMS {
            tooShort(&fx)
        } else {
            fx.append(.send)
            fx.append(.announce(.voiceMessageSent))
            fx.append(.haptic(.success))
        }
    }

    /// Stop, the Stop square, the shortcut (S2.5).
    private static func stopIntoReview(_ fx: inout [HoldEffect], recordedMS: UInt64, _ c: HoldConstants) {
        if recordedMS < c.shortestRecordingMS {
            tooShort(&fx)
        } else {
            fx.append(.review)
            fx.append(.announce(.readyToReview(recordedMS: recordedMS)))
        }
    }

    /// Stopped by something other than the person (S2.8, S4): kept as not
    /// sent, or, with nothing worth keeping, deleted without a word. An
    /// interruption never sends.
    private static func interrupted(_ fx: inout [HoldEffect], recordedMS: UInt64, _ c: HoldConstants) {
        if recordedMS < c.shortestRecordingMS {
            fx.append(.delete)
        } else {
            fx.append(.park)
        }
    }

    // MARK: - u64 arithmetic, as the reference does it

    /// `a.saturating_add(b)` on u64.
    static func saturatingAdd(_ a: UInt64, _ b: UInt64) -> UInt64 {
        let (sum, overflow) = a.addingReportingOverflow(b)
        return overflow ? .max : sum
    }

    /// `a.saturating_sub(b)` on u64.
    static func saturatingSub(_ a: UInt64, _ b: UInt64) -> UInt64 {
        a > b ? a - b : 0
    }
}
