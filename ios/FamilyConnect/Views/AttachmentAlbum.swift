//
//  AttachmentAlbum.swift
//  FamilyConnect
//
//  A message's photos and videos, as the thing a bubble draws and a
//  viewer pages through. Shared by the phone and the Mac.
//
//  A message carries its attachments as ONE list in sent order, but a
//  bubble does not draw them as one list: photos and videos are looked
//  at, and files, audio and a location are read. So the media form a
//  pile the reader opens — one card with the ones behind it peeking out —
//  and everything else stays the rows it has always been, stacked under
//  the pile. The split lives here once, so the phone, the Mac and (in
//  intent) Android cut the same message the same way.
//
//  The album is also the viewer's whole input: which items, and which
//  one is up. Hashable + Codable because a Mac window is keyed by its
//  value (WindowGroup(for:) needs both), and the index is part of that
//  key, so the window that opens on the third photo is not the window
//  that opened on the first.
//
//  The card's size comes from the attachments' METADATA — the width and
//  height the upload recorded — never from loaded bytes. Both threads are
//  non-lazy windows whose whole design is real row heights (see the
//  ConversationView header), and a card that grew when its preview landed
//  would shove the thread exactly the way an unsized single photo used to.
//
//  The pile's geometry — how far up, how much smaller, how tilted the
//  cards behind the top one sit — is the one table `Layer` below, shared
//  by the phone and the Mac so a three-photo message piles the same on
//  both. The lift is what the eye sees: the highest point of a card
//  behind, above the top card's edge. Two things would otherwise eat it
//  and made the pile invisible on every portrait album: a shrink about
//  the card's centre pulls its top edge DOWN by (1 − scale)·height/2
//  (more than the lift on a tall card), and a tilt raises one corner
//  by (width·scale/2)·sin(tilt) — above the room the container
//  reserves. So the views shrink about the TOP edge, and `offset(for:)`
//  spends the tilt's rise out of the lift, which is why the reservation
//  is exactly the deepest lift on every card shape.
//
//  Android counterpart: ui/components/AttachmentAlbum.kt.
//

import CoreGraphics
import Foundation

nonisolated struct AttachmentAlbum: Hashable, Codable, Identifiable, Sendable {
    /// Photos and videos only, in sent order — what `media(of:)` returns.
    /// Never empty: an album with nothing to look at is never built.
    let items: [AttachmentDTO]
    /// Which item is up. Clamped at construction, so `current` is always
    /// one of `items` no matter what the opener counted.
    let index: Int

    init(items: [AttachmentDTO], index: Int) {
        self.items = items
        self.index = items.isEmpty ? 0 : min(max(index, 0), items.count - 1)
    }

    var id: AttachmentAlbum { self }

    var count: Int { items.count }
    var current: AttachmentDTO { items[index] }
    var hasPrevious: Bool { index > 0 }
    var hasNext: Bool { index + 1 < items.count }

    /// The same album one item back; at the first, itself — a viewer's
    /// arrow keys stop at the ends rather than wrap.
    func previous() -> AttachmentAlbum {
        AttachmentAlbum(items: items, index: index - 1)
    }

    func next() -> AttachmentAlbum {
        AttachmentAlbum(items: items, index: index + 1)
    }

    /// Where a card behind the top one sits. `lift` is the height of its
    /// highest point above the top card's edge — the peek a reader sees;
    /// `scale` how much smaller it is drawn, about its TOP edge (see the
    /// header); `tilt` its rotation in degrees, about its centre, the
    /// second and third leaning opposite ways so the pile reads as three
    /// things rather than one card with a thick edge.
    struct Layer: Sendable {
        let lift: CGFloat
        let scale: CGFloat
        let tilt: Double

        static let second = Layer(lift: 6, scale: 0.94, tilt: -2.5)
        static let third = Layer(lift: 12, scale: 0.88, tilt: 2.5)

        /// How far up the view moves this card so that its highest
        /// corner, once shrunk about the top and tilted about the
        /// centre, sits exactly `lift` above the top card. The tilt
        /// raises one top corner by (width·scale/2)·sin(tilt) and the
        /// centre of rotation, half the UNSHRUNK height down, pulls the
        /// top edge back by (height/2)·(1 − cos(tilt)) — a fraction of a
        /// point, kept so the result is exact rather than nearly so.
        /// Can go a hair negative on a wide card: the top edge's middle
        /// then sits on the top card's edge and only the corner shows,
        /// which is what a tilted card in a pile looks like.
        func offset(for card: CGSize) -> CGFloat {
            let radians = abs(tilt) * .pi / 180
            let cornerRise = card.width * scale / 2 * CGFloat(sin(radians))
            let centreDrop = card.height / 2 * CGFloat(1 - cos(radians))
            return lift - cornerRise + centreDrop
        }
    }

    /// Room the pile reserves ABOVE the card for the cards behind it to
    /// peek into, on every platform: the container is the card plus this,
    /// with the card sitting at its bottom, so nothing is clipped. It is
    /// the deepest card's lift because `Layer.offset(for:)` keeps every
    /// card's highest corner at its lift, whatever the card's shape.
    static var peek: CGFloat { Layer.third.lift }

    /// Everything a reader looks at, in sent order.
    static func media(of attachments: [AttachmentDTO]) -> [AttachmentDTO] {
        attachments.filter { !$0.isFile && !$0.isAudio && !$0.isLocation }
    }

    /// Everything a reader reads — files, audio, a location — in sent
    /// order. The complement of `media(of:)`.
    static func rows(of attachments: [AttachmentDTO]) -> [AttachmentDTO] {
        attachments.filter { $0.isFile || $0.isAudio || $0.isLocation }
    }

    /// The card a pile draws its top item into: the platform's full media
    /// width, at the item's own shape held between 3:4 and 3:2. A
    /// panorama would otherwise be a ribbon and a tall screenshot a
    /// tower, and with two cards peeking out behind it either reads as a
    /// mistake. Metadata only — see the header. Whole points, so the row
    /// height the thread measures is the one the card draws at.
    static func cardSize(for first: AttachmentDTO, maxWidth: CGFloat) -> CGSize {
        let ratio = min(max(CGFloat(first.aspectRatio), 0.75), 1.5)
        return CGSize(width: maxWidth, height: (maxWidth / ratio).rounded())
    }
}
