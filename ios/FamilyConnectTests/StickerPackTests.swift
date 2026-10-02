//
//  StickerPackTests.swift
//  FamilyConnectTests
//
//  The sticker pack's rules that need no network and no store
//  (docs/protocol.md, "Sticker pack"): what bytes may be a sticker, how a
//  picture becomes one, who may remove one, whether the pack already holds
//  one, which the panel shows first — and what a sticker message is, to a
//  bubble and to a chat-list row.
//
//  The one these exist to pin above all: A STICKER IS NEVER PREPARED. Every
//  photograph this app sends is turned into a JPEG on its way out, and a
//  JPEG has no transparency and one frame. A finished WebP or PNG must come
//  out of `StickerPack.make` byte for byte — animation, alpha and all — and
//  only a picture that is not a sticker yet is redrawn, as a PNG that still
//  has its transparency.
//

import CoreGraphics
import Foundation
import Testing
@testable import FamilyConnect

@Suite("Sticker pack rules")
struct StickerPackTests {

    private static let ceiling = 512 * 1024

    // MARK: - What the bytes are

    @Test("the type is judged from the bytes: PNG, WebP, and nothing else")
    func mimeComesFromTheMagicNumber() {
        #expect(StickerPack.mime(of: StickerFixtures.stillWebP) == "image/webp")
        #expect(StickerPack.mime(of: StickerFixtures.animatedWebP) == "image/webp")
        #expect(StickerPack.mime(of: StickerFixtures.png(width: 8, height: 8)) == "image/png")
        #expect(StickerPack.mime(of: TestImages.photograph(width: 64, height: 48)) == nil)
        // RIFF, but a WAVE: the four bytes at offset 8 are what make it a
        // WebP, exactly as the server checks it.
        #expect(StickerPack.mime(of: Data("RIFF\0\0\0\0WAVEfmt ".utf8)) == nil)
        #expect(StickerPack.mime(of: Data()) == nil)
    }

    @Test("an animated WebP reports its frames, a still one reports one")
    func frameCounts() {
        #expect(StickerPack.frameCount(of: StickerFixtures.stillWebP) == 1)
        #expect(StickerPack.frameCount(of: StickerFixtures.animatedWebP) == 2)
        #expect(StickerPack.frameCount(of: Data("not a picture".utf8)) == 0)
    }

    // MARK: - Making one

    @Test("a WebP within the ceiling goes up exactly as it is")
    func aFinishedWebPIsTakenAsGiven() throws {
        let made = try StickerPack.make(from: StickerFixtures.stillWebP, maxBytes: Self.ceiling)
        #expect(made.data == StickerFixtures.stillWebP, "a finished sticker was re-encoded")
        #expect(made.mime == "image/webp")
        #expect(made.width == 8)
        #expect(made.height == 8)
    }

    @Test("an ANIMATED WebP keeps every byte, and so every frame")
    func anAnimatedWebPIsNeverReencoded() throws {
        let made = try StickerPack.make(from: StickerFixtures.animatedWebP, maxBytes: Self.ceiling)
        #expect(made.data == StickerFixtures.animatedWebP)
        #expect(StickerPack.frameCount(of: made.data) == 2, "the animation was lost")
    }

