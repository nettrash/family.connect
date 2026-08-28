//
//  MediaOnlyTests.swift
//  FamilyConnectTests
//
//  Pins which messages the bubble draws BARE for their attachments — no
//  balloon, the tile as the message — and which keep the balloon. The
//  rule is a cross-platform contract (MediaOnlyTest.kt mirrors these
//  vectors): photos and videos alone, one or several, draw bare; a
//  caption, a quote, a poll, a call, a body still arriving, or any file,
//  audio or location row keeps the balloon, because those are words and
//  controls and need the surface.
//
//  Also pins the hairline rule that rides on it: a media tile strokes its
//  edge only where its own pixels are not already the edge — over a
//  balloon, or before the picture lands. The album pile is NOT this rule
//  and always strokes: there the line separates photo from photo.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Media-only messages")
struct MediaOnlyTests {

    private static func attachment(_ id: Int64, kind: String) -> AttachmentDTO {
        AttachmentDTO(
            id: id,
            kind: kind,
            mime: kind == AttachmentDTO.Kind.video ? "video/mp4" : "image/jpeg",
            size: 1,
            width: 1200,
            height: 900,
            durationMS: nil,
            hasPreview: false,
            name: kind == AttachmentDTO.Kind.file ? "receipt.pdf" : nil,
            latitude: nil,
            longitude: nil,
            accuracyM: nil)
    }

    private static func message(
        body: String = "",
        attachments: [AttachmentDTO],
        replyTo: ReplyToSnapshot? = nil,
        poll: PollSnapshot? = nil,
        call: CallDTO? = nil
    ) -> MessageSnapshot {
        MessageSnapshot(
            localID: "m:1",
            serverID: 1,
            chatID: 42,
            senderID: 7,
            body: body,
            createdAt: Date(timeIntervalSince1970: 1_700_000_000),
            state: .sent,
            replyTo: replyTo,
            attachment: attachments.first,
            attachments: attachments,
            poll: poll,
            call: call)
    }

    private static let photo = attachment(1, kind: AttachmentDTO.Kind.photo)
    private static let video = attachment(2, kind: AttachmentDTO.Kind.video)
    private static let file = attachment(3, kind: AttachmentDTO.Kind.file)
    private static let audio = attachment(4, kind: AttachmentDTO.Kind.audio)
    private static let place = attachment(5, kind: AttachmentDTO.Kind.location)

    @Test("photos and videos alone, one or several, draw bare")
    func mediaAloneIsBare() {
        #expect(MessagePresentation.isMediaOnly(Self.message(attachments: [Self.photo])))
        #expect(MessagePresentation.isMediaOnly(Self.message(attachments: [Self.video])))
        #expect(MessagePresentation.isMediaOnly(
            Self.message(attachments: [Self.photo, Self.video, Self.photo])))
    }

    @Test("a file, audio or location row keeps the balloon — alone or beside a photo")
    func rowsKeepTheBalloon() {
        #expect(!MessagePresentation.isMediaOnly(Self.message(attachments: [Self.file])))
        #expect(!MessagePresentation.isMediaOnly(Self.message(attachments: [Self.audio])))
        #expect(!MessagePresentation.isMediaOnly(Self.message(attachments: [Self.place])))
        #expect(!MessagePresentation.isMediaOnly(Self.message(attachments: [Self.photo, Self.file])))
        #expect(!MessagePresentation.isMediaOnly(Self.message(attachments: [Self.photo, Self.photo, Self.audio])))
    }

    @Test("anything with words keeps the balloon: a caption, a quote, a poll, a call, a body still arriving")
    func wordsKeepTheBalloon() {
        #expect(!MessagePresentation.isMediaOnly(Self.message(body: "look", attachments: [Self.photo])))
        #expect(!MessagePresentation.isMediaOnly(Self.message(
            attachments: [Self.photo],
            replyTo: ReplyToSnapshot(messageID: 9, senderID: 3, excerpt: "which one?"))))
        #expect(!MessagePresentation.isMediaOnly(Self.message(
            attachments: [Self.photo],
            poll: PollSnapshot(pollSeq: 1, closed: false, options: []))))
        #expect(!MessagePresentation.isMediaOnly(Self.message(
            attachments: [Self.photo],
            call: CallDTO(outcome: "missed"))))
        #expect(!MessagePresentation.isMediaOnly(
            Self.message(attachments: [Self.photo]), isStreaming: true))
    }

    @Test("a tile strokes its edge only over a balloon, or before its picture lands")
    func hairlineRule() {
        // In a balloon: always — that is what the stroke was cut for.
        #expect(MessagePresentation.drawsHairline(onBalloon: true, hasImage: true))
        #expect(MessagePresentation.drawsHairline(onBalloon: true, hasImage: false))
        // Bare with a picture: the photo's own pixels ARE the edge, and a
        // stroke there is a frame around a picture.
        #expect(!MessagePresentation.drawsHairline(onBalloon: false, hasImage: true))
        // Bare with nothing in it yet: a reserved rectangle holding a wash
        // needs the only edge it has.
        #expect(MessagePresentation.drawsHairline(onBalloon: false, hasImage: false))
    }

    @Test("a text message, and an empty one, are not media-only")
    func noAttachmentsIsNotBare() {
        #expect(!MessagePresentation.isMediaOnly(Self.message(body: "hi", attachments: [])))
        #expect(!MessagePresentation.isMediaOnly(Self.message(attachments: [])))
    }
}
