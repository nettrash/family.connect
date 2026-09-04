//
//  MentionPictureNotice.swift
//  FamilyConnect
//
//  What the FAMILY composer says when an `@ai` draft is about to carry a
//  photograph (docs/protocol.md, "Showing the assistant a picture from the
//  family chat" — "What a client's family-chat composer must say").
//
//  Until #56 a photo in the family chat never reached the assistant, so
//  the composer had nothing to say there. Now a photo on an `@ai` message,
//  or on the message that mention replies to, goes to the model under the
//  same two locks a private question needs — and the doctrine that put a
//  sentence above a staged photo in the assistant's own chat ("say what it
//  is about to do, at the moment it matters") puts one here for the same
//  reason: the switch was read once, on a settings screen, possibly by
//  somebody else; the photo is chosen now, in this composer.
//
//  One rule, extracted from the two Apple composers so iOS and macOS cannot
//  drift on it and so a test can pin every row without a composer on
//  screen. It mirrors the SERVER's counting exactly, which is what makes
//  the "4" it shows honest: the mention's own photos take their places
//  first, the quoted message's fill what is left, four across the two and
//  never four each; a photo that cannot travel at all (a HEIC with no
//  preview, one over 5 MiB) does not spend a place and is said out loud.
//  The bounds are `AssistantPictureLimits`' — held once, never copied.
//
//  Since the owner's THIRD switch (`ai_history_photos`, protocol.md,
//  "Recent photos from the family chat") the same rule also says, when
//  that switch is on and `ai_history` with it, that the chat's most recent
//  photos may go too — "up to N", where N is what the draft and the quote
//  left of the same four, because the server fills history LAST. The strip
//  cannot say which photos those are (the server decides from the
//  transcript it builds at the moment it answers), so it says "up to" and
//  never names one; and under that switch it shows for an `@ai` draft with
//  no photo of its own, the case #56's strip was absent for, since that is
//  precisely the mention on which every one of the four may be somebody
//  else's picture.
//
//  Android counterpart: `AiPictureNotice.forMention` beside its composer.
//

import Foundation

/// One attachment, reduced to the three facts the picture rule reads.
///
/// Reduced rather than passed as the attachment itself so the rule can be
/// asked about a photo that is STAGED (ours, a file on disk) and a photo
/// that is on a message somebody else sent (theirs, a DTO) with one
/// signature — and so a test can build either in a line.
nonisolated struct AssistantPictureCandidate: Equatable, Sendable {
    let kind: String
    /// What it will TRAVEL as: a preview is a JPEG by definition.
    let mime: String
    /// What it will travel as, in bytes — nil when this client cannot know,
    /// which is the honest answer for a preview on somebody else's message:
    /// the size rule then goes unapplied rather than guessed at.
    let bytes: Int?

    init(kind: String, mime: String, bytes: Int?) {
        self.kind = kind
        self.mime = mime
        self.bytes = bytes
    }

    /// An attachment as it sits on a message in the chat — the message a
    /// member is replying to, usually somebody else's.
    ///
    /// The server prefers the PREVIEW, and a preview's length is not on the
    /// wire — only the original's `size` is. So when a preview exists the
    /// encoding is judged as JPEG and the size goes unjudged (a 600-pixel
    /// preview is three orders of magnitude under the ceiling), and when
    /// none exists the original is judged by its own type and its stored
    /// size, which is exactly what the server will measure.
    init(attachment: AttachmentDTO) {
        kind = attachment.kind
        mime = AssistantPictureLimits.wireMIME(
            mime: attachment.mime, hasPreview: attachment.hasPreview)
        bytes = attachment.hasPreview ? nil : Int(clamping: attachment.size)
    }
}

