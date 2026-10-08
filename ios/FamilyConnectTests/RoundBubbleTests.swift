//
//  RoundBubbleTests.swift
//  FamilyConnectTests
//
//  How a video message is DRAWN (#79, docs/audio-video-messages-2026-10-04.md,
//  S5.2; docs/protocol.md, "How it is drawn"): no balloon, a circle 200
//  across in a compact width and 240 otherwise, the square poster filling it
//  — and nothing outside the circle.
//
//  A PIXEL TEST, for StickerBubbleTests' reason. The poster is a solid red
//  square seeded under the preview key, so the circle reads straight off the
//  rendering: the red is as wide and as tall as the diameter, and the
//  corners of that square — outside the circle — are not drawn at all.
//
//  iOS only, like the view it measures.
//

#if os(iOS)

import CoreGraphics
import Foundation
import SwiftData
import SwiftUI
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Video message bubble")
struct RoundBubbleTests {

    /// Held for the whole test: a coordinator outliving its container traps.
    @MainActor
    final class World {
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let session: AppSession
        let store: AttachmentStore

        init() throws {
            container = try ModelContainer(
                for: ChatEntity.self, MessageEntity.self, MemberEntity.self, NoteEntity.self,
                PendingMediaItemEntity.self,
                configurations: ModelConfiguration(isStoredInMemoryOnly: true))
            let api = APIClient(serverURL: URL(string: "https://circles.invalid"))
            coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
            coordinator.currentUserIDOverride = 7
            session = AppSession(api: api, defaultServerURL: { nil })
            let directory = URL(fileURLWithPath: NSTemporaryDirectory())
                .appendingPathComponent("round-bubble-\(UUID().uuidString)")
            store = AttachmentStore(api: api, directory: directory)
            store.seed(Self.redPoster(), id: 91, preview: true)
        }

        /// A solid red square JPEG — the poster.
        static func redPoster() -> Data {
            let context = CGContext(
                data: nil, width: 64, height: 64, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
            context.setFillColor(red: 1, green: 0, blue: 0, alpha: 1)
            context.fill(CGRect(x: 0, y: 0, width: 64, height: 64))
            return PlatformImage.jpegData(from: context.makeImage()!, quality: 1) ?? Data()
        }
    }

    private func video(round: Bool) -> AttachmentDTO {
        AttachmentDTO(
            id: 91, kind: "video", mime: "video/mp4", size: 1_649_700, width: 480, height: 480,
            durationMS: 23_400, hasPreview: true, name: nil, latitude: nil, longitude: nil,
            accuracyM: nil, isRound: round)
    }

    private func render(
        _ world: World, attachments: [AttachmentDTO], replyTo: ReplyToSnapshot? = nil,
        sizeClass: UserInterfaceSizeClass, serverID: Int64? = 1, state: MessageStatus = .sent
    ) throws -> CGImage {
        let message = MessageSnapshot(
            localID: "r:1", serverID: serverID, chatID: 42, senderID: 7, body: "",
            createdAt: Date(timeIntervalSince1970: 0), state: state,
            replyTo: replyTo, attachment: attachments.first, attachments: attachments)
        let view = MessageBubbleView(
            message: message,
            isMine: true,
            showsSenderName: false,
            senderName: nil,
            isRead: false,
            memberNames: [7: "You", 9: "Ana"],
            currentUserID: 7)
            .frame(width: 360)
            .tint(.blue)
            .environment(\.horizontalSizeClass, sizeClass)
            .environment(\.dynamicTypeSize, .large)
            .environment(LinkPreviewLoader())
            .environment(world.store)
            .environment(world.coordinator)
            .environment(world.session)
            .environment(AvatarStore(api: APIClient(serverURL: URL(string: "https://avatars.invalid"))))
        let renderer = ImageRenderer(content: view)
        renderer.scale = 1
        return try #require(renderer.cgImage, "the bubble did not render")
    }

    private struct Pixels {
        let data: [UInt8]
        let width: Int
        let height: Int

        func rgba(_ x: Int, _ y: Int) -> (UInt8, UInt8, UInt8, UInt8) {
            let offset = (y * width + x) * 4
            return (data[offset], data[offset + 1], data[offset + 2], data[offset + 3])
        }
    }

    private func pixels(of image: CGImage) throws -> Pixels {
        var data = [UInt8](repeating: 0, count: image.width * image.height * 4)
        let context = try #require(CGContext(
            data: &data, width: image.width, height: image.height,
            bitsPerComponent: 8, bytesPerRow: image.width * 4,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        return Pixels(data: data, width: image.width, height: image.height)
    }

    /// The bounding box of every pixel `matches` accepts, and how many.
    private func extent(
        _ pixels: Pixels, where matches: (UInt8, UInt8, UInt8, UInt8) -> Bool
    ) -> (count: Int, minX: Int, minY: Int, maxX: Int, maxY: Int) {
        var count = 0
        var minX = Int.max, maxX = -1, minY = Int.max, maxY = -1
        for y in 0..<pixels.height {
            for x in 0..<pixels.width {
                let (r, g, b, a) = pixels.rgba(x, y)
                guard matches(r, g, b, a) else { continue }
                count += 1
                minX = min(minX, x); maxX = max(maxX, x)
                minY = min(minY, y); maxY = max(maxY, y)
            }
        }
        return (count, minX, minY, maxX, maxY)
    }

