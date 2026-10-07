//
//  RoundPresentationTests.swift
//  FamilyConnectTests
//
//  Which message is drawn as a circle, what it offers, and what a quote of
//  one says (#79, docs/audio-video-messages-2026-10-04.md, S5.1, S5.4, S5.7).
//
//  The drawing test itself is `RoundVideo.isRound`, held to fc_text's
//  `is_round` by the record vectors (RecordVectorTests); these pin that the
//  presentation layer ASKS it — on the real snapshot, with the real field —
//  and the rules built on it.
//

import Foundation
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Video message presentation")
struct RoundPresentationTests {

    private func attachment(kind: String = "video", round: Bool, id: Int64 = 91) -> AttachmentDTO {
        AttachmentDTO(
            id: id, kind: kind, mime: kind == "video" ? "video/mp4" : "image/jpeg", size: 1,
            width: 480, height: 480, durationMS: 23_400, hasPreview: true, name: nil,
            latitude: nil, longitude: nil, accuracyM: nil, isRound: round)
    }

    private func message(
        serverID: Int64? = 1, senderID: Int64 = 9, body: String = "",
        attachments: [AttachmentDTO], replyTo: ReplyToSnapshot? = nil, state: MessageStatus = .sent
    ) -> MessageSnapshot {
        MessageSnapshot(
            localID: "m:\(serverID ?? 0)", serverID: serverID, chatID: 42, senderID: senderID,
            body: body, createdAt: Date(timeIntervalSince1970: 0), state: state,
            replyTo: replyTo, attachment: attachments.first, attachments: attachments)
    }

    // MARK: - S5.1

    @Test("one video carrying round: true, and no body, is a circle")
    func theCircle() {
        #expect(MessagePresentation.isRoundVideo(message(attachments: [attachment(round: true)])))
    }

    @Test("a reply is still a circle")
    func aReplyIsACircle() {
        let quote = ReplyToSnapshot(messageID: 5, senderID: 9, excerpt: "See you at six")
        #expect(MessagePresentation.isRoundVideo(
            message(attachments: [attachment(round: true)], replyTo: quote)))
    }

    @Test("anything else is the ordinary message it otherwise is")
    func everythingElse() {
        // No flag: an old message, or an ordinary video.
        #expect(!MessagePresentation.isRoundVideo(message(attachments: [attachment(round: false)])))
        // Two attachments.
        #expect(!MessagePresentation.isRoundVideo(message(attachments: [
            attachment(round: true), attachment(round: true, id: 92)])))
        // The flag on a photo.
        #expect(!MessagePresentation.isRoundVideo(
            message(attachments: [attachment(kind: "photo", round: true)])))
        // Words beside it.
        #expect(!MessagePresentation.isRoundVideo(
            message(body: "look", attachments: [attachment(round: true)])))
        // Nothing at all.
        #expect(!MessagePresentation.isRoundVideo(message(body: "hi", attachments: [])))
    }

    // MARK: - S5.4

    @Test("no Edit on one's own video message; a plain video of one's own still offers it")
    func noEdit() {
        let mine = message(senderID: 7, attachments: [attachment(round: true)])
        #expect(!MessagePresentation.offersEdit(mine, currentUserID: 7))
        let plain = message(senderID: 7, attachments: [attachment(round: false)])
        #expect(MessagePresentation.offersEdit(plain, currentUserID: 7))
    }

    // MARK: - S5.7

    @Test("the round ids are the server ids of the circles, and only theirs")
    func roundIDs() {
        let ids = MessagePresentation.roundMessageIDs([
            message(serverID: 1, attachments: [attachment(round: true)]),
            message(serverID: 2, attachments: [attachment(round: false)]),
            message(serverID: nil, attachments: [attachment(round: true)]),
            message(serverID: 3, body: "hi", attachments: []),
        ])
        #expect(ids == [1])
    }

    @Test("a quote of a circle says Video message; words and unknown messages are kept")
    func quoteWord() {
        let word = String(localized: "Video message")
        #expect(MessagePresentation.quoteWord(excerpt: "", messageID: 1, roundIDs: [1]) == word)
        #expect(MessagePresentation.quoteWord(excerpt: "hi", messageID: 1, roundIDs: [1]) == "hi")
        #expect(MessagePresentation.quoteWord(excerpt: "", messageID: 2, roundIDs: [1]) == "")
    }

    @Test("both levels of a quote are named, and nothing else about the message changes")
    func namingBothLevels() {
        let quote = ReplyToSnapshot(
            messageID: 1, senderID: 9, excerpt: "",
            parent: QuotedParentSnapshot(messageID: 2, senderID: 4, excerpt: ""))
        let reply = message(serverID: 10, body: "Lovely", attachments: [], replyTo: quote)
        let named = MessagePresentation.namingRoundQuotes(reply, roundIDs: [1, 2])
        let word = String(localized: "Video message")
        #expect(named.replyTo?.excerpt == word)
        #expect(named.replyTo?.parent?.excerpt == word)
        #expect(named.replyTo?.messageID == 1)
        #expect(named.body == "Lovely")
        #expect(MessagePresentation.namingRoundQuotes(reply, roundIDs: []) == reply)
        #expect(MessagePresentation.namingRoundQuotes(reply, roundIDs: [99]) == reply)
    }

    @Test("only a SENT circle plays: your own going up says Sending…, your own failed one offers nothing to play (S5.6)")
    func uploadState() {
        #expect(RoundUpload.of(isMine: true, serverID: nil, state: .pending) == .sending)
        #expect(RoundUpload.of(isMine: true, serverID: nil, state: .failed) == .failed)
        #expect(RoundUpload.of(isMine: true, serverID: 5, state: .sent) == .sent)
        #expect(RoundUpload.of(isMine: false, serverID: 5, state: .sent) == .sent)
        #expect(!RoundUpload.failed.playable, "a failed circle streams an attachment the server does not have")
        #expect(!RoundUpload.sending.playable)
        #expect(RoundUpload.sent.playable)
        #expect(RoundUpload.of(message(serverID: nil, senderID: 7, attachments: [attachment(round: true)], state: .failed), isMine: true) == .failed)
    }
}
