//
//  RecordGesture.swift
//  FamilyConnect
//
//  The slot's voice recording as a reducer: a press, a hold, a slide, a
//  release, a timer, the length limit or an interruption goes in with the
//  state; the next state and what to do come out (#79,
//  docs/audio-video-messages-2026-10-04.md, S2.1, S2.3, S2.5, S2.6).
//
//  A PORT of `hold_step` in `web/text/src/record.rs`, checked case by case
//  against `record-vectors.json` (RecordVectorTests) — see ComposerSlot.swift
//  for why the reference is an oracle and not a document to read. Every
//  branch below is the reference's, in its order; where Swift needs a
//  different spelling (saturating arithmetic on `UInt64`, a `switch` for a
//  `match`) the meaning is the same for every input the reference takes.
//
//  It is only the DECISION. Opening the microphone, staging, parking,
//  sending, speaking and buzzing are the composer's (VoiceComposer), which
//  turns each `HoldEffect` into the thing it names, in the order given.
//
//  Two clocks, as the reference says: `atMS` is one monotonic clock for the
//  press, the guard and the Undo window; `recordedMS` is the recorder's own
//  clock — the one the person watches, which with VoiceOver running starts
//  only once "Recording" has been spoken.
//

import Foundation

nonisolated enum RecordGesture {

    // MARK: - The numbers

    /// The numbers `step` decides by: S1.1's, with H for this system.
    nonisolated struct HoldConstants: Equatable, Sendable {
        /// H — `holdThresholdMS(systemLongPressMS:)`.
        var holdThresholdMS: UInt64
        var tapSlop: Double
        var lockDistance: Double
        var cancelArmDistance: Double
        var cancelDisarmDistance: Double
        var shortestRecordingMS: UInt64
        var undoWindowMS: UInt64
        var activationGuardMS: UInt64
        var deleteAsksFromMS: UInt64

        /// S1.1's numbers with H at its floor — the reference's `Default`.
        static let standard = HoldConstants(
            holdThresholdMS: RecordRules.minHoldThresholdMS,
            tapSlop: RecordRules.tapSlop,
            lockDistance: RecordRules.lockDistance,
            cancelArmDistance: RecordRules.cancelArmDistance,
            cancelDisarmDistance: RecordRules.cancelDisarmDistance,
            shortestRecordingMS: RecordRules.shortestRecordingMS,
            undoWindowMS: RecordRules.undoWindowMS,
            activationGuardMS: RecordRules.activationGuardMS,
            deleteAsksFromMS: RecordRules.deleteAsksFromMS)

        /// S1.1's numbers, with H for a system whose long press takes
        /// `systemLongPressMS`. iOS passes 500 — the long press the plan
        /// names for it, and the minimum duration of the recognizer that
        /// drives the hold.
        static func forSystem(longPressMS: UInt64) -> HoldConstants {
            var constants = standard
            constants.holdThresholdMS = RecordGesture.holdThresholdMS(systemLongPressMS: longPressMS)
            return constants
        }
    }

    /// H for a system whose own long press takes `systemLongPressMS`: never
    /// shorter than 500 ms, so a brush does not open the microphone, and
    /// never shorter than the person asked their system for.
    static func holdThresholdMS(systemLongPressMS: UInt64) -> UInt64 {
        max(systemLongPressMS, RecordRules.minHoldThresholdMS)
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
    /// a granted microphone, nothing in the way, no screen reader, a device
    /// that has released before, and Review Before Sending off.
    nonisolated struct Situation: Equatable, Sendable {
        var permission: Permission = .granted
        /// The dimmed row the microphone is in (rows 7–9), if any.
        var blocked: ComposerSlot.Dimmed?
        /// VoiceOver or Switch Control runs: a held release reviews.
        var assistive = false
        /// This device has not yet had its first held release taught.
        var firstRelease = false
        /// The per-device setting (S9): a held release reviews.
        var reviewBeforeSending = false
    }

    /// Where a recording waiting on the permission prompt came from.
    nonisolated enum Source: Equatable, Sendable {
        /// A completed tap, a click, Enter or Space, a screen reader's
        /// activation: Allow starts recording.
        case tap
        /// The hold threshold: a prompt a hold raised never records.
        case hold
        /// Record Voice Message from a menu, or ⌥⌘R: Allow starts recording,
        /// beside the draft if there is one.
        case menu
    }

    // MARK: - The state

    /// Where the slot's voice recording is.
    nonisolated enum Phase: Equatable, Sendable {
        /// Nothing pressed, nothing recording. A released note may still be
        /// waiting out its Undo window (`HoldState.undo`).
        case idle
        /// A finger or pen is down on the microphone, before H.
        case pressed(downAtMS: UInt64, downX: Double, downY: Double, rtl: Bool, mayHold: Bool)
        /// Recording, the finger still down: the hold row.
        case holding(downX: Double, downY: Double, rtl: Bool, armed: Bool)
        /// Recording, hands-free: the recording row.
        case handsFree(besideDraft: Bool)
        /// Stopped by Delete at 10 s or more; "Delete this recording?".
        case askingDelete(recordedMS: UInt64)
        /// The system's microphone prompt is up.
        case awaitingPermission(source: Source, besideDraft: Bool)
    }

    /// A released note waiting out its Undo window: nothing has left the
    /// device.
    nonisolated struct UndoNote: Equatable, Sendable {
        /// When the window runs out and the note goes to the outbox.
        var untilMS: UInt64
        var recordedMS: UInt64
    }

    /// The reducer's whole state. The Undo window is not a phase: the
    /// microphone is usable while it waits, and a new press must not end the
    /// window before it is a tap or a hold.
    nonisolated struct HoldState: Equatable, Sendable {
        var phase: Phase = .idle
        /// Activation of the slot before this moment is ignored whole.
        var guardUntilMS: UInt64 = 0
        var undo: UndoNote?

        /// What `ComposerSlot.of` is told about this recording.
        var recording: ComposerSlot.Recording {
            switch phase {
            case .holding: .held
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

    /// What happened. `down`, `move`, `up` and `systemCancel` are the
    /// MICROPHONE's touch and matter only from `.idle`; every other
    /// activation of the slot is `activate`. The lift of a press that went
    /// down on the microphone is always `up` — after a lock it does nothing.
    nonisolated enum HoldEvent: Equatable, Sendable {
        /// A press went down on the microphone. `canHold`: a finger or a
        /// Pencil; false for a pointer, where a press of any length clicks.
        case down(atMS: UInt64, x: Double, y: Double, canHold: Bool, rtl: Bool)
        /// It moved, in window coordinates.
        case move(atMS: UInt64, x: Double, y: Double)
        /// It lifted; `inside` is the button's own hit test. Its position
        /// counts as a last move first.
        case up(
            atMS: UInt64, x: Double, y: Double, inside: Bool,
            situation: Situation, recordedMS: UInt64, heard: Bool)
        /// The system cancelled the touch; `background`: the app went there.
        case systemCancel(atMS: UInt64, background: Bool, recordedMS: UInt64)
        /// Time passed — one at H, from the long-press recognizer, and one
        /// when the Undo window runs out.
        case tick(atMS: UInt64, situation: Situation)
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
        /// The Undo row's Undo.
        case undo(atMS: UInt64)
        /// Any other action — a character typed or deleted, a paste, a
        /// pick, a sticker: it ends the Undo window early, and when idle it
        /// lifts the activation guard (S1.1: typing is never guarded).
        case otherAction(atMS: UInt64)
        /// The slot's own Send or Save just emptied the composer.
        case emptied(atMS: UInt64)

        var atMS: UInt64 {
            switch self {
            case .down(let at, _, _, _, _), .move(let at, _, _), .up(let at, _, _, _, _, _, _),
                 .systemCancel(let at, _, _), .tick(let at, _), .cap(let at),
                 .interruption(let at, _), .activate(let at, _, _), .record(let at, _, _, _),
                 .stop(let at, _), .delete(let at, _), .answer(let at, _),
                 .permissionAnswer(let at, _), .undo(let at), .otherAction(let at),
                 .emptied(let at):
                at
            }
        }
    }

    // MARK: - What to do

    /// S2.9's haptics — phones only; a composer drops them on an iPad.
    nonisolated enum Haptic: Equatable, Sendable {
        /// Recording starts from a tap: `.impact(weight: .light)`.
        case light
        /// Recording starts at H: `.impact(weight: .medium)`.
        case medium
        /// Lock; cancel armed: `.selection`.
        case selection
        /// Sent: `.success`.
        case success
        /// Too short; deleted: `.warning`.
        case warning
    }

    /// A line the composer SHOWS — in the row or its notice line.
    nonisolated enum Hint: Equatable, Sendable {
        case stillRecording
        case nextTimeSends
        case nothingHeard
        case stoppedAtFiveMinutes
        case tooShort
        case canRecordNow

        var key: String {
            switch self {
            case .stillRecording: "Still recording. Tap Send when you're done."
            case .nextTimeSends: "Next time, letting go will send it."
            case .nothingHeard: "We didn't hear anything."
            case .stoppedAtFiveMinutes: "Recording stopped at five minutes."
            case .tooShort: "That recording was too short."
            case .canRecordNow: "You can record now."
            }
        }

        var text: String {
            switch self {
            case .stillRecording: String(localized: "Still recording. Tap Send when you're done.")
            case .nextTimeSends: String(localized: "Next time, letting go will send it.")
            case .nothingHeard: String(localized: "We didn't hear anything.")
            case .stoppedAtFiveMinutes: String(localized: "Recording stopped at five minutes.")
            case .tooShort: String(localized: "That recording was too short.")
            case .canRecordNow: String(localized: "You can record now.")
            }
        }
    }

    /// What is SPOKEN, politely, to a screen reader (S6) — state changes
    /// only, never the ticking clock.
    nonisolated enum Announcement: Equatable, Sendable {
        case recording
        case recordingLocked
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
            case .recordingLocked: "Recording locked"
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
            case .recordingLocked: String(localized: "Recording locked")
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
        /// Open the microphone: the hold row when `held`, the recording row
        /// otherwise. Keep the screen awake; pause anything playing.
        case start(held: Bool)
        /// Hands-free from here; the keyboard goes down, the lift is nothing.
        case lock
        /// Cancel armed: "Release to cancel".
        case arm
        /// "‹ Slide to cancel" again.
        case disarm
        /// Stop the recording if it runs, and delete it.
        case delete
        /// Stop it and hand it to the outbox now, with the primed reply.
        case send
        /// Stop it and stage it in the chip above the field (S2.7).
        case review
        /// Stop it and keep it as the chat's "not sent" row (S2.8).
        case park
        /// Stop it, park it marked "sending", show the Undo row (S2.6).
        case undoWindow
        /// The note in its Undo window goes to the outbox now.
        case undoSend
        /// The note in its Undo window goes to review instead.
        case undoReview
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
        /// Remember, on this device, that its first held release was taught.
        case firstReleaseDone
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

        // The Undo window runs out on every event's clock: a late timer must
        // not keep a note waiting, nor an Undo after five seconds take it back.
        if let note = s.undo, at >= note.untilMS {
            undoSend(&s, &fx)
        }

        switch event {
        case let .down(_, x, y, canHold, rtl):
            if s.phase == .idle && !s.guarded(atMS: at) {
                s.phase = .pressed(downAtMS: at, downX: x, downY: y, rtl: rtl, mayHold: canHold)
            }

        case let .move(_, x, y):
            switch s.phase {
            case let .pressed(downAtMS, downX, downY, rtl, true):
                let dx = x - downX
                let dy = y - downY
                if dx * dx + dy * dy > c.tapSlop * c.tapSlop {
                    s.phase = .pressed(downAtMS: downAtMS, downX: downX, downY: downY, rtl: rtl, mayHold: false)
                }
            case .holding:
                slide(&s, &fx, x: x, y: y, c)
            default:
                break
            }

        case let .up(_, x, y, inside, situation, recordedMS, heard):
            switch s.phase {
            case .pressed:
                if inside {
                    activateMicrophone(&s, &fx, at: at, situation, source: .tap, besideDraft: false, c)
                } else {
                    s.phase = .idle
                }
            case .holding:
                slide(&s, &fx, x: x, y: y, c)
                if case let .holding(_, _, _, armed) = s.phase {
                    s.guardUntilMS = saturatingAdd(at, c.activationGuardMS)
                    if armed {
                        s.phase = .idle
                        deleted(&fx)
                    } else {
                        release(&s, &fx, at: at, situation, recordedMS: recordedMS, heard: heard, c)
                    }
                }
            default:
                break
            }

        case let .systemCancel(_, background, recordedMS):
            switch s.phase {
            case .pressed:
                s.phase = .idle
            case .holding(_, _, _, true):
                s.phase = .idle
                deleted(&fx)
            case .holding where background:
                s.phase = .idle
                interrupted(&fx, recordedMS: recordedMS, c)
            case .holding:
                s.phase = .handsFree(besideDraft: false)
                fx.append(.lock)
                fx.append(.announce(.recordingLocked))
            default:
                break
            }

        case let .tick(_, situation):
            if case let .pressed(downAtMS, downX, downY, rtl, true) = s.phase,
               saturatingSub(at, downAtMS) >= c.holdThresholdMS {
                sendWaiting(&s, &fx)
                switch refusal(situation) {
                case .askPermission?:
                    s.phase = .awaitingPermission(source: .hold, besideDraft: false)
                    fx.append(.askPermission)
                case let effect?:
                    s.phase = .idle
                    fx.append(effect)
                case nil:
                    s.phase = .holding(downX: downX, downY: downY, rtl: rtl, armed: false)
                    s.guardUntilMS = saturatingAdd(at, c.activationGuardMS)
                    fx.append(.start(held: true))
                    fx.append(.announce(.recording))
                    fx.append(.haptic(.medium))
                }
            }

        case .cap:
            if isRecording(s) {
                s.phase = .idle
                fx.append(.review)
                fx.append(.hint(.stoppedAtFiveMinutes))
                fx.append(.announce(.stoppedAtFiveMinutes))
            }

        case let .interruption(_, recordedMS):
            if s.undo != nil {
                undoSend(&s, &fx)
            }
            switch s.phase {
            case .holding, .handsFree:
                s.phase = .idle
                interrupted(&fx, recordedMS: recordedMS, c)
            case .askingDelete:
                // The question's answer never came: kept, never lost, never sent.
                s.phase = .idle
                fx.append(.park)
            case .pressed, .awaitingPermission:
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
                default:
                    break
                }
            }

        case let .record(_, besideDraft, situation, recordedMS):
            switch s.phase {
            case .idle:
                activateMicrophone(&s, &fx, at: at, situation, source: .menu, besideDraft: besideDraft, c)
            case .holding, .handsFree:
                s.phase = .idle
                stopIntoReview(&fx, recordedMS: recordedMS, c)
            default:
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
                switch (granted, source) {
                case (false, _):
                    fx.append(.denied)
                case (true, .hold):
                    fx.append(.hint(.canRecordNow))
                case (true, _):
                    startHandsFree(&s, &fx, at: at, source: source, besideDraft: besideDraft, c)
                }
            }

        case .undo:
            if let note = s.undo {
                s.undo = nil
                fx.append(.undoReview)
                fx.append(.announce(.readyToReview(recordedMS: note.recordedMS)))
            }

        case .otherAction:
            if s.undo != nil {
                undoSend(&s, &fx)
            }
            // The person changed the composer — typed, pasted, deleted,
            // staged: the next press is a decision of its own, not the second
            // half of a double tap, so it lifts the guard. While a recording
            // runs the field is behind the row and nothing in it is the
            // person's to change, so the guard on its Send stays.
            if s.phase == .idle {
                s.guardUntilMS = 0
            }

        case .emptied:
            // A text Send is an action like any other: a released note still
            // waiting goes first. (The composer says otherAction when the
            // first character is typed, so this is a backstop.)
            if s.undo != nil {
                undoSend(&s, &fx)
            }
            s.guardUntilMS = saturatingAdd(at, c.activationGuardMS)
        }
        return (s, fx)
    }

    // MARK: - The reference's helpers, in its order

    /// A voice recording runs.
    private static func isRecording(_ s: HoldState) -> Bool {
        switch s.phase {
        case .holding, .handsFree: true
        default: false
        }
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

    /// The microphone's completed activation from a tap or a menu.
    private static func activateMicrophone(
        _ s: inout HoldState,
        _ fx: inout [HoldEffect],
        at: UInt64,
        _ situation: Situation,
        source: Source,
        besideDraft: Bool,
        _ c: HoldConstants
    ) {
        sendWaiting(&s, &fx)
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
        fx.append(.start(held: false))
        fx.append(.announce(.recording))
        fx.append(.haptic(.light))
    }

    /// A held finger moved (or lifted): arm or disarm cancel, then lock —
    /// never while cancel is armed.
    private static func slide(
        _ s: inout HoldState,
        _ fx: inout [HoldEffect],
        x: Double,
        y: Double,
        _ c: HoldConstants
    ) {
        guard case let .holding(downX, downY, rtl, armed) = s.phase else { return }
        let towardLeading = rtl ? x - downX : downX - x
        let up = downY - y
        var nowArmed = armed
        if !armed && towardLeading >= c.cancelArmDistance {
            nowArmed = true
            fx.append(.arm)
            fx.append(.haptic(.selection))
        } else if armed && towardLeading < c.cancelDisarmDistance {
            nowArmed = false
            fx.append(.disarm)
        }
        if !nowArmed && up >= c.lockDistance {
            s.phase = .handsFree(besideDraft: false)
            fx.append(.lock)
            fx.append(.announce(.recordingLocked))
            fx.append(.haptic(.selection))
        } else {
            s.phase = .holding(downX: downX, downY: downY, rtl: rtl, armed: nowArmed)
        }
    }

    /// A hold let go with cancel not armed (S2.3), in the plan's order.
    private static func release(
        _ s: inout HoldState,
        _ fx: inout [HoldEffect],
        at: UInt64,
        _ situation: Situation,
        recordedMS: UInt64,
        heard: Bool,
        _ c: HoldConstants
    ) {
        if recordedMS < c.shortestRecordingMS {
            s.phase = .handsFree(besideDraft: false)
            fx.append(.lock)
            fx.append(.hint(.stillRecording))
            return
        }
        s.phase = .idle
        if !heard {
            fx.append(.review)
            fx.append(.hint(.nothingHeard))
            fx.append(.announce(.readyToReview(recordedMS: recordedMS)))
            return
        }
        if situation.firstRelease || situation.reviewBeforeSending || situation.assistive {
            // Said only when it is true: not to somebody whose releases
            // always review.
            let teach = situation.firstRelease && !situation.reviewBeforeSending && !situation.assistive
            fx.append(.review)
            if teach {
                fx.append(.hint(.nextTimeSends))
            }
            fx.append(.announce(.readyToReview(recordedMS: recordedMS)))
            if teach {
                fx.append(.firstReleaseDone)
            }
            return
        }
        s.undo = UndoNote(untilMS: saturatingAdd(at, c.undoWindowMS), recordedMS: recordedMS)
        fx.append(.undoWindow)
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
    /// sent, or, with nothing worth keeping, deleted without a word.
    private static func interrupted(_ fx: inout [HoldEffect], recordedMS: UInt64, _ c: HoldConstants) {
        if recordedMS < c.shortestRecordingMS {
            fx.append(.delete)
        } else {
            fx.append(.park)
        }
    }

    /// The microphone's activation ends the Undo window by sending.
    private static func sendWaiting(_ s: inout HoldState, _ fx: inout [HoldEffect]) {
        if s.undo != nil {
            undoSend(&s, &fx)
        }
    }

    private static func undoSend(_ s: inout HoldState, _ fx: inout [HoldEffect]) {
        s.undo = nil
        fx.append(.undoSend)
        fx.append(.announce(.voiceMessageSent))
        fx.append(.haptic(.success))
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
