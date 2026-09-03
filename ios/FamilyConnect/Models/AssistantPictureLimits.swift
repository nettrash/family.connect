//
//  AssistantPictureLimits.swift
//  FamilyConnect
//
//  The bounds on a photograph shown to the assistant (docs/protocol.md,
//  "Pictures", and the Limits table).
//
//  All three are FIXED — not operator-configurable, not advertised on the
//  wire, and not negotiable — which is precisely why a client may hold
//  them. They are here rather than spelled into a composer because the
//  numbers decide a SENTENCE somebody reads before pixels leave their
//  house, and a sentence that disagreed with the server would be worse
//  than no sentence at all.
//
//  What the server actually does with them, so the copy above this file's
//  callers can be checked against it:
//
//  - at most FOUR photos, off the ONE message being answered, in the
//    sender's order;
//  - each as the downscaled PREVIEW when the client uploaded one — which
//    on Apple is always, since `MediaPrep.preparePhoto` writes a preview
//    for every photo it makes — and only otherwise the stored original;
//  - a photo still over FIVE MiB after that choice is not sent, and the
//    assistant is TOLD one was left out;
//  - and only JPEG or PNG bytes travel. An iPhone's HEIC original is a
//    photograph no chat deployment reads, so it counts as left out and is
//    said out loud in the same sentence.
//
//  The last two almost never fire on this platform and that is not an
//  accident: every `kind=photo` this app uploads is a re-encoded JPEG
//  (MediaPrep is the only door), and a 600-pixel preview JPEG is three
//  orders of magnitude under the ceiling. They are APPLIED anyway, by the
//  two composers, and that is the point of holding them: "almost never" is
//  not "never" — a preview is best-effort, and a photo whose preview never
//  got made travels as its original — and a client that knows the rule can
//  say which of a member's photos will actually be looked at. A rule the
//  file states and the code does not apply is worse than no rule: it makes
//  the composer promise, in as many words, that a photograph the server
//  will leave out "goes to the model your server is set up to use".
//
//  Android counterpart: the same three constants beside its own composer.
//

import Foundation

nonisolated enum AssistantPictureLimits {
    /// How many photos off one message reach the model. The rest are named
    /// to it, never silently dropped.
    static let maxPerQuestion = 4

    /// The largest photo that travels, after the preview has been
    /// preferred over the original. 5 MiB, exactly.
    static let maxBytes = 5 * 1024 * 1024

    /// The only two encodings a chat deployment reads.
    static let acceptedMIMETypes = ["image/jpeg", "image/png"]

    /// Would this attachment be shown to the model, or only named to it?
    ///
    /// Kind first, because it is the rule that actually turns people away:
    /// a video, a file, an audio note and a location never reach a model
    /// anywhere in this protocol, at any size, under any setting.
    ///
    /// `bytes` is what will be ON THE WIRE for this attachment — the
    /// PREVIEW's length where the client made one, since that is what the
    /// server prefers, and the original's only otherwise (see `wireBytes`).
    /// nil means "not known here", which is the honest answer for an
    /// attachment that arrived on somebody else's message: the size rule
    /// then goes unapplied rather than guessed at, and the two rules that
    /// ARE knowable still hold.
    static func isShownToModel(kind: String, mime: String, bytes: Int? = nil) -> Bool {
        guard kind == AttachmentDTO.Kind.photo,
              acceptedMIMETypes.contains(mime.lowercased())
        else { return false }
        guard let bytes else { return true }
        return bytes <= maxBytes
    }

    /// The media type this attachment will TRAVEL as.
    ///
    /// The server prefers the preview, and a preview is a JPEG by
    /// definition (protocol.md, "Photos, videos, audio, files and
    /// locations") — so the encoding a client should judge by is not
    /// always the one it uploaded. This is the whole of the difference,
    /// and it is why the HEIC rule almost never fires here.
    static func wireMIME(mime: String, hasPreview: Bool) -> String {
        hasPreview ? "image/jpeg" : mime
    }

    /// The number of bytes this attachment will TRAVEL as, by the same
    /// rule: the preview when there is one, the original otherwise.
    static func wireBytes(previewBytes: Int?, originalBytes: Int) -> Int {
        previewBytes ?? originalBytes
    }

    /// Of the attachments on ONE message, everything that CAN travel —
    /// kind, encoding and size, with no cap applied.
    ///
    /// Split from `shown` because the two omissions are different facts and
    /// a composer says different things about them: past the cap is
    /// something the member can fix by sending fewer, and a photograph that
    /// cannot travel at all is not.
    static func carried<T>(
        _ attachments: [T],
        kind: (T) -> String,
        mime: (T) -> String,
        bytes: (T) -> Int? = { _ in nil }
    ) -> [T] {
        attachments.filter { isShownToModel(kind: kind($0), mime: mime($0), bytes: bytes($0)) }
    }

    /// Of the attachments on ONE message, the ones the model will actually
    /// be shown — in the sender's order, and no more than four.
    ///
    /// The order is the message's own, which matters: "the first four"
    /// is only a meaningful thing to say to somebody if it means the first
    /// four they chose.
    static func shown<T>(
        _ attachments: [T],
        kind: (T) -> String,
        mime: (T) -> String,
        bytes: (T) -> Int? = { _ in nil }
    ) -> [T] {
        carried(attachments, kind: kind, mime: mime, bytes: bytes)
            .prefix(maxPerQuestion)
            .map { $0 }
    }
}

/// Which picture surfaces this client offers, and on what.
///
/// Two one-line rules, extracted from the two composers that ask them so
/// that iOS and macOS cannot drift on the answer and so a test can pin it
/// without a composer on screen. They decide what a member is INVITED to
/// do; what actually leaves the server is the server's decision and is
/// never inferred from these.
nonisolated enum AssistantSurfaces {

    /// May this composer offer to show the assistant a photograph?
    ///
    /// THREE facts, and all three have to hold. Two of them are here:
    /// the operator configured a deployment that can see, and this
    /// family's owner turned `ai_vision` on. The third is the member
    /// attaching the picture to this one question — that one is an act,
    /// not a setting, it is performed in the composer one send at a time,
    /// and it must never be given storage anywhere.
    ///
    /// `&&` and not `||`, which is the entire content of this function and
    /// the reason it is not written inline twice: three of its four rows
    /// must offer nothing.
    ///
    /// The assistant's OWN chat only. `@ai` in the family chat never
    /// carries a picture, at either `ai_history` setting, in any family,
    /// under any configuration — the photograph there is very often
    /// somebody else's, and the member typing the mention is in no
    /// position to consent on their behalf (protocol.md, "Pictures").
    static func offersPictureAttach(
        isAssistantChat: Bool, serverCanSee: Bool, familyAllows: Bool
    ) -> Bool {
        isAssistantChat && serverCanSee && familyAllows
    }

    /// May this composer offer `/draw`?
    ///
    /// The server's capability AND a spelling check. `assistant.draw`
    /// exists so a client can be certain the server means the same five
    /// characters its own grammar does; a server that named a different
    /// token is one this build cannot compose for, and a button that typed
    /// the wrong thing is exactly the affordance that silently does
    /// nothing. A server that named NO token predates pictures, where
    /// `images` is false anyway.
    ///
    /// No family switch, deliberately: what leaves on such a request is the
    /// words after the token and nothing else, which is a SMALLER
    /// disclosure than an ordinary text question already is.
    static func offersPictureRequests(serverCanDraw: Bool, serverToken: String?) -> Bool {
        serverCanDraw && (serverToken ?? AssistantMention.drawToken) == AssistantMention.drawToken
    }
}