/// The strip above the family composer while an `@ai` draft carries a
/// photograph — the counts, and the sentence they decide.
nonisolated struct MentionPictureNotice: Equatable, Sendable {
    /// Photographs the model will be shown off the draft itself. First,
    /// because they are the ones the member chose just now.
    let shownOnMention: Int
    /// Photographs it will be shown off the message being replied to —
    /// whatever the draft's own left of the four.
    let shownOnQuote: Int
    /// Photographs past the shared budget: named to the model, not shown.
    let extra: Int
    /// Photographs that cannot travel at all — over 5 MiB, or in an
    /// encoding no chat deployment reads: told, never shown.
    let unreadable: Int
    /// How many of the chat's most recent photographs may ALSO go — what
    /// the draft and the quote left of the four — or nil when the owner's
    /// `ai_history_photos` is not in effect (off, or `ai_history` off, so
    /// there is no transcript for it to widen). Zero is a real value: the
    /// switch is on and the member's own photos have spent the budget, so
    /// no recent photo travels and the sentence may still promise that no
    /// other photo in this chat does.
    let recentUpTo: Int?

    init(shownOnMention: Int, shownOnQuote: Int, extra: Int, unreadable: Int, recentUpTo: Int? = nil) {
        self.shownOnMention = shownOnMention
        self.shownOnQuote = shownOnQuote
        self.extra = extra
        self.unreadable = unreadable
        self.recentUpTo = recentUpTo
    }

    var shown: Int { shownOnMention + shownOnQuote }

    /// The notice for this draft, or nil because there is nothing to say.
    ///
    /// Nil is the common answer, and every reason for it is a fact the
    /// sentence would otherwise be lying about:
    ///
    /// - either lock shut — no vision deployment, or `ai_vision` off — and
    ///   nothing leaves; the family composer says nothing rather than
    ///   naming a lock a non-owner cannot open;
    /// - the draft does not mention the assistant, so the photo is an
    ///   ordinary attachment on an ordinary message;
    /// - the draft is `@ai /draw …` on a server that can draw: a picture
    ///   request sends the words after the token and nothing else — not
    ///   the photo — so there is no disclosure to announce;
    /// - no photograph on the draft and none on the message it replies to —
    ///   unless the owner's `ai_history_photos` is in effect, when a bare
    ///   `@ai` draft is exactly the one on which all four may be recent
    ///   photos nobody pointed at, and the strip shows for it.
    ///
    /// `quoted` is the attachment set of the message being replied to, in
    /// the sender's order, and [] when the draft replies to nothing.
    /// `familyHistory` and `familyHistoryPhotos` are the family's
    /// `ai_history` and `ai_history_photos`; the second is in effect only
    /// with the first, because without a transcript there is no photo in
    /// it to send (protocol.md, "Recent photos from the family chat").
    static func of(
        draft: String,
        staged: [AssistantPictureCandidate],
        quoted: [AssistantPictureCandidate],
        serverCanSee: Bool,
        familyAllows: Bool,
        familyHistory: Bool,
        familyHistoryPhotos: Bool,
        serverCanDraw: Bool
    ) -> MentionPictureNotice? {
        guard serverCanSee, familyAllows else { return nil }
        guard AssistantMention.mentions(draft) else { return nil }
        if serverCanDraw, AssistantMention.asksForPicture(draft) { return nil }
        // The third switch does nothing on its own: it needs both locks
        // above (checked already), and a transcript to take photos from.
        let recentPhotos = familyHistory && familyHistoryPhotos
        let onMention = staged.filter { $0.kind == AttachmentDTO.Kind.photo }
        let onQuote = quoted.filter { $0.kind == AttachmentDTO.Kind.photo }
        guard !onMention.isEmpty || !onQuote.isEmpty || recentPhotos else { return nil }

        // The server's own rule in the server's own order: what can travel
        // at all, then the mention's first and the quote's in what is left
        // (handlers_ai.rs, `vision_images` called twice with one budget),
        // and the transcript's newest fill only what remains — the same
        // four, never four more.
        let carriedOnMention = AssistantPictureLimits.carried(
            onMention, kind: \.kind, mime: \.mime, bytes: \.bytes)
        let carriedOnQuote = AssistantPictureLimits.carried(
            onQuote, kind: \.kind, mime: \.mime, bytes: \.bytes)
        let budget = AssistantPictureLimits.maxPerQuestion
        let shownOnMention = min(carriedOnMention.count, budget)
        let shownOnQuote = min(carriedOnQuote.count, budget - shownOnMention)
        return MentionPictureNotice(
            shownOnMention: shownOnMention,
            shownOnQuote: shownOnQuote,
            extra: (carriedOnMention.count - shownOnMention)
                + (carriedOnQuote.count - shownOnQuote),
            unreadable: (onMention.count - carriedOnMention.count)
                + (onQuote.count - carriedOnQuote.count),
            recentUpTo: recentPhotos ? budget - shownOnMention - shownOnQuote : nil)
    }

    /// What the strip says — the private thread's sentences where the fact
    /// is the same, and its own where it is not.
    ///
    /// The order is the private strip's: a photo that cannot travel is
    /// said first, ahead of the cap, because nobody expects a photograph
    /// to be left out for its bytes and everybody half expects a limit.
    /// Neither the private strip's "Nothing else from this chat does" nor
    /// its "The first 4 photos" is reused here, because both would be
    /// false in the family chat: with `ai_history` on a transcript goes
    /// with the mention, so the honest promise is about PHOTOS — no other
    /// photo in this chat does — and the four are counted across two
    /// messages in a stated order, which the sentence has to say.
    ///
    /// "No other photo in this chat does" survives only where it is still
    /// true: with `recentUpTo` nil (the third switch not in effect) or
    /// zero (the member's own photos spent the four, so history takes
    /// nothing). With places left, the sentence says "up to N" instead —
    /// and cannot say WHICH, because the server decides that from the
    /// transcript it builds when it answers. The cap sentence needs no
    /// variant: past the budget nothing is left for history at all.
    var sentence: String {
        let recent = recentUpTo ?? 0
        if unreadable > 0 {
            let base = String(localized: "A photo here is too large, or in a format the model can't read, so it will be told it's here but won't be shown it.")
            guard recent > 0 else { return base }
            // A photo that cannot travel spends no place, so the recent
            // ones may still fill it — said in a second sentence rather
            // than folded into the first, which is the private strip's
            // and is kept word for word.
            return base + " " + String(localized: "Up to \(recent) of the most recent photos in this chat may still go.")
        }
        if extra > 0 {
            return String(localized: "Only the first \(AssistantPictureLimits.maxPerQuestion) photos go to the model your server is set up to use — yours first, then the ones you're replying to. The rest are named to it, not shown.")
        }
        switch (shownOnMention > 0, shownOnQuote > 0) {
        case (true, false):
            if recent > 0 {
                return String(localized: "This goes to the model your server is set up to use, with your \(AssistantMention.token) message, and up to \(recent) of the most recent photos in this chat may go too.")
            }
            return String(localized: "This goes to the model your server is set up to use, with your \(AssistantMention.token) message. No other photo in this chat does.")
        case (false, true):
            if recent > 0 {
                return String(localized: "The photo you're replying to goes to the model your server is set up to use, with your \(AssistantMention.token) message, and up to \(recent) of the most recent photos in this chat may go too.")
            }
            return String(localized: "The photo you're replying to goes to the model your server is set up to use, with your \(AssistantMention.token) message. No other photo in this chat does.")
        case (true, true):
            if recent > 0 {
                return String(localized: "This and the photo you're replying to go to the model your server is set up to use, with your \(AssistantMention.token) message, and up to \(recent) of the most recent photos in this chat may go too.")
            }
            return String(localized: "This and the photo you're replying to go to the model your server is set up to use, with your \(AssistantMention.token) message. No other photo in this chat does.")
        case (false, false):
            // Nothing of the member's own: the strip is here only because
            // the third switch is on, and every one of the four may be
            // somebody else's recent picture. Said as such — whose they
            // are is whoever the chat says sent them.
            return String(localized: "Up to \(recent) of the most recent photos in this chat go to the model your server is set up to use, with your \(AssistantMention.token) message — pictures nobody pointed it at, whoever sent them.")
        }
    }
}
