//
//  TranscriptDoor.swift
//  FamilyConnect
//
//  What "Show text" is under one recording: there or not, and whether the
//  member is asked the consent question first (docs/protocol.md,
//  "Transcripts on request" and "Consenting to the assistant").
//
//  Asking for the text of a recording sends that recording's SOUND to the
//  assistant's provider, so it is the asker's consent the server checks,
//  the way a `/draw` checks the drawer's. Whose VOICE it is matters too:
//  a member's own recording may be asked about in any chat they are in;
//  somebody else's only in the family chat (its threads are the same
//  chat), only while the owner's `ai_transcripts` is on — and, a check this
//  client cannot make, only when that sender has agreed to the assistant
//  themselves. Never another member's in a direct chat, never the
//  assistant's own.
//
//  One rule for the phone's bubble and the Mac's row, so the two cannot
//  come to disagree, and a test can pin every row with no view on screen.
//  `BackdropDoor` is the same shape for the same reason.
//
//  TWO ROUTES, ONE ACTION (`TranscriptRoute`). A voice note or an audio
//  file the server can send as it is goes by the server's STORED copy; a
//  video, an Ogg file, a type outside the provider's list or a recording
//  over the ceiling goes by sound THIS device takes out of the file and
//  sends with the request (`TranscriptSound`). The reader sees the same
//  "Show text" either way.
//
//  THE ACTION IS NEVER HIDDEN FOR A REASON OF THE SOUND. Whether this
//  device can take the sound out of a file, and whether it then fits, is
//  something it only learns by trying (an Ogg file opens on one OS and not
//  on an older one; a video's sound track is invisible until the file is
//  here). A hidden action cannot say why it is missing, so the action is
//  offered on every video and audio file the RULES allow, and what the
//  device cannot do is said under it, terminally, in words the reader can
//  act on: "This recording is too long to turn into text." or "Couldn't
//  read the sound in this file." (`TranscriptFailure`). Only the rules —
//  who may ask, about what — ever take the action away.
//

import Foundation