    private let isRed: (UInt8, UInt8, UInt8, UInt8) -> Bool = { r, g, b, a in
        a > 200 && r > 200 && g < 70 && b < 70
    }
    /// The tint — the colour of an own balloon.
    private let isTint: (UInt8, UInt8, UInt8, UInt8) -> Bool = { r, g, b, a in
        a > 200 && b > 200 && r < 80
    }

    /// The circle: red across the diameter both ways, no balloon, and the
    /// four corners of its square not drawn.
    private func expectCircle(_ image: CGImage, diameter: Int) throws {
        let pixels = try pixels(of: image)
        #expect(extent(pixels, where: isTint).count == 0, "a video message drew a balloon")

        let red = extent(pixels, where: isRed)
        #expect(red.count > 0, "the poster was not drawn")
        #expect(abs(red.maxX - red.minX + 1 - diameter) <= 3,
                "the circle is \(red.maxX - red.minX + 1) wide, not \(diameter)")
        #expect(abs(red.maxY - red.minY + 1 - diameter) <= 3,
                "the circle is \(red.maxY - red.minY + 1) tall, not \(diameter)")

        // Outside the circle, inside its square: transparent.
        let inset = diameter / 10
        for (x, y) in [
            (red.minX + inset, red.minY + inset), (red.maxX - inset, red.minY + inset),
            (red.minX + inset, red.maxY - inset), (red.maxX - inset, red.maxY - inset),
        ] {
            let alpha = pixels.rgba(x, y).3
            #expect(alpha < 10, "a corner of the circle's square is drawn (alpha \(alpha) at \(x),\(y))")
        }
        // And inside the circle, off its centre glyph, it is the poster.
        let left = pixels.rgba(red.minX + diameter / 6, (red.minY + red.maxY) / 2)
        #expect(isRed(left.0, left.1, left.2, left.3), "the poster does not fill the circle")
    }

    @Test("a video message is a circle 200 across in a compact width, with no balloon")
    func compact() throws {
        let world = try World()
        let image = try render(world, attachments: [video(round: true)], sizeClass: .compact)
        try expectCircle(image, diameter: Int(RecordRules.roundDiameterCompact))
    }

    @Test("and 240 across in a regular width")
    func regular() throws {
        let world = try World()
        let image = try render(world, attachments: [video(round: true)], sizeClass: .regular)
        try expectCircle(image, diameter: Int(RecordRules.roundDiameterRegular))
    }

    @Test("a video message that is a reply keeps its quote above, and still has no balloon")
    func aReplyHasNoBalloon() throws {
        let world = try World()
        let quote = ReplyToSnapshot(messageID: 5, senderID: 9, excerpt: "See you at six")
        let image = try render(
            world, attachments: [video(round: true)], replyTo: quote, sizeClass: .compact)
        try expectCircle(image, diameter: Int(RecordRules.roundDiameterCompact))
        let alone = try render(world, attachments: [video(round: true)], sizeClass: .compact)
        #expect(image.height > alone.height + 10, "the reply's quote was not drawn")
    }

    /// The circle's centre pixel: the poster's red, or darkened by the play
    /// disc drawn over it.
    private func centre(_ image: CGImage) throws -> (UInt8, UInt8, UInt8, UInt8) {
        let pixels = try pixels(of: image)
        let red = extent(pixels, where: isRed)
        try #require(red.count > 0, "the poster was not drawn")
        return pixels.rgba((red.minX + red.maxX) / 2, (red.minY + red.maxY) / 2)
    }

    @Test("a sent circle offers its play disc; your own FAILED one does not — nothing there to play (S5.6)")
    func aFailedCircleHasNoPlayDisc() throws {
        let world = try World()
        let sent = try centre(render(world, attachments: [video(round: true)], sizeClass: .compact))
        #expect(!isRed(sent.0, sent.1, sent.2, sent.3), "a sent circle lost its play disc")
        let failed = try centre(render(
            world, attachments: [video(round: true)], sizeClass: .compact, serverID: nil, state: .failed))
        #expect(isRed(failed.0, failed.1, failed.2, failed.3),
                "a failed circle offers a play button for a video the server never got")
        let sending = try centre(render(
            world, attachments: [video(round: true)], sizeClass: .compact, serverID: nil, state: .pending))
        #expect(isRed(sending.0, sending.1, sending.2, sending.3), "a circle still going up offers play")
    }

    @Test("the control: the same video without the flag is a square tile, corners and all")
    func theControlIsSquare() throws {
        let world = try World()
        let image = try render(world, attachments: [video(round: false)], sizeClass: .compact)
        let pixels = try pixels(of: image)
        let red = extent(pixels, where: isRed)
        #expect(red.count > 0)
        // A 480 × 480 video in the phone's tile: 240 square, its corners
        // drawn (a 14-point radius, so 24 points in is well inside).
        #expect(abs(red.maxX - red.minX + 1 - 240) <= 3)
        let corner = pixels.rgba(red.minX + 24, red.minY + 24)
        #expect(isRed(corner.0, corner.1, corner.2, corner.3), "an unflagged video was drawn round")
    }
}

#endif