    @Test("an animated sticker over the ceiling is refused, never redrawn as its first frame")
    func anAnimatedStickerOverTheCeilingIsRefused() {
        #expect(throws: StickerPack.MakeError.tooLarge) {
            try StickerPack.make(
                from: StickerFixtures.animatedWebP,
                maxBytes: StickerFixtures.animatedWebP.count - 1)
        }
    }

    @Test("an animated GIF is REFUSED in words, never flattened to its first frame")
    func anAnimatedGIFIsRefused() {
        let gif = StickerFixtures.animatedGIF
        #expect(StickerPack.frameCount(of: gif) == 2)
        #expect(StickerPack.isAnimated(gif))
        #expect(throws: StickerPack.MakeError.animatedNotWebP) {
            try StickerPack.make(from: gif, maxBytes: Self.ceiling)
        }
        #expect(!StickerPack.animatedNotWebPNotice.isEmpty)
    }

    @Test("an animated PNG within the ceiling goes up exactly as it is, every frame of it")
    func anAnimatedPNGWithinTheCeilingIsTakenAsGiven() throws {
        let apng = StickerFixtures.animatedPNG
        #expect(StickerPack.mime(of: apng) == "image/png")
        #expect(StickerPack.frameCount(of: apng) == 2)
        #expect(StickerPack.isAnimated(apng))
        #expect(apng.count < Self.ceiling)
        // A PNG within the ceiling is a finished sticker, and these bytes
        // travel untouched — nothing is flattened, so nothing is refused,
        // which is what the other clients do with the same file.
        let made = try StickerPack.make(from: apng, maxBytes: Self.ceiling)
        #expect(made.data == apng, "an animated PNG was re-encoded")
        #expect(made.mime == "image/png")
        #expect(StickerPack.frameCount(of: made.data) == 2, "the animation was lost")
        #expect(made.width == 8)
        #expect(made.height == 8)
    }

    @Test("an animated PNG OVER the ceiling is refused in words, never redrawn as its first frame")
    func anAnimatedPNGOverTheCeilingIsRefused() {
        // The redraw a still PNG gets here would keep frame zero alone; and
        // "too large" is not the sentence — a WebP of it is what would fit.
        let apng = StickerFixtures.animatedPNG
        #expect(throws: StickerPack.MakeError.animatedNotWebP) {
            try StickerPack.make(from: apng, maxBytes: apng.count - 1)
        }
        // At exactly the ceiling it still goes.
        #expect((try? StickerPack.make(from: apng, maxBytes: apng.count))?.data == apng)
    }

    @Test("several images in one file is not motion: a two-page TIFF is a still picture")
    func aMultiPageStillIsNotAnimated() throws {
        let tiff = StickerFixtures.twoPageTIFF
        #expect(StickerPack.frameCount(of: tiff) == 2)
        #expect(!StickerPack.isAnimated(tiff))
        let made = try StickerPack.make(from: tiff, maxBytes: Self.ceiling)
        #expect(made.mime == "image/png")
        // And the stills that ARE sticker types are not animated either.
        #expect(!StickerPack.isAnimated(StickerFixtures.stillWebP))
        #expect(!StickerPack.isAnimated(StickerFixtures.png(width: 8, height: 8)))
        #expect(StickerPack.isAnimated(StickerFixtures.animatedWebP))
    }

    @Test("a JPEG is never sent as a JPEG, and never loses its shape to the photo path")
    func aPhotographNeverGoesThroughThePhotoPath() throws {
        // 2000 × 500: `MediaPrep.preparePhoto` would answer with a JPEG;
        // a sticker is a PNG, whole, inside 512 × 512, its proportions kept.
        let made = try StickerPack.make(
            from: TestImages.photograph(width: 2000, height: 500), maxBytes: Self.ceiling)
        #expect(made.mime == "image/png")
        #expect(StickerPack.mime(of: made.data) == "image/png")
        #expect(made.width == 512)
        #expect(made.height == 128)
    }

    @Test("a PNG within the ceiling is taken as given whatever its pixel size")
    func aLargePNGWithinTheCeilingIsNotScaled() throws {
        // 1024 px of flat colour is a few kilobytes: far over 512 × 512 and
        // far under the byte ceiling, which is the only limit that binds a
        // picture that is already a sticker.
        let png = StickerFixtures.png(width: 1024, height: 1024)
        let made = try StickerPack.make(from: png, maxBytes: Self.ceiling)
        #expect(made.data == png)
        #expect(made.width == 1024)
    }

    @Test("a photograph becomes a PNG fitted whole into 512 × 512")
    func aPhotographIsFittedAndWrittenAsPNG() throws {
        let jpeg = TestImages.photograph(width: 1600, height: 1200)
        let made = try StickerPack.make(from: jpeg, maxBytes: 8 * 1024 * 1024)
        #expect(made.mime == "image/png")
        #expect(StickerPack.mime(of: made.data) == "image/png", "not a PNG — and never a JPEG")
        // Whole, never cropped: the longest edge lands on the box and the
        // other keeps the 4:3 it had.
        #expect(made.width == 512)
        #expect(made.height == 384)
    }

    @Test("a picture smaller than the box is not enlarged")
    func aSmallPictureKeepsItsSize() throws {
        let jpeg = TestImages.photograph(width: 120, height: 90)
        let made = try StickerPack.make(from: jpeg, maxBytes: Self.ceiling)
        #expect(made.width == 120)
        #expect(made.height == 90)
    }

    @Test("a still PNG too heavy to go as it is keeps its transparency when redrawn")
    func redrawingKeepsTheAlphaChannel() throws {
        let png = StickerFixtures.png(width: 1024, height: 1024)
        // A ceiling under the original forces the redraw path.
        let made = try StickerPack.make(from: png, maxBytes: png.count - 1)
        #expect(made.mime == "image/png")
        #expect(made.width == 512)
        #expect(made.data != png)
        // Left half opaque, right half a hole — not a picture on white.
        #expect(StickerFixtures.alpha(of: made.data, x: 10, y: 10) == 255)
        #expect(StickerFixtures.alpha(of: made.data, x: 500, y: 10) == 0,
                "the cut-out was flattened — the JPEG path's behaviour, which a sticker must not get")
    }

    @Test("a picture that cannot fit at any size is refused rather than sent too big")
    func aPictureThatNeverFitsIsRefused() {
        // Noise does not compress: even the smallest rung of the ladder is
        // far over a 2 KB ceiling.
        let noise = StickerFixtures.noisyPNG(width: 600, height: 600)
        #expect(throws: StickerPack.MakeError.tooLarge) {
            try StickerPack.make(from: noise, maxBytes: 2048)
        }
    }

    @Test("a photograph over the ceiling at 512 steps down until it fits")
    func theLadderFindsASizeThatFits() throws {
        let noise = StickerFixtures.noisyPNG(width: 600, height: 600)
        let atFull = try StickerPack.make(from: noise, maxBytes: 64 * 1024 * 1024)
        // A ceiling just under what 512 needs: the next rung down goes.
        let made = try StickerPack.make(from: noise, maxBytes: atFull.data.count * 600 / 1000)
        #expect((made.width ?? 0) < 512)
        #expect(made.data.count <= atFull.data.count * 600 / 1000)
    }

    @Test("bytes that are no picture at all are refused")
    func rubbishIsUnreadable() {
        #expect(throws: StickerPack.MakeError.unreadable) {
            try StickerPack.make(from: Data("hello".utf8), maxBytes: Self.ceiling)
        }
    }

    @Test("a label is optional and trimmed; spaces alone are no label")
    func labels() {
        #expect(StickerPack.label("  party cat \n") == .text("party cat"))
        #expect(StickerPack.label("   ") == StickerPack.Label.none)
        #expect(StickerPack.label("") == StickerPack.Label.none)
        #expect(StickerPack.label(nil) == StickerPack.Label.none)
    }

    @Test("over 64 is REFUSED, never cut: a label is fixed once added")
    func anOverLongLabelIsRefused() {
        let exact = String(repeating: "я", count: 64)
        #expect(StickerPack.label(exact) == .text(exact))
        #expect(StickerPack.label(exact + "я") == .tooLong)
        // The 64 are counted AFTER trimming, as the server trims before it
        // counts: spaces around a label that fits do not make it too long.
        #expect(StickerPack.label("   " + exact + " \n\t") == .text(exact))
        // And the refusal has words.
        #expect(!StickerPack.labelTooLongNotice.isEmpty)
    }

    @Test("the label's 64 are the server's: Unicode scalars, not what the eye counts")
    func labelLimitCountsScalars() {
        // The server measures `label.chars().count()`. A family emoji is
        // ONE character to Swift and seven scalars to the server, so a
        // label well under 64 characters can be one the server refuses.
        let family = "👨‍👩‍👧‍👦"
        #expect(family.count == 1)
        #expect(family.unicodeScalars.count == 7)

        let typed = String(repeating: "a", count: 55) + family + family
        #expect(typed.count == 57, "within a limit counted in characters")
        #expect(StickerPack.labelLength(typed) == 69)
        #expect(StickerPack.label(typed) == .tooLong)
        // One family fewer is 62 scalars, and goes whole.
        let fits = String(repeating: "a", count: 55) + family
        #expect(StickerPack.label(fits) == .text(fits))

        // Flags, two scalars each: 32 are exactly 64 and the 33rd is over.
        #expect(StickerPack.label(String(repeating: "🇷🇸", count: 32))
            == .text(String(repeating: "🇷🇸", count: 32)))
        #expect(StickerPack.label(String(repeating: "🇷🇸", count: 33)) == .tooLong)
        // A decomposed é is two scalars to the server, and so here.
        #expect(StickerPack.label(String(repeating: "e\u{301}", count: 32)) != .tooLong)
        #expect(StickerPack.label(String(repeating: "e\u{301}", count: 33)) == .tooLong)
    }

    @Test("what is trimmed is what the server trims: White_Space, and nothing else")
    func labelTrimIsTheServers() {
        // A no-break space and an ideographic space are White_Space.
        #expect(StickerPack.label("\u{00A0}\u{3000}cat\u{2028}") == .text("cat"))
        // A ZERO WIDTH SPACE is not — the server's `trim()` leaves it and
        // counts it, where Foundation's whitespace set would strip it and
        // the two would disagree about a label at the limit by one.
        let padded = "\u{200B}" + String(repeating: "a", count: 64)
        #expect(StickerPack.label(padded) == .tooLong)
        #expect(StickerPack.label("\u{200B}cat") == .text("\u{200B}cat"))
    }

    // MARK: - Making one, off the main actor

    @MainActor
    @Test("a picked picture is read and made into a sticker off the main actor")
    func preparingRunsOffTheMainActor() async throws {
        let ranOnMain = MainThreadProbe()
        let source = TestImages.photograph(width: 1200, height: 900)

        // Called from the main actor, exactly as the view calls it.
        let result = await StickerPack.prepare(maxBytes: Self.ceiling) {
            ranOnMain.note()
            return source
        }

        // The read is awaited INSIDE `prepare`, so where it ran is where
        // the decode and the PNG encodes that follow it run.
        #expect(ranOnMain.value == false, "a 48 MP photograph would freeze the screen")
        let prepared = try result.get()
        // The same sticker `make` gives — moving the work moved nothing else.
        #expect(prepared.made == (try StickerPack.make(from: source, maxBytes: Self.ceiling)))
        let still = try #require(prepared.still)
        #expect(max(still.width, still.height) <= StickerPack.maxEdge)
    }

    @Test("what could not be read, what is too big, and what moves without being a WebP, each come back as itself")
    func preparingReportsItsFailures() async {
        let gif = StickerFixtures.animatedGIF
        let moving = await StickerPack.prepare(maxBytes: Self.ceiling) { gif }
        #expect(throws: StickerPack.MakeError.animatedNotWebP) { try moving.get() }
        let unread = await StickerPack.prepare(maxBytes: Self.ceiling) { nil }
        #expect(throws: StickerPack.MakeError.unreadable) { try unread.get() }
        let notAPicture = await StickerPack.prepare(maxBytes: Self.ceiling) { Data("hello".utf8) }
        #expect(throws: StickerPack.MakeError.unreadable) { try notAPicture.get() }
        // An animated sticker over the ceiling is refused, never redrawn.
        let animated = StickerFixtures.animatedWebP
        let tooBig = await StickerPack.prepare(maxBytes: animated.count - 1) { animated }
        #expect(throws: StickerPack.MakeError.tooLarge) { try tooBig.get() }
    }

    // MARK: - Who may do what

    @MainActor
    @Test("a cached sticker gives its bytes, whether it moves, and its first frame — from the file, by the @concurrent readers")
    func theCachedFileReaders() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("sticker-offmain-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let moving = directory.appendingPathComponent("moving")
        let still = directory.appendingPathComponent("still")
        try StickerFixtures.animatedWebP.write(to: moving)
        try StickerFixtures.png(width: 900, height: 300).write(to: still)

        // Only a sticker that MOVES comes back to be animated.
        #expect(await StickerPack.animatedBytes(fileAt: moving) == StickerFixtures.animatedWebP)
        #expect(await StickerPack.animatedBytes(fileAt: still) == nil)
        #expect(await StickerPack.bytes(fileAt: still) == StickerFixtures.png(width: 900, height: 300))
        #expect(await StickerPack.bytes(fileAt: directory.appendingPathComponent("absent")) == nil)

        // Frame zero, decoded there, no larger than asked for.
        let decoded = try #require(await StickerPack.still(fileAt: still, maxPixels: 300))
        #expect(max(decoded.image.width, decoded.image.height) <= 300)
        #expect(await StickerPack.still(fileAt: directory.appendingPathComponent("absent"), maxPixels: 300) == nil)

        // Bytes from the server are cached only when they are a picture.
        let fresh = directory.appendingPathComponent("fresh")
        #expect(await StickerPack.still(storing: Data("no picture".utf8), at: fresh, maxPixels: 300) == nil)
        #expect(!FileManager.default.fileExists(atPath: fresh.path))
        #expect(await StickerPack.still(storing: StickerFixtures.stillWebP, at: fresh, maxPixels: 300) != nil)
        #expect(try Data(contentsOf: fresh) == StickerFixtures.stillWebP, "the cache must hold the ORIGINAL bytes")
    }

    // MARK: - Where the sticker button is

    private func door(
        _ kind: String?, offers: Bool = true, hasAssistant: Bool = true,
        processor: String? = "Example AI", agreed: Bool = false
    ) -> StickerDoor {
        StickerDoor.of(
            offersStickers: offers, chatKind: kind, hasAssistant: hasAssistant,
            processor: processor, agreedAt: agreed ? Date(timeIntervalSince1970: 0) : nil)
    }

    @Test("the button is offered in every chat a message can be sent in")
    func theButtonIsEverywhere() {
        // The family chat and a one-to-one: a sticker has no body to hold
        // an `@ai`, so nothing there reaches the model and nothing is asked.
        #expect(door("family") == .open)
        #expect(door("direct") == .open)
        #expect(door("family", agreed: true) == .open)
        // The assistant's own chat — once this member has agreed.
        #expect(door("ai", agreed: true) == .open)
        // A thread asks the same question with its chat's kind.
    }

    @Test("in the assistant's chat a sticker goes THROUGH the consent question, never around it")
    func theAssistantChatAsksFirst() {
        #expect(door("ai") == .asksFirst)
        // It is the consent rule's own answer for a message with no words,
        // so the two cannot come apart.
        #expect(AssistantConsent.isRequired(
            chatKind: "ai", body: "", processor: "Example AI", agreedAt: nil))
        // A server whose assistant names nobody: no honest way to ask, so
        // no button — exactly as it takes no words there.
        #expect(door("ai", processor: nil) == .absent)
        #expect(door("ai", processor: "  ") == .absent)
        // The other chats on that server are untouched.
        #expect(door("family", processor: nil) == .open)
    }

    @Test("a server that predates the pack has no sticker button anywhere")
    func noPackNoButton() {
        for kind in ["family", "direct", "ai"] {
            #expect(door(kind, offers: false, agreed: true) == .absent)
        }
    }

    @Test("a sticker is drawn in a 160-point box, and the device remembers its last 16")
    func theAgreedNumbers() {
        #expect(StickerPack.messageBox == 160)
        #expect(StickerRecents.limit == 16)
        #expect(StickerPack.labelLimit == 64)
        #expect(StickerPack.maxEdge == 512)
        // Seventeen sent: the first falls off, the newest is first.
        let sent = (1...17).reduce([Int64]()) { StickerRecents.noting(Int64($1), in: $0) }
        #expect(sent == (2...17).reversed().map(Int64.init))
    }

    @Test("whoever added a sticker, or the family owner, may remove it — and nobody else")
    func removalIsForTheAdderOrTheOwner() {
        #expect(StickerPack.canRemove(addedBy: 7, currentUserID: 7, isOwner: false))
        #expect(StickerPack.canRemove(addedBy: 9, currentUserID: 7, isOwner: true))
        #expect(!StickerPack.canRemove(addedBy: 9, currentUserID: 7, isOwner: false))
    }

    @Test("a full pack has no room; an unknown limit leaves it to the server")
    func room() {
        #expect(StickerPack.hasRoom(count: 199, limit: 200))
        #expect(!StickerPack.hasRoom(count: 200, limit: 200))
        #expect(!StickerPack.hasRoom(count: 250, limit: 200), "a lowered limit freezes the pack")
        #expect(StickerPack.hasRoom(count: 5000, limit: nil))
    }

    // MARK: - Does the pack hold it?

    private func item(_ id: Int64, mime: String = "image/webp", size: Int64) -> PackItemSnapshot {
        PackItemSnapshot(id: id, addedBy: 7, attachmentID: 100 + id, mime: mime, size: size)
    }

    @Test("the pack holds a sticker when size, type AND bytes all match")
    func holdsNeedsTheBytes() {
        let bytes = StickerFixtures.stillWebP
        let other = StickerFixtures.animatedWebP
        let items = [item(1, size: Int64(other.count)), item(2, size: Int64(bytes.count))]
        let cache: [Int64: Data] = [101: other, 102: bytes]

        #expect(StickerPack.holds(
            bytes: bytes, mime: "image/webp", size: Int64(bytes.count),
            items: items, bytesOf: { cache[$0.attachmentID] }))
        // The same size and type with different bytes is a different sticker.
        let impostor = Data(bytes.reversed())
        #expect(!StickerPack.holds(
            bytes: impostor, mime: "image/webp", size: Int64(bytes.count),
            items: items, bytesOf: { cache[$0.attachmentID] }))
        // The same bytes under another type do not match either.
        #expect(!StickerPack.holds(
            bytes: bytes, mime: "image/png", size: Int64(bytes.count),
            items: items, bytesOf: { cache[$0.attachmentID] }))
    }

    @Test("an item whose bytes are not on this device counts as not held, so the offer stays up")
    func holdsIsNotGuessedFromSizeAlone() {
        let bytes = StickerFixtures.stillWebP
        #expect(!StickerPack.holds(
            bytes: bytes, mime: "image/webp", size: Int64(bytes.count),
            items: [item(2, size: Int64(bytes.count))], bytesOf: { _ in nil }))
    }

    @Test("a pending message knows no size, and the bytes decide alone")
    func holdsWithoutASize() {
        let bytes = StickerFixtures.stillWebP
        #expect(StickerPack.holds(
            bytes: bytes, mime: "image/webp", size: nil,
            items: [item(2, size: Int64(bytes.count))], bytesOf: { _ in bytes }))
    }

    // MARK: - Recents

    @Test("the sticker just sent moves to the front, once, and the tail falls off")
    func recentsOrder() {
        #expect(StickerRecents.noting(3, in: [1, 2, 3, 4]) == [3, 1, 2, 4])
        #expect(StickerRecents.noting(9, in: []) == [9])
        let full = Array(Int64(1)...Int64(StickerRecents.limit))
        let after = StickerRecents.noting(99, in: full)
        #expect(after.count == StickerRecents.limit)
        #expect(after.first == 99)
        #expect(!after.contains(Int64(StickerRecents.limit)))
    }

    @Test("a recently used sticker that has left the pack is simply not shown")
    func recentsSkipRemovedItems() {
        let items = [item(1, size: 1), item(2, size: 1), item(3, size: 1)]
        let recent = StickerRecents.recent(of: items, recents: [3, 77, 1])
        #expect(recent.map(\.id) == [3, 1])
    }

    // MARK: - What a sticker message is

    private func attachment(sticker: Bool, kind: String = "photo") -> AttachmentDTO {
        AttachmentDTO(
            id: 90, kind: kind, mime: "image/webp", size: 38, width: 8, height: 8,
            durationMS: nil, hasPreview: false, name: nil, latitude: nil, longitude: nil,
            accuracyM: nil, sticker: sticker)
    }

    private func message(
        body: String = "", attachments: [AttachmentDTO], replyTo: ReplyToSnapshot? = nil
    ) -> MessageSnapshot {
        MessageSnapshot(
            localID: "s:1", serverID: 1, chatID: 42, senderID: 9, body: body,
            createdAt: Date(timeIntervalSince1970: 0), state: .sent,
            replyTo: replyTo, attachment: attachments.first, attachments: attachments)
    }

    @Test("one flagged picture with no words is a sticker — and so draws with no bubble")
    func aStickerMessage() {
        #expect(MessagePresentation.isSticker(message(attachments: [attachment(sticker: true)])))
    }

    @Test("a sticker that is a reply is still a sticker")
    func aStickerReplyStaysBare() {
        let quote = ReplyToSnapshot(messageID: 5, senderID: 7, excerpt: "See you at six", parent: nil)
        let reply = message(attachments: [attachment(sticker: true)], replyTo: quote)
        #expect(MessagePresentation.isSticker(reply))
        // The photo rule keeps the balloon for a quote; the sticker rule is
        // what takes it away.
        #expect(!MessagePresentation.isMediaOnly(reply))
    }

    @Test("a photo sent before stickers existed is a photo, exactly as it was")
    func anOldPhotoIsNotASticker() {
        let old = message(attachments: [attachment(sticker: false)])
        #expect(!MessagePresentation.isSticker(old))
        #expect(MessagePresentation.isMediaOnly(old))
    }

    @Test("the test is exactly three conditions: ONE attachment, a photo, carrying the flag")
    func theOneTestEveryClientRuns() {
        // A second attachment, flagged or not: not a sticker.
        #expect(!MessagePresentation.isSticker(
            message(attachments: [attachment(sticker: true), attachment(sticker: true)])))
        #expect(!MessagePresentation.isSticker(
            message(attachments: [attachment(sticker: true), attachment(sticker: false)])))
        // No attachment at all.
        #expect(!MessagePresentation.isSticker(message(attachments: [])))
        // The flag on something that is not a photo.
        for kind in ["video", "audio", "file", "location"] {
            #expect(!MessagePresentation.isSticker(
                message(attachments: [attachment(sticker: true, kind: kind)])),
                "a flagged \(kind) was drawn as a sticker")
        }
        // A photo without the flag.
        #expect(!MessagePresentation.isSticker(message(attachments: [attachment(sticker: false)])))

        // And NOTHING ELSE is asked. The server refuses a body beside the
        // flag, so this message does not exist — but the test every client
        // runs has three conditions, and a fourth here is this client
        // drawing by a different rule from the other four.
        #expect(MessagePresentation.isSticker(
            message(body: "look", attachments: [attachment(sticker: true)])))
    }

    @Test("Edit is never offered on a sticker, and still is on the sender's own words and photos")
    func aStickerOffersNoEdit() {
        // The fixtures are sent by member 9 and delivered (server id 1).
        let sticker = message(attachments: [attachment(sticker: true)])
        #expect(!MessagePresentation.offersEdit(sticker, currentUserID: 9),
                "the server answers a sticker's PATCH with `validation`")
        let quote = ReplyToSnapshot(messageID: 5, senderID: 7, excerpt: "See you at six", parent: nil)
        #expect(!MessagePresentation.offersEdit(
            message(attachments: [attachment(sticker: true)], replyTo: quote), currentUserID: 9))

        // The controls: what cannot be edited is a STICKER, not a WebP and
        // not a message with a picture in it.
        #expect(MessagePresentation.offersEdit(message(body: "hello", attachments: []), currentUserID: 9))
        #expect(MessagePresentation.offersEdit(
            message(attachments: [attachment(sticker: false)]), currentUserID: 9))
        // And the rule it replaced is intact: only the author, only once acked.
        #expect(!MessagePresentation.offersEdit(message(body: "hello", attachments: []), currentUserID: 7))
        let pending = MessageSnapshot(
            localID: "p:1", serverID: nil, chatID: 42, senderID: 9, body: "hello",
            createdAt: Date(timeIntervalSince1970: 0), state: .pending)
        #expect(!MessagePresentation.offersEdit(pending, currentUserID: 9))
    }

    @Test("a chat-list row says Sticker where a photo's says Photo")
    func chatListPreview() {
        #expect(ChatSyncCoordinator.preview(body: "", attachment: attachment(sticker: true))
            == String(localized: "Sticker"))
        #expect(ChatSyncCoordinator.preview(body: "", attachment: attachment(sticker: false))
            == String(localized: "Photo"))
    }

    // MARK: - Animation

    /// ImageIO animates an animated WebP on this platform — the claim the
    /// chat's moving stickers rest on. A still one never starts.
    @MainActor
    @Test("an animated WebP produces frames; a still one produces none", .timeLimit(.minutes(1)))
    func thePlayerAnimates() async {
        let still = StickerPlayer()
        still.play(StickerFixtures.stillWebP)
        #expect(still.frame == nil)

        let moving = StickerPlayer()
        moving.play(StickerFixtures.animatedWebP)
        // The block runs on the main run loop; give it turns to run in.
        for _ in 0..<200 where moving.frame == nil {
            try? await Task.sleep(for: .milliseconds(10))
        }
        #expect(moving.frame != nil, "ImageIO delivered no frame of an animated WebP")
        #expect(moving.frame?.width == 8)
        moving.stop()
    }
}

/// Records whether it was called on the main thread — from inside a
/// `@Sendable` closure, which a plain `var` cannot be written from.
private final class MainThreadProbe: @unchecked Sendable {
    private let lock = NSLock()
    private var seen: Bool?
    nonisolated func note() {
        let onMain = Thread.isMainThread
        lock.lock(); defer { lock.unlock() }
        seen = onMain
    }
    nonisolated var value: Bool? {
        lock.lock(); defer { lock.unlock() }
        return seen
    }
}