nonisolated enum TranscriptDoor: Equatable {
    /// No action.
    case absent
    /// One tap asks.
    case open
    /// The member has not agreed that what they send may go to the model.
    /// The action is there; the tap raises the consent question first and
    /// asks when it is answered yes.
    case asksFirst

    /// `assistant.transcribe_max_bytes` when the server does not say —
    /// the protocol's default, which is also its ceiling (25 MiB).
    static let defaultMaxBytes: Int64 = 26_214_400

    /// The stored types the server sends as they are (protocol.md,
    /// "Transcripts on request" — the no-body form). Matched exactly, as
    /// the server matches them. `audio/ogg` is deliberately absent: the
    /// provider refuses Ogg.
    static let storedTypes: Set<String> = ["audio/mp4", "audio/m4a", "audio/mpeg", "audio/wav"]

    /// The most bytes of sound one request may send: what the server
    /// says, never more than the protocol's ceiling, the ceiling itself
    /// when the server does not say.
    static func ceiling(maxBytes: Int64?) -> Int64 {
        let said = maxBytes.flatMap { $0 > 0 ? $0 : nil } ?? defaultMaxBytes
        return min(said, defaultMaxBytes)
    }

    /// Can the server send THIS attachment's stored copy as it is?
    ///
    /// A voice note or an audio file (never a video, photo, file or
    /// place), of one of the four types, no bigger than the ceiling.
    static func storedCopyQualifies(_ attachment: AttachmentDTO, maxBytes: Int64?) -> Bool {
        guard attachment.kind == "audio" else { return false }
        guard storedTypes.contains(attachment.mime) else { return false }
        return attachment.size > 0 && attachment.size <= ceiling(maxBytes: maxBytes)
    }

    /// Which way this attachment's sound reaches the provider, or nil for
    /// an attachment that has none to send (a photo, a file, a place).
    ///
    /// The stored copy whenever it qualifies — it costs this device
    /// nothing, and its answer is kept by the server for the next member
    /// who may ask. Otherwise any voice note, audio file or video goes by
    /// sound this device supplies, WHATEVER its type or size: whether the
    /// sound can be taken out, and whether it then fits, is learned by
    /// trying and said under the action (the header above).
    static func route(for attachment: AttachmentDTO, maxBytes: Int64?) -> TranscriptRoute? {
        guard attachment.size > 0 else { return nil }
        if storedCopyQualifies(attachment, maxBytes: maxBytes) { return .stored }
        switch attachment.kind {
        case "audio", "video": return .supplied
        default: return nil
        }
    }

    /// One video of an album pile with its own "Show text" under the pile.
    nonisolated struct PileVideo: Equatable, Identifiable {
        let attachment: AttachmentDTO
        /// Its place among the pile's videos, counted from 1 — drawn as
        /// "Video 2" so the action says which it is the text of. Nil for a
        /// pile's only video.
        let number: Int?
        var id: Int64 { attachment.id }
    }

    /// The videos of an album pile, in order, each with its own action
    /// under the pile — numbered when there is more than one, because the
    /// pile is one card and an unnumbered action could not say which video
    /// it meant. Android, the web and Windows offer the same.
    static func pileVideos(_ media: [AttachmentDTO]) -> [PileVideo] {
        let videos = media.filter(\.isVideo)
        return videos.enumerated().map { at, video in
            PileVideo(attachment: video, number: videos.count > 1 ? at + 1 : nil)
        }
    }

    /// The protocol's "who may ask", as far as this client can know it.
    ///
    /// - Your own message: in any chat you are in, direct chats included.
    /// - Another member's: only in the family chat (threads included —
    ///   they are messages of the same chat) and only while the owner's
    ///   `ai_transcripts` is on.
    /// - The assistant's own messages: never.
    ///
    /// The server adds one check this client cannot make — that the
    /// SENDER has agreed to the assistant — and answers
    /// `transcript_not_allowed` when they have not, which the row draws as
    /// "Not available for this message."
    static func allows(
        chatKind: String?,
        senderID: Int64,
        currentUserID: Int64,
        assistantUserID: Int64?,
        familyAllowsTranscripts: Bool
    ) -> Bool {
        if let assistantUserID, senderID == assistantUserID { return false }
        if senderID == currentUserID, currentUserID > 0 { return chatKind != nil }
        return chatKind == "family" && familyAllowsTranscripts
    }

    /// The whole door, from every fact it turns on.
    ///
    /// `messageID` is the message's SERVER id: a message still on its way
    /// has nothing for the server to look up. A server that can transcribe
    /// but names no processor gets NO action, for the reason it gets no
    /// `/draw`: a consent screen with a hole where the recipient goes is
    /// not consent (`AssistantConsent.isAvailable`).
    static func of(
        serverTranscribes: Bool,
        maxBytes: Int64?,
        processor: String?,
        agreedAt: Date?,
        attachment: AttachmentDTO,
        messageID: Int64?,
        chatKind: String?,
        senderID: Int64,
        currentUserID: Int64,
        assistantUserID: Int64?,
        familyAllowsTranscripts: Bool
    ) -> TranscriptDoor {
        guard serverTranscribes else { return .absent }
        guard let messageID, messageID > 0, attachment.id > 0 else { return .absent }
        guard route(for: attachment, maxBytes: maxBytes) != nil else { return .absent }
        guard allows(
            chatKind: chatKind, senderID: senderID, currentUserID: currentUserID,
            assistantUserID: assistantUserID, familyAllowsTranscripts: familyAllowsTranscripts)
        else { return .absent }
        guard AssistantConsent.isAvailable(processor: processor) else { return .absent }
        return agreedAt == nil ? .asksFirst : .open
    }

    /// Whether the action is on screen.
    var isOffered: Bool { self != .absent }
}

