//
//  StickerPack.swift
//  FamilyConnect
//
//  The rules of the family's sticker pack that need no network and no
//  store (docs/protocol.md, "Sticker pack"): what bytes may be a sticker,
//  how a picture becomes one, who may remove one, whether the pack already
//  holds one, and which the panel shows first.
//
//  Everything here is a pure function of its arguments, which is the
//  point: the coordinator does the requests and the views do the drawing,
//  and the five clients have to agree on exactly these answers.
//
//  THE ONE RULE NOT TO LOSE. A sticker is NEVER prepared. `MediaPrep` turns
//  every photograph into a JPEG on its way out, and a JPEG has no
//  transparency and one frame — so nothing in this file, and nothing that
//  calls it, goes near `MediaPrep.preparePhoto`. A finished `.webp` or
//  `.png` is taken as given, byte for byte; only a picture that is NOT yet
//  a sticker is redrawn, and then as a PNG, because an Apple device
//  decodes a WebP and cannot write one.
//

import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

nonisolated enum StickerPack {

    /// The box a client-made sticker is fitted into — the CLIENT's rule,
    /// the server never decodes an image (docs/protocol.md, "What a sticker
    /// is made of").
    static let maxEdge = 512

    /// The longest label the server takes; longer is `validation`.
    ///
    /// In UNICODE SCALARS, because that is what the server counts
    /// (`label.chars().count()`), and not in the Characters a Swift String
    /// counts by default: a family emoji is one of those and seven of
    /// these. Measured with `labelLength`, never with `.count`.
    static let labelLimit = 64

    /// The side of the box a sticker MESSAGE is drawn in, in points — one
    /// fixed box for every sticker on this client, fitted whole: larger
    /// than the biggest emoji on the EmojiOnly ladder and smaller than a
    /// photograph's 240pt tile. Never the picture's own pixel size, which
    /// would make a 96-pixel sticker a speck and a 2000-pixel one a poster.
    static let messageBox: CGFloat = 160

    // MARK: - What the bytes are

    /// `image/webp`, `image/png`, or nil for anything else — judged from
    /// the BYTES, never from a file extension, with the server's own magic
    /// numbers: a PNG's eight-byte signature, or `RIFF` at offset 0 and
    /// `WEBP` at offset 8 (the four bytes between are a length and are not
    /// checked).
    static func mime(of data: Data) -> String? {
        let head = [UInt8](data.prefix(12))
        if head.starts(with: [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
            return "image/png"
        }
        if head.count >= 12,
           Array(head[0..<4]) == Array("RIFF".utf8),
           Array(head[8..<12]) == Array("WEBP".utf8) {
            return "image/webp"
        }
        return nil
    }

    /// How many frames ImageIO finds — 1 for a still, more for an animated
    /// WebP (or an animated PNG), 0 for bytes it cannot read at all.
    static func frameCount(of data: Data) -> Int {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return 0 }
        return CGImageSourceGetCount(source)
    }

    /// The container types in which more than one frame means MOTION. A
    /// multi-page TIFF and an icon file holding six sizes also report
    /// several "images" and move nowhere; those are still pictures whose
    /// first image is the picture.
    private static let animatedTypes: Set<String> = [
        "org.webmproject.webp", "public.png", "com.compuserve.gif",
        "public.heics", "public.avis",
    ]

    /// Does this picture MOVE? An animated WebP, an animated GIF, an
    /// animated PNG, an image sequence — judged from the bytes, like
    /// everything else here.
    static func isAnimated(_ data: Data) -> Bool {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              CGImageSourceGetCount(source) > 1,
              let type = CGImageSourceGetType(source) as String?
        else { return false }
        return animatedTypes.contains(type)
    }

    /// The picture's own pixel size, read from its header without decoding
    /// it — what rides along as `width` and `height` on the upload.
    static func pixelSize(of data: Data) -> (width: Int, height: Int)? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil)
                as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0
        else { return nil }
        return (width, height)
    }

    // MARK: - Making one

    /// A sticker ready to upload: the bytes exactly as they will travel.
    struct Made: Equatable, Sendable {
        let data: Data
        let mime: String
        let width: Int?
        let height: Int?
    }

    enum MakeError: Error, Equatable {
        /// Not a picture this device can read.
        case unreadable
        /// It MOVES, it is not a WebP, and it cannot go as it is — an
        /// animated GIF, an animated PNG over the ceiling. Refused in words
        /// rather than made into a sticker: the only thing this device
        /// could make of it is its first frame, and a picture that silently
        /// stopped moving is not the one that was picked.
        case animatedNotWebP
        /// Over the per-item ceiling, and nothing may be done about it: an
        /// animated sticker cannot be re-encoded, and a still one did not
        /// fit even when redrawn smaller.
        case tooLarge
    }

    /// The sentence for `MakeError.animatedNotWebP`.
    static var animatedNotWebPNotice: String {
        String(localized: "An animated sticker has to be a WebP picture.")
    }

    /// The sizes a picture that is being MADE into a sticker is tried at,
    /// largest first. 512 is the rule; the smaller rungs exist because the
    /// only thing this platform can write is PNG, and a 512-pixel
    /// photograph as a PNG can run past a 512 KiB ceiling on its own. A
    /// smaller sticker that goes is better than a refusal.
    static let edgeLadder = [512, 448, 384, 320, 256, 192]

    /// Turn what somebody picked into a sticker (docs/protocol.md, "What a
    /// sticker is made of").
    ///
    /// - A `.webp` or `.png` within the ceiling is taken AS GIVEN, whatever
    ///   its pixel size and whether or not it moves: re-encoding somebody's
    ///   finished sticker buys nothing and, for an animated one, is not
    ///   possible. That includes an animated PNG — its bytes travel
    ///   untouched, so nothing is flattened, and every client of this
    ///   protocol takes the same file.
    /// - An ANIMATED picture that cannot go as given is never redrawn: that
    ///   would silently keep frame zero and call it the same sticker. A
    ///   WebP over the ceiling is refused as too big, which is all that is
    ///   wrong with it; anything else that moves — an animated GIF, an
    ///   animated PNG over the ceiling — is refused with a sentence saying
    ///   what an animated sticker has to be.
    /// - Any other STILL picture — a JPEG, a HEIC, a still PNG too heavy to
    ///   go as it is — is fitted WHOLE into 512 × 512, its proportions and
    ///   its transparency kept, and written as PNG.
    static func make(from data: Data, maxBytes: Int) throws -> Made {
        guard frameCount(of: data) > 0 else { throw MakeError.unreadable }
        let mime = mime(of: data)

        if let mime, data.count <= maxBytes {
            let size = pixelSize(of: data)
            return Made(data: data, mime: mime, width: size?.width, height: size?.height)
        }
        if isAnimated(data) {
            throw mime == "image/webp" ? MakeError.tooLarge : MakeError.animatedNotWebP
        }

        for edge in edgeLadder {
            guard let image = fitted(data, maxEdge: edge) else { throw MakeError.unreadable }
            guard let png = pngData(from: image) else { throw MakeError.unreadable }
            if png.count <= maxBytes {
                return Made(data: png, mime: "image/png", width: image.width, height: image.height)
            }
            // A picture already smaller than this rung comes back the same
            // size from every rung below it, so trying them is the same
            // failure again.
            if max(image.width, image.height) < edge { break }
        }
        throw MakeError.tooLarge
    }

    /// A sticker that has been made, and the one frame a sheet shows of it
    /// while somebody types its label.
    struct Prepared: @unchecked Sendable {
        let made: Made
        /// Frame zero, no larger than the sticker box. `CGImage` is
        /// immutable, which is what the `@unchecked` above rests on.
        let still: CGImage?
    }

    /// The bytes of a file somebody picked — the person's own, reachable
    /// only inside its security scope and only for as long as it is held,
    /// so the scope is opened and closed around the read, on whatever
    /// thread the read is on.
    static func read(fileAt url: URL) -> Data? {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        return try? Data(contentsOf: url)
    }

    /// Read what was picked and make a sticker of it, OFF the caller's
    /// actor.
    ///
    /// `@concurrent`, because this target builds with
    /// `NonisolatedNonsendingByDefault`: a plain `nonisolated async`
    /// function runs on its CALLER's actor, which for the view that calls
    /// this is the main one — and `make` is a full decode of whatever was
    /// picked plus up to six PNG encodes (`MediaPrep.preparePhoto` says the
    /// same of its own work, for the same reason).
    ///
    /// A `Result` rather than `throws`, so the three things there are to
    /// say — too big, moving and not a WebP, and not a picture — are the
    /// only three a caller can get.
    @concurrent
    static func prepare(
        maxBytes: Int, read: @Sendable () async -> Data?
    ) async -> Result<Prepared, MakeError> {
        guard let data = await read() else { return .failure(.unreadable) }
        do {
            let made = try make(from: data, maxBytes: maxBytes)
            return .success(Prepared(made: made, still: fitted(made.data, maxEdge: maxEdge)))
        } catch let error as MakeError {
            return .failure(error)
        } catch {
            return .failure(.unreadable)
        }
    }

    /// Frame zero, no larger than `maxEdge` on its longest side, the right
    /// way up. A picture already within the box is NOT enlarged: ImageIO's
    /// thumbnail never scales up, which is what "fits into" means.
    private static func fitted(_ data: Data, maxEdge: Int) -> CGImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        return CGImageSourceCreateThumbnailAtIndex(source, 0, [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            // EXIF orientation, so a portrait photograph is not a sticker
            // lying on its side.
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxEdge,
        ] as CFDictionary)
    }

    /// PNG, alpha and all. NOT `PlatformImage.jpegData`, which flattens
    /// onto white by design — the exact thing a sticker must not have
    /// happen to it.
    private static func pngData(from image: CGImage) -> Data? {
        let output = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(
            output, UTType.png.identifier as CFString, 1, nil)
        else { return nil }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else { return nil }
        return output as Data
    }

    /// A label's length as the server will measure it
    /// (`label.chars().count()`): in Unicode scalar values.
    static func labelLength(_ text: String) -> Int {
        text.unicodeScalars.count
    }

    /// What somebody typed into the label field, judged.
    enum Label: Equatable {
        /// Nothing, or nothing but spaces: an empty label is no label.
        case none
        /// A label, trimmed — exactly what is sent.
        case text(String)
        /// Over the server's 64. REFUSED, in words, before any request.
        case tooLong
    }

    /// The label as the server will see it: TRIMMED first, then counted in
    /// Unicode scalars, and refused when that is over `labelLimit`.
    ///
    /// Refused, not cut. The field used to hold itself to the limit on
    /// every keystroke and the send cut whatever was left over — so a
    /// description pasted in arrived shorter than it was written, with
    /// nothing said, and it is fixed once added: there is no edit. The
    /// server's answer to an over-long label is `validation`; this is that
    /// answer given where the person is still typing, and before a picture
    /// has been uploaded for nothing.
    ///
    /// Trimmed by the Unicode `White_Space` property, scalar by scalar,
    /// which is what the server's `trim()` removes — so the two count the
    /// same string.
    static func label(_ raw: String?) -> Label {
        guard let raw else { return .none }
        var scalars = Substring.UnicodeScalarView(raw.unicodeScalars)
        while let first = scalars.first, first.properties.isWhitespace { scalars.removeFirst() }
        while let last = scalars.last, last.properties.isWhitespace { scalars.removeLast() }
        guard !scalars.isEmpty else { return .none }
        guard scalars.count <= labelLimit else { return .tooLong }
        return .text(String(scalars))
    }

    /// The sentence for `Label.tooLong`.
    static var labelTooLongNotice: String {
        String(localized: "A description can be at most 64 characters.")
    }

    // MARK: - Reading and decoding, off the main actor

    /// One decoded frame on its way back to the main actor. `CGImage` is
    /// immutable, which is what the `@unchecked` rests on.
    struct Still: @unchecked Sendable {
        let image: CGImage
    }

    /// The bytes of a cached file, read OFF the caller's actor — a sticker
    /// is up to half a megabyte, a panel holds two hundred of them, and a
    /// view's body is not where any of that is read.
    @concurrent
    static func bytes(fileAt url: URL) async -> Data? {
        let data = try? Data(contentsOf: url)
        return data?.isEmpty == false ? data : nil
    }

    /// The same bytes, and only when they MOVE — what a view that animates
    /// asks for, so a still sticker costs it a header and no more.
    @concurrent
    static func animatedBytes(fileAt url: URL) async -> Data? {
        guard let data = try? Data(contentsOf: url), isAnimated(data) else { return nil }
        return data
    }

    /// Frame zero of a cached sticker, read and decoded OFF the caller's
    /// actor. nil when the file is not there or is not a picture.
    @concurrent
    static func still(fileAt url: URL, maxPixels: Int) async -> Still? {
        guard let data = try? Data(contentsOf: url), !data.isEmpty,
              let image = PlatformImage.decode(data, maxPixels: maxPixels)
        else { return nil }
        return Still(image: image)
    }

    /// Bytes fresh from the server: decoded, and written to the cache only
    /// when they ARE a picture — both off the caller's actor.
    @concurrent
    static func still(storing data: Data, at url: URL, maxPixels: Int) async -> Still? {
        guard let image = PlatformImage.decode(data, maxPixels: maxPixels) else { return nil }
        try? data.write(to: url, options: .atomic)
        return Still(image: image)
    }

    // MARK: - Who may do what

    /// May this person remove that item? Whoever added it, or the family's
    /// owner — a permission shape the board does not have, because a
    /// member who has left leaves their stickers behind and under an
    /// author-only rule nobody could ever take those down. The server
    /// enforces it (`not_pack_item_author`); this decides whether the
    /// action is OFFERED.
    static func canRemove(addedBy: Int64, currentUserID: Int64, isOwner: Bool) -> Bool {
        isOwner || addedBy == currentUserID
    }

    /// Is there room for one more? Asked where somebody is choosing a
    /// picture, so the refusal arrives beside the picker rather than as a
    /// rejected request. nil limit = unknown, and the server decides.
    static func hasRoom(count: Int, limit: Int?) -> Bool {
        guard let limit else { return true }
        return count < limit
    }

    // MARK: - Does the pack hold it?

    /// Does the family's pack already hold this sticker? Decided from bytes
    /// this device already has: an item whose size and type match AND whose
    /// bytes are the same (docs/protocol.md, "Sending one").
    ///
    /// Nothing on the wire names the item a message was sent from. A wrong
    /// "no" costs one offer the server answers `200` to; a wrong "yes"
    /// would hide the only way to keep a sticker the pack has lost, so an
    /// item whose bytes are not cached here counts as NOT a match.
    ///
    /// `size` is nil for a message this device has not got a server copy of
    /// yet (its own pending row); then the bytes decide alone.
    static func holds(
        bytes: Data,
        mime: String,
        size: Int64?,
        items: [PackItemSnapshot],
        bytesOf: (PackItemSnapshot) -> Data?
    ) -> Bool {
        items.contains { item in
            guard item.mime == mime else { return false }
            if let size, size > 0, item.size > 0, item.size != size { return false }
            return bytesOf(item) == bytes
        }
    }

    /// The same question asked of the CACHE, off the caller's actor: the
    /// message's bytes and each candidate's are files, and comparing them
    /// is up to two hundred reads that a sheet's first frame must not wait
    /// for. nil when the message's own bytes are not on this device yet —
    /// "still finding out", which is not an answer either way.
    @concurrent
    static func holds(
        fileAt url: URL,
        mime: String,
        size: Int64?,
        items: [PackItemSnapshot],
        fileOf: @Sendable (PackItemSnapshot) -> URL
    ) async -> Bool? {
        guard let bytes = try? Data(contentsOf: url), !bytes.isEmpty else { return nil }
        return holds(bytes: bytes, mime: mime, size: size, items: items) { item in
            try? Data(contentsOf: fileOf(item))
        }
    }
}

