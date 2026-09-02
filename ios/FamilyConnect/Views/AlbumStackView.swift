//
//  AlbumStackView.swift
//  FamilyConnect
//
//  Several photos in one bubble, drawn as a pile: the first one as a card,
//  with the second and third peeking out behind it, a count in the corner.
//
//  A pile rather than a grid because a grid of squares has to CROP every
//  photo to the same shape and then leaves an odd one alone at an edge —
//  the "not so symmetric" look a message with three photos always had.
//  One card at the first photo's own shape is what a lone photo has
//  always looked like, and the peeking corners behind it are real
//  previews, not blank cards, so the pile already shows what is in it.
//
//  The card is sized from the attachments' METADATA (AttachmentAlbum), so
//  a bubble reserves its exact height before a byte arrives — the same
//  contract AttachmentView keeps, for the same reason: the thread's
//  non-lazy window measures real heights, and a card that grew when its
//  preview landed would shove it.
//
//  Android counterpart: AlbumStack in ui/components/Attachments.kt.
//

// iOS only — the Mac has its own views (MacViews/).
#if os(iOS)

import SwiftUI

struct AlbumStackView: View {
    /// The message's media, in sent order; at least two.
    let album: AttachmentAlbum
    /// Opens the viewer at the first item.
    let onOpen: () -> Void
    /// The bubble's own gestures, forwarded — the same trio the single
    /// thumbnail carries, and for the same reason: a caption-less pile IS
    /// the balloon.
    var onLongPress: () -> Void = {}
    var onDoubleTap: () -> Void = {}
    var isMine: Bool = false

    @Environment(AttachmentStore.self) private var store

    private var card: CGSize {
        AttachmentAlbum.cardSize(for: album.items[0], maxWidth: AttachmentView.maxWidth)
    }

    var body: some View {
        // Reading `store.generation` is what makes this redraw when a
        // fetch lands — the LOAD-BEARING rule AttachmentView explains.
        let _ = store.generation
        ZStack(alignment: .bottom) {
            if album.items.count >= 3 {
                layer(album.items[2], .third)
            }
            layer(album.items[1], .second)
            topCard
        }
        // The peek room is reserved and the card sits at the bottom of
        // it, so the tilted corners above are drawn, never clipped —
        // AttachmentAlbum.Layer keeps every corner inside that room.
        .frame(width: card.width, height: card.height + AttachmentAlbum.peek, alignment: .bottom)
        .contentShape(Rectangle())
        // Count 2 BEFORE count 1, and both as onTapGesture: that is what
        // makes them exclusive (AttachmentView has the history).
        .onTapGesture(count: 2) { onDoubleTap() }
        .onTapGesture(count: 1) { onOpen() }
        .simultaneousGesture(LongPressGesture().onEnded { _ in onLongPress() })
        // One element: the pile opens as one thing, and a reader who
        // wants a particular photo pages to it in the viewer.
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityText)
        .accessibilityAddTraits(.isButton)
        // A bare gesture publishes no accessibility action — measured, see
        // ZZAXProbeTests.
        .accessibilityAction { onOpen() }
    }

    /// "Album, 1 of N" — the viewer's own position wording, so what
    /// VoiceOver hears on the bubble is what it hears once inside.
    private var accessibilityText: String {
        let position = String(
            localized: "\(1) of \(album.count)",
            comment: "Which photo of an album is being looked at: the first number is its position, the second the album's size.")
        return "\(String(localized: "Album", comment: "Several photos or videos sent as one message.")), \(position)"
    }

    /// The first item, exactly the tile a lone photo draws, plus the
    /// count badge.
    private var topCard: some View {
        ZStack {
            AttachmentView.placeholder(isMine: isMine)
            if let image = AttachmentView.image(for: album.items[0], in: store) {
                image
                    .resizable()
                    .aspectRatio(contentMode: .fill)
            } else if isAwaitingBytes(album.items[0]) {
                ProgressView()
            }
            if album.items[0].isVideo {
                VideoBadgeOverlay(attachment: album.items[0])
            }
        }
        .frame(width: card.width, height: card.height)
        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(hairline))
        .overlay(alignment: .topTrailing) { countBadge }
    }

    /// A card behind the top one: the item's preview at the card's own
    /// frame, then shrunk, tilted and lifted. Filled and clipped like the
    /// top card, so a corner that peeks out is a corner of a photo.
    /// Shrunk about the TOP edge, not the centre: a centre shrink pulls
    /// the top edge down by (1 − scale)·height/2, which on a portrait
    /// card is more than the lift — the pile then drew as one card and a
    /// badge (AttachmentAlbum.Layer has the geometry).
    private func layer(_ item: AttachmentDTO, _ place: AttachmentAlbum.Layer) -> some View {
        ZStack {
            AttachmentView.placeholder(isMine: isMine)
            if let image = AttachmentView.image(for: item, in: store) {
                image
                    .resizable()
                    .aspectRatio(contentMode: .fill)
            }
        }
        .frame(width: card.width, height: card.height)
        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(hairline))
        .scaleEffect(place.scale, anchor: .top)
        .rotationEffect(.degrees(place.tilt))
        .offset(y: -place.offset(for: card))
    }

    /// The duration badge's style, carrying the count instead: black
    /// capsule, white glyphs, readable over any photo.
    private var countBadge: some View {
        HStack(spacing: 3) {
            Image(systemName: "photo.stack")
            Text(album.count, format: .number)
        }
        .font(.caption2.weight(.medium))
        .foregroundStyle(.white)
        .padding(.horizontal, 6)
        .padding(.vertical, 2)
        .background(.black.opacity(0.55), in: Capsule())
        .padding(6)
    }

    /// The same hairline the single thumbnail draws, so a pale photo does
    /// not melt into a pale balloon.
    private var hairline: Color {
        isMine ? Color.white.opacity(0.18) : Color.primary.opacity(0.08)
    }

    /// AttachmentView's rule: a video's poster is always asked for, but
    /// never promised — the badge over the placeholder, not a spinner.
    private func isAwaitingBytes(_ item: AttachmentDTO) -> Bool {
        AttachmentView.image(for: item, in: store) == nil && (item.hasPreview || !item.isVideo)
    }
}

#endif
