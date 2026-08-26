//
//  CallRecordText.swift
//  FamilyConnect
//
//  The words a call record is drawn with (docs/protocol.md, "Voice
//  calls"). The server writes an English placeholder into the body for
//  clients that predate calls; this client knows the `call` object and
//  never shows that body — it draws its OWN wording from the outcome, the
//  duration, and which side of the call the reader was on.
//
//  Pure and shared by the bubble, the chat-list preview and the Mac's
//  local notification, so the three can never say different things about
//  the same call.
//

import Foundation

nonisolated enum CallRecordText {

    /// The one line a record is drawn with. `isMine` is whether the READER
    /// placed the call — the record's sender is the caller.
    static func label(_ call: CallDTO, isMine: Bool) -> String {
        label(outcome: call.outcome, durationSecs: call.durationSecs, isMine: isMine)
    }

    static func label(outcome: String, durationSecs: Int?, isMine: Bool) -> String {
        switch outcome {
        case CallDTO.Outcome.completed:
            if let durationSecs {
                return String(localized: "Voice call · \(duration(durationSecs))")
            }
            return String(localized: "Voice call")
        case CallDTO.Outcome.missed:
            // "No answer" is what the caller sees; the callee missed it.
            return isMine ? String(localized: "No answer") : String(localized: "Missed voice call")
        case CallDTO.Outcome.declined:
            return isMine ? String(localized: "Voice call declined") : String(localized: "Declined voice call")
        case CallDTO.Outcome.failed:
            if let durationSecs {
                return String(localized: "Call failed · \(duration(durationSecs))")
            }
            return String(localized: "Call failed")
        default:
            // An outcome this build does not know: still a call, and the
            // render floor says never draw nothing.
            return String(localized: "Voice call")
        }
    }

    /// `3:42`, or `1:03:42` past an hour — the same shape the in-call
    /// timer counts in.
    static func duration(_ seconds: Int) -> String {
        let whole = max(0, seconds)
        let hours = whole / 3600
        let minutes = (whole % 3600) / 60
        let secs = whole % 60
        if hours > 0 {
            return String(format: "%d:%02d:%02d", hours, minutes, secs)
        }
        return String(format: "%d:%02d", minutes, secs)
    }
}

/// The status line of the in-call screen — pure, so the wording is
/// testable without a view.
nonisolated enum CallStatusText {
    static func line(phase: CallManager.Phase, direction: CallManager.Direction?, elapsed: Int) -> String {
        switch phase {
        case .idle:
            return ""
        case .outgoing(let ringing):
            return ringing ? String(localized: "Ringing…") : String(localized: "Calling…")
        case .incoming:
            return String(localized: "Incoming call")
        case .connecting:
            return String(localized: "Connecting…")
        case .active:
            return CallRecordText.duration(elapsed)
        case .ended(let reason):
            return ended(reason, direction: direction)
        }
    }

    static func ended(_ reason: CallEndReason, direction: CallManager.Direction?) -> String {
        switch reason {
        case .hangup, .cancel:
            return String(localized: "Call ended")
        case .decline:
            return direction == .outgoing ? String(localized: "Declined") : String(localized: "Call ended")
        case .timeout:
            return direction == .outgoing ? String(localized: "No answer") : String(localized: "Missed voice call")
        case .failed:
            return String(localized: "Call failed")
        case .answeredElsewhere:
            return String(localized: "Answered on another device")
        case .busy:
            return String(localized: "Busy")
        case .unreachable, .unavailable:
            return String(localized: "Unavailable")
        case .microphoneDenied:
            return String(localized: "Microphone access is needed for calls.")
        }
    }
}