/// The two forms of the request (protocol.md, "Transcripts on request").
nonisolated enum TranscriptRoute: Equatable, Sendable {
    /// No body: the server sends the file it holds, and keeps the answer.
    case stored
    /// A multipart `audio` part this device took out of the file. The
    /// server keeps nothing; this device's copy of the answer is the only
    /// one.
    case supplied
}

/// Why asking did not produce a text, sorted by what the row can do next.
nonisolated enum TranscriptFailure: Equatable, Sendable {
    /// `internal`, a timeout, no connection, a throttle, anything the
    /// server did not refuse on purpose. Asking again may work.
    case retry
    /// `transcript_refused`: the provider's own filter. Terminal — the same
    /// recording gets the same refusal, so no retry is offered.
    case refused
    /// `not_transcribable`, `transcript_not_allowed`,
    /// `transcripts_unavailable`, and the message or attachment no longer
    /// being there. Terminal until something on the server changes.
    case notAvailable
    /// The sound this device took out of the file is over
    /// `transcribe_max_bytes` even at 64 kbit/s mono — or the recording's
    /// length alone says it would be. Terminal: the same file gives the
    /// same sound.
    case tooLong
    /// This device could not take the sound out of the file: no sound
    /// track, or a format this OS cannot open. Terminal on this device.
    case unreadable

    /// The sentence the row draws.
    var message: String {
        switch self {
        case .retry: String(localized: "Couldn't get the text. Try again.")
        case .refused: String(localized: "The assistant's provider refused this recording.")
        case .notAvailable: String(localized: "Not available for this message.")
        case .tooLong: String(localized: "This recording is too long to turn into text.")
        case .unreadable: String(localized: "Couldn't read the sound in this file.")
        }
    }

    /// Whether the row offers to ask again.
    var offersRetry: Bool { self == .retry }
}

/// What asking for a recording's text came to.
nonisolated enum TranscriptOutcome: Equatable, Sendable {
    case text(TranscriptDTO)
    /// `assistant_consent_required` (403): the ASKER has not agreed. Nothing
    /// was sent. Not a failure to report but the consent question to ask —
    /// the row asks before it offers, and this is the backstop for a
    /// consent withdrawn on another device.
    case consentRequired
    case failed(TranscriptFailure)

    /// Sorted from what the request threw. Only the protocol's codes decide
    /// a refusal; anything the server did not answer on purpose is worth
    /// asking again.
    init(error: Error) {
        switch error {
        // What this device found when it tried to make the sound itself.
        case TranscriptSound.Failure.tooLong:
            self = .failed(.tooLong)
        case TranscriptSound.Failure.unreadable, is MediaTranscoder.Failure:
            self = .failed(.unreadable)
        // A body over nginx's limit. Only the supplied form has a body,
        // and this device checks the ceiling before it sends, so this is
        // a server whose limit is lower than it says — still "too long".
        case APIError.payloadTooLarge:
            self = .failed(.tooLong)
        case APIError.forbidden(let code) where code == "assistant_consent_required":
            self = .consentRequired
        case APIError.conflict(let code, _) where code == "transcript_refused":
            self = .failed(.refused)
        case APIError.conflict(let code, _)
            where code == "not_transcribable" || code == "blocked":
            self = .failed(.notAvailable)
        // `transcript_not_allowed`, `transcripts_unavailable`,
        // `not_chat_member` — every 403 here is the server refusing on
        // purpose, and none of them changes by asking again.
        case APIError.forbidden:
            self = .failed(.notAvailable)
        // `chat_not_found`, `message_not_found`, `attachment_not_found`:
        // the recording is not there to be read any more.
        case APIError.notFound:
            self = .failed(.notAvailable)
        default:
            self = .failed(.retry)
        }
    }

    /// Does the row answer this by showing the consent sheet — and asking
    /// again on a yes? Only where the question can be asked at all.
    func asksForConsent(processor: String?) -> Bool {
        self == .consentRequired && AssistantConsent.isAvailable(processor: processor)
    }
}
