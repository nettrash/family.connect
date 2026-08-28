//
//  AttachmentAlbumTests.swift
//  FamilyConnectTests
//
//  Pins how a message's attachments are cut for drawing. The media go
//  into the pile and everything else stays a row, each half in SENT order
//  — a reader who sent "photo, receipt, photo" must find both photos in
//  the pile and the receipt under it, not a pile with a gap. The card's
//  size is the other contract: it comes from the attachments' metadata,
//  clamped to a shape the pile can hold, and never from bytes — a card
//  that changed size when its preview landed would shove the thread the
//  non-lazy window exists to keep still. The index rules are the
//  viewer's: whatever an opener counted, `current` is one of the items,
//  and the arrows stop at the ends.
//

import CoreGraphics
import Foundation
import Testing
@testable import FamilyConnect

@Suite("Attachment album")
struct AttachmentAlbumTests {

    private func attachment(
        _ id: Int64, kind: String = AttachmentDTO.Kind.photo,
        width: Int? = 1200, height: Int? = 900
    ) -> AttachmentDTO {
        AttachmentDTO(
            id: id,
            kind: kind,
            mime: kind == AttachmentDTO.Kind.video ? "video/mp4" : "image/jpeg",
            size: 1,
            width: width,
            height: height,
            durationMS: nil,
            hasPreview: false,
            name: kind == AttachmentDTO.Kind.file ? "receipt.pdf" : nil,
            latitude: nil,
            longitude: nil,
            accuracyM: nil)
    }

    @Test("the pile keeps photos and videos in sent order; the rows keep the rest")
    func partitionKeepsOrderAndKinds() {
        let sent = [
            attachment(1),
            attachment(2, kind: AttachmentDTO.Kind.file),
            attachment(3, kind: AttachmentDTO.Kind.video),
            attachment(4, kind: AttachmentDTO.Kind.audio),
            attachment(5),
            attachment(6, kind: AttachmentDTO.Kind.location),
        ]
        #expect(AttachmentAlbum.media(of: sent).map(\.id) == [1, 3, 5])
        #expect(AttachmentAlbum.rows(of: sent).map(\.id) == [2, 4, 6])
        // Nothing is lost and nothing is in both halves.
        let split = AttachmentAlbum.media(of: sent) + AttachmentAlbum.rows(of: sent)
        #expect(Set(split.map(\.id)) == Set(sent.map(\.id)))
        #expect(AttachmentAlbum.media(of: []).isEmpty)
        #expect(AttachmentAlbum.rows(of: []).isEmpty)
    }

    @Test("the card is the full width at the first item's shape")
    func cardSizeFollowsTheFirstItem() {
        let size = AttachmentAlbum.cardSize(for: attachment(1, width: 1200, height: 900), maxWidth: 240)
        #expect(size.width == 240)
        #expect(size.height == 180)
        // A different platform width, the same shape.
        let mac = AttachmentAlbum.cardSize(for: attachment(1, width: 1200, height: 900), maxWidth: 300)
        #expect(mac.width == 300)
        #expect(mac.height == 225)
    }

    @Test("the card's shape is clamped both ways")
    func cardSizeClamps() {
        // A panorama stops at 3:2 …
        let wide = AttachmentAlbum.cardSize(for: attachment(1, width: 4000, height: 800), maxWidth: 240)
        #expect(wide.width == 240)
        #expect(wide.height == 160)
        // … and a tall screenshot at 3:4.
        let tall = AttachmentAlbum.cardSize(for: attachment(1, width: 800, height: 4000), maxWidth: 240)
        #expect(tall.width == 240)
        #expect(tall.height == 320)
    }

    /// The card is sized before a byte arrives: unknown dimensions take
    /// the DTO's own fallback shape, and no image is consulted.
    @Test("the card never waits for bytes")
    func cardSizeFromMetadataOnly() {
        let unknown = attachment(1, width: nil, height: nil)
        let size = AttachmentAlbum.cardSize(for: unknown, maxWidth: 240)
        #expect(size.width == 240)
        #expect(size.height == 180)
        #expect(AttachmentAlbum.peek > 0)
    }

    /// The pile's whole point is the cards peeking out behind the top
    /// one, and a shrink about the centre once ate that peek on every
    /// portrait album (the top edge dropped further than the lift raised
    /// it). Pins the geometry the views draw from: for both platform
    /// widths and the card shapes the clamp allows, each card's highest
    /// corner — shrunk about the top, tilted about the centre, then moved
    /// by `offset(for:)` — sits exactly its lift above the top card,
    /// inside the room the container reserves, and the third above the
    /// second.
    @Test("every card behind peeks out by its lift, inside the reserved room")
    func layersPeekByTheirLift() {
        for width in [CGFloat(240), CGFloat(300)] {
            for ratio in [CGFloat(0.75), CGFloat(1), CGFloat(1.5)] {
                let card = CGSize(width: width, height: (width / ratio).rounded())
                var lastRise = CGFloat(0)
                for layer in [AttachmentAlbum.Layer.second, .third] {
                    // Independent of the code under test: a top corner
                    // relative to the centre of rotation, rotated.
                    let radians = abs(layer.tilt) * .pi / 180
                    let x = card.width * layer.scale / 2
                    let y = card.height / 2
                    let cornerAboveCentre = x * CGFloat(sin(radians)) + y * CGFloat(cos(radians))
                    let rise = cornerAboveCentre - card.height / 2 + layer.offset(for: card)
                    #expect(abs(rise - layer.lift) < 0.001, "\(Int(width))pt card at \(ratio)")
                    #expect(rise <= AttachmentAlbum.peek + 0.001)
                    #expect(rise > lastRise)
                    lastRise = rise
                }
            }
        }
    }

    @Test("the index is clamped to the items")
    func indexClamps() {
        let items = [attachment(1), attachment(2), attachment(3)]
        #expect(AttachmentAlbum(items: items, index: -4).index == 0)
        #expect(AttachmentAlbum(items: items, index: 1).index == 1)
        #expect(AttachmentAlbum(items: items, index: 9).index == 2)
        #expect(AttachmentAlbum(items: items, index: 9).current.id == 3)
        #expect(AttachmentAlbum(items: items, index: 0).count == 3)
    }

    @Test("previous and next stop at the ends")
    func previousNextStopAtEnds() {
        let items = [attachment(1), attachment(2), attachment(3)]
        let first = AttachmentAlbum(items: items, index: 0)
        #expect(!first.hasPrevious)
        #expect(first.hasNext)
        #expect(first.previous() == first)
        #expect(first.next().index == 1)
        let last = first.next().next()
        #expect(last.index == 2)
        #expect(last.hasPrevious)
        #expect(!last.hasNext)
        #expect(last.next() == last)
        #expect(last.previous().index == 1)
        // A lone item has nowhere to go.
        let lone = AttachmentAlbum(items: [attachment(7)], index: 0)
        #expect(!lone.hasPrevious && !lone.hasNext)
        #expect(lone.next() == lone && lone.previous() == lone)
    }

    /// A Mac window is keyed by the album; the key has to survive the
    /// scene's encode/decode round trip and tell two positions apart.
    @Test("an album round-trips through Codable and is keyed by position")
    func codableAndHashable() throws {
        let items = [attachment(1), attachment(2, kind: AttachmentDTO.Kind.video)]
        let album = AttachmentAlbum(items: items, index: 1)
        let data = try JSONEncoder().encode(album)
        let back = try JSONDecoder().decode(AttachmentAlbum.self, from: data)
        #expect(back == album)
        #expect(AttachmentAlbum(items: items, index: 0) != album)
        #expect(album.id == album)
    }
}
