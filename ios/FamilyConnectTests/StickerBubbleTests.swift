//
//  StickerBubbleTests.swift
//  FamilyConnectTests
//
//  How a sticker is DRAWN, which is a rule five clients have to agree on
//  and no logic test can see (docs/protocol.md, "How it is drawn"): no
//  bubble, one fixed box, fitted whole, its transparency showing the chat
//  behind it.
//
//  A PIXEL TEST, for BubbleLayoutTests' reason: every one of those is a
//  fact about ink. The fixture is the 8 × 8 WebP whose left half is opaque
//  red and whose right half is a hole, so the four rules read straight off
//  the rendering —
//
//    * NO BUBBLE: an own message's balloon is filled with the tint, so a
//      sticker's rendering holds no tint-coloured pixel at all (a text
//      message, the control, holds thousands);
//    * THE FIXED BOX, FITTED WHOLE: the red half is as tall as the box and
//      half as wide — an 8-pixel picture drawn at the box's size, not at
//      its own;
//    * TRANSPARENCY: the other half of the box is not drawn. A sticker that
//      had gone through the JPEG photo path would be red beside WHITE;
//    * THE ORIGINAL BYTES: the store under test holds nothing under the
//      preview key, and `has_preview` on the fixture is TRUE — so anything
//      drawn at all was drawn from the original, whatever the flag says.
//
//  iOS only, like the view it measures.
//

#if os(iOS)

import CoreGraphics
import Foundation
import SwiftUI
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Sticker bubble")
struct StickerBubbleTests {

    private func store(seeding bytes: Data?, id: Int64) -> AttachmentStore {
        let api = APIClient(serverURL: URL(string: "https://stickers.invalid"))
        let directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("sticker-bubble-\(UUID().uuidString)")
        let store = AttachmentStore(api: api, directory: directory)
        if let bytes { store.seed(bytes, id: id, preview: false) }
        return store
    }

    private func render(
        body: String, attachments: [AttachmentDTO], replyTo: ReplyToSnapshot? = nil,
        store: AttachmentStore
    ) throws -> CGImage {
        let message = MessageSnapshot(
            localID: "s:1", serverID: 1, chatID: 42, senderID: 7, body: body,
            createdAt: Date(timeIntervalSince1970: 0), state: .sent,
            replyTo: replyTo, attachment: attachments.first, attachments: attachments)
        let view = MessageBubbleView(
            message: message,
            isMine: true,
            showsSenderName: false,
            senderName: nil,
            // Unread, so the footer's tick is grey: a READ tick is drawn in
            // the tint and would be the one blue thing on the canvas.
            isRead: false,
            memberNames: [7: "You", 9: "Ana"],
            currentUserID: 7)
            .frame(width: 320)
            .tint(.blue)
            .environment(\.dynamicTypeSize, .large)
            .environment(LinkPreviewLoader())
            .environment(store)
            .environment(AvatarStore(api: APIClient(serverURL: URL(string: "https://avatars.invalid"))))
        let renderer = ImageRenderer(content: view)
        renderer.scale = 1
        return try #require(renderer.cgImage, "the bubble did not render")
    }

    /// Every pixel as RGBA, straight (the canvas is transparent).
    private func pixels(of image: CGImage) throws -> (data: [UInt8], width: Int, height: Int) {
        var data = [UInt8](repeating: 0, count: image.width * image.height * 4)
        let context = try #require(CGContext(
            data: &data, width: image.width, height: image.height,
            bitsPerComponent: 8, bytesPerRow: image.width * 4,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        return (data, image.width, image.height)
    }

    /// The bounding box of every pixel `matches` accepts, and how many.
    private func extent(
        of image: CGImage, where matches: (UInt8, UInt8, UInt8, UInt8) -> Bool
    ) throws -> (count: Int, width: Int, height: Int) {
        let (data, width, height) = try pixels(of: image)
        var count = 0
        var minX = Int.max, maxX = -1, minY = Int.max, maxY = -1
        for y in 0..<height {
            for x in 0..<width {
                let offset = (y * width + x) * 4
                guard matches(data[offset], data[offset + 1], data[offset + 2], data[offset + 3])
                else { continue }
                count += 1
                minX = min(minX, x); maxX = max(maxX, x)
                minY = min(minY, y); maxY = max(maxY, y)
            }
        }
        guard count > 0 else { return (0, 0, 0) }
        return (count, maxX - minX + 1, maxY - minY + 1)
    }

