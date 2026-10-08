//
//  VoiceRecordingDoor.swift
//  FamilyConnect
//
//  The question a composer asks before the "not sent" row's Send (#79,
//  Phase 0 — docs/audio-video-messages-2026-10-04.md, S2.7, S2.8). Pure, so
//  both composers ask it the same way and the answer can be pinned without a
//  composer on screen.
//
//  And the question a composer asks as its chat is left: do the voice notes
//  still in review become "not sent" (S2.8, S4)?
//
//  Phase 0 asked a second question here — may "Record Audio" start — and
//  Phase 1 replaced it with the shared slot rules (`ComposerSlot.Inputs
//  .blocked`, checked against `record-vectors.json`), whose rows 7 to 9 are
//  that refusal and one more. Once the Mac moved to the slot too, nothing
//  asked it any more, and it went.
//

import Foundation

/// What the "not sent" row's Send does with its own caption — the same
/// question every other send door asks, asked of THAT caption rather than of
/// whatever happens to be in the field (protocol.md, "Consenting to the
/// assistant"; S2.7: "the consent question only if the words mention @ai").
nonisolated enum NotSentSendDoor: Equatable, Sendable {
    case send
    /// The words would reach the model and nobody has agreed yet: the
    /// consent sheet, holding the note, which goes on a yes.
    case asksConsent
    /// The words would reach an assistant this server will not name:
    /// nothing goes, and the composer says why.
    case withheld

    static func of(
        caption: String,
        chatKind: String?,
        processor: String?,
        agreedAt: Date?,
        hasAssistant: Bool
    ) -> NotSentSendDoor {
        if AssistantConsent.isRequired(
            chatKind: chatKind, body: caption, processor: processor, agreedAt: agreedAt) {
            return .asksConsent
        }
        if AssistantConsent.isWithheldFromAnUnnamedAssistant(
            chatKind: chatKind, body: caption, hasAssistant: hasAssistant, processor: processor) {
            return .withheld
        }
        return .send
    }
}

/// Leaving the chat: do the voice notes still in review become "not sent"
/// rows (S2.8; S4's "Leaving the chat" row), or go with a session that has
/// ended? A composer asks it in its `onDisappear`, which runs only when the
/// chat really goes — a call's full-screen cover never triggers it on the
/// view underneath (checked on iOS 18.6, 26.0 and 27.0 simulators).
nonisolated enum LeavingChatDoor {
    /// `callInProgress` is NOT part of the answer, and is an input so that
    /// a test can pin exactly that. A call placed from outside the app —
    /// Siri, Phone's Recents, a contact card — opens its chat
    /// (`ChatListView.place(callTo:video:)`) in the same main-actor turn that
    /// moves the call manager out of idle, so the chat being left already
    /// sees a call going. Answering "keep them in review" then lost the
    /// notes: the composer's state dies with the view, and its file waited
    /// in tmp for the next launch's sweep. "A recording cannot be made
    /// again" (S2.8).
    ///
    /// After a sign-out or a leave the store has just been emptied for the
    /// session that ended, and what this composer holds goes with it.
    static func parksReviewNotes(signedIn: Bool, callInProgress: Bool) -> Bool {
        signedIn
    }
}
