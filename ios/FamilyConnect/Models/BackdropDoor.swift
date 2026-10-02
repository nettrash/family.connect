//
//  BackdropDoor.swift
//  FamilyConnect
//
//  What "Draw a backdrop" is on one event: there or not, and whether the
//  author is asked the consent question first (docs/protocol.md, "Board"
//  and "Consenting to the assistant").
//
//  The backdrop is drawn from the event's TITLE, which is words the author
//  wrote going to the model — exactly what the consent question is about.
//  The server refuses it with `assistant_consent_required` without that
//  consent, as it refuses a `/draw`, so the client asks before it draws,
//  the way the composer asks before it sends. One rule for the phone's note
//  sheet and the Mac's note menu, so the two cannot come to disagree, and a
//  test can pin every row with no view on screen. `StickerDoor` is the same
//  shape for the same reason.
//

import Foundation

nonisolated enum BackdropDoor: Equatable {
    /// No control, and no hint beside it.
    case absent
    /// One tap draws.
    case open
    /// The author has not agreed that their words may go to the model.
    /// The control is there; the tap raises the consent question first and
    /// draws when it is answered yes.
    case asksFirst

    /// `PictureRequestHint.offersBackdrop` decides whether the control
    /// exists at all — an event, saved, its author, a server that can draw
    /// — and consent decides what a tap does.
    ///
    /// A server that can draw but names no processor gets NO control, for
    /// the reason it gets no `/draw` in the composer: a consent screen with
    /// a hole where the recipient goes is not consent, so there is no
    /// honest way to ask and nothing may be sent
    /// (`AssistantConsent.isAvailable`).
    static func of(
        isEvent: Bool,
        isSaved: Bool,
        isAuthor: Bool,
        serverCanDraw: Bool,
        processor: String?,
        agreedAt: Date?
    ) -> BackdropDoor {
        guard PictureRequestHint.offersBackdrop(
            isEvent: isEvent, isSaved: isSaved,
            isAuthor: isAuthor, serverCanDraw: serverCanDraw)
        else { return .absent }
        guard AssistantConsent.isAvailable(processor: processor) else { return .absent }
        return agreedAt == nil ? .asksFirst : .open
    }

    /// Whether the control — and the hint beside it — is on screen.
    var isOffered: Bool { self != .absent }
}

/// The backdrops being drawn right now, by note id: one request per note
/// at a time. The request can take up to two minutes — up to three
/// provider calls in a row, and a consent question in front of it
/// (docs/protocol.md, "Board") — and every one it makes costs the family a
/// picture, while the second answer would only replace the first. The
/// phone's note sheet holds one `drawing` flag for its one note; the Mac's
/// board shows every note's menu at once, so it keeps this set instead,
/// and a test pins it with no view on screen.
nonisolated struct BackdropDraws: Equatable {
    private var inFlight: Set<Int64> = []

    /// Whether a backdrop for this note is being drawn now.
    func isDrawing(_ noteID: Int64) -> Bool { inFlight.contains(noteID) }

    /// Start one. False — and nothing may be sent — while one for the same
    /// note is still out.
    mutating func begin(_ noteID: Int64) -> Bool { inFlight.insert(noteID).inserted }

    /// The answer came, whatever it was: the control is a control again.
    mutating func end(_ noteID: Int64) { inFlight.remove(noteID) }
}