/// What the sticker button is, in one chat: there or not, and whether the
/// person is asked something first.
///
/// One rule for every composer this client has — the phone's, the Mac's
/// and the thread's — so the three cannot come to disagree about it
/// (docs/protocol.md, "Sending one": "a client offers the sticker button
/// in EVERY place a message can be written: the family chat, a one-to-one
/// chat, the assistant's chat, and the composer of a thread").
nonisolated enum StickerDoor: Equatable {
    /// No button. A server that predates the pack, or a chat in which
    /// nothing at all may be sent.
    case absent
    /// One tap sends.
    case open
    /// The ASSISTANT's chat, and this member has not yet agreed that what
    /// they send there goes to the model. The button is there; the send
    /// raises the consent question first and goes when it is answered yes.
    case asksFirst

    /// A sticker is a message with no words, so it is asked of
    /// `AssistantConsent` exactly as an empty body would be — THROUGH the
    /// consent rule and never beside it. That makes the assistant's own
    /// chat the one place it can matter (everything sent there reaches the
    /// model; in the family chat only an `@ai` does, and a sticker has no
    /// body to hold one), and makes the answer change with that rule
    /// rather than with a copy of it.
    ///
    /// A server whose assistant names no processor gets NO button in that
    /// chat, for the reason it gets no words there: there is no honest way
    /// to ask, so there is nothing to send (`isWithheldFromAnUnnamedAssistant`).
    static func of(
        offersStickers: Bool,
        chatKind: String?,
        hasAssistant: Bool,
        processor: String?,
        agreedAt: Date?
    ) -> StickerDoor {
        guard offersStickers else { return .absent }
        if AssistantConsent.isWithheldFromAnUnnamedAssistant(
            chatKind: chatKind, body: "", hasAssistant: hasAssistant, processor: processor)
        {
            return .absent
        }
        if AssistantConsent.isRequired(
            chatKind: chatKind, body: "", processor: processor, agreedAt: agreedAt)
        {
            return .asksFirst
        }
        return .open
    }
}

/// Which stickers this device used most recently — a panel puts them
/// first. The device's own business and never on the wire
/// (docs/protocol.md, "The pack").
nonisolated enum StickerRecents {

    /// How many are remembered: two rows of the panel on a phone.
    static let limit = 16

    /// The list after `id` was just sent: it moves to the front, appears
    /// once, and the tail falls off.
    static func noting(_ id: Int64, in recents: [Int64]) -> [Int64] {
        Array(([id] + recents.filter { $0 != id }).prefix(limit))
    }

    /// The recently used items that are STILL in the pack, most recent
    /// first. An item somebody has since removed simply is not here — the
    /// remembered id stays harmlessly in the list, since ids are never
    /// reused and it can never come to mean a different sticker.
    static func recent(of items: [PackItemSnapshot], recents: [Int64]) -> [PackItemSnapshot] {
        let byID = Dictionary(items.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        return recents.compactMap { byID[$0] }
    }
}