    private let isRed: (UInt8, UInt8, UInt8, UInt8) -> Bool = { r, g, b, a in
        a > 200 && r > 200 && g < 70 && b < 70
    }
    /// The tint the fixture is rendered with — the colour of an own balloon.
    private let isTint: (UInt8, UInt8, UInt8, UInt8) -> Bool = { r, g, b, a in
        a > 200 && b > 200 && r < 80
    }
    private let isWhite: (UInt8, UInt8, UInt8, UInt8) -> Bool = { r, g, b, a in
        a > 200 && r > 240 && g > 240 && b > 240
    }

    private func sticker(hasPreview: Bool = true) -> AttachmentDTO {
        AttachmentDTO(
            id: 90, kind: "photo", mime: "image/webp", size: 38, width: 8, height: 8,
            durationMS: nil, hasPreview: hasPreview, name: nil, latitude: nil, longitude: nil,
            accuracyM: nil, sticker: true)
    }

    @Test("a sticker draws with no bubble, in the fixed box, from its original bytes, transparent")
    func aStickerIsBare() throws {
        let store = store(seeding: StickerFixtures.stillWebP, id: 90)
        let image = try render(body: "", attachments: [sticker()], store: store)

        let tint = try extent(of: image, where: isTint)
        #expect(tint.count == 0, "a sticker drew \(tint.count) pixels of balloon")

        // The red half of an 8 × 8 picture, drawn at the BOX's size: as
        // tall as the box and half as wide.
        let red = try extent(of: image, where: isRed)
        let box = Int(StickerPack.messageBox)
        #expect(red.count > 0, "nothing was drawn — a sticker read its preview and found none")
        #expect(abs(red.height - box) <= 3, "the sticker is \(red.height)pt tall in a \(box)pt box")
        // Looser sideways than in height: the picture is eight pixels
        // scaled twentyfold, so the edge between the red half and the hole
        // is a ramp one source pixel wide, and "red" stops partway down it.
        #expect(abs(red.width - box / 2) <= 12, "the red half is \(red.width)pt wide")

        // And the other half is a HOLE, not white paper.
        let white = try extent(of: image, where: isWhite)
        #expect(white.count < 200, "the sticker's transparent half was filled in (\(white.count) px)")
    }

    @Test("the control: a text message, and the same picture sent as a PHOTO, do draw a surface")
    func theControlsDrawWhatAStickerDoesNot() throws {
        let store = store(seeding: StickerFixtures.stillWebP, id: 90)
        let text = try render(body: "See you at six", attachments: [], store: store)
        #expect(try extent(of: text, where: isTint).count > 1000, "the control balloon has no tint")

        // The very same attachment WITHOUT the flag — what an old message,
        // or a client that has never heard of stickers, holds. It is a
        // photo tile at the photo's size, not a sticker in the box.
        let photo = AttachmentDTO(
            id: 90, kind: "photo", mime: "image/webp", size: 38, width: 8, height: 8,
            durationMS: nil, hasPreview: false, name: nil, latitude: nil, longitude: nil,
            accuracyM: nil)
        let asPhoto = try render(body: "", attachments: [photo], store: store)
        let red = try extent(of: asPhoto, where: isRed)
        #expect(red.height > Int(StickerPack.messageBox) + 40,
                "an unflagged photo was drawn in the sticker box")
    }

    @Test("a sticker that is a reply keeps its quote and still has no balloon")
    func aStickerReplyHasNoBalloon() throws {
        let store = store(seeding: StickerFixtures.stillWebP, id: 90)
        let quote = ReplyToSnapshot(messageID: 5, senderID: 9, excerpt: "See you at six", parent: nil)
        let image = try render(body: "", attachments: [sticker()], replyTo: quote, store: store)

        #expect(try extent(of: image, where: isTint).count == 0)
        let red = try extent(of: image, where: isRed)
        #expect(abs(red.height - Int(StickerPack.messageBox)) <= 3)
        // The quote is there: the canvas is taller than the sticker alone.
        let alone = try render(body: "", attachments: [sticker()], store: store)
        #expect(image.height > alone.height + 10, "the reply's quote was not drawn")
    }
}

#endif
