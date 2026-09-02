//
//  StagedAttachment.swift
//  FamilyConnect
//
//  Something prepared and waiting for Send, and the chips that show it.
//
//  Staging separates "what to send" from "when": picking used to send
//  immediately, which meant a caption had to be typed BEFORE choosing the
//  photo — and once picked there was no way out. A message now carries up
//  to TEN attachments (docs/protocol.md, "Photos, videos, audio, files
//  and locations"), so a second pick APPENDS behind the first rather than
//  superseding it; at the cap the composer says so with a brief notice
//  instead of silently dropping the pick.
//
//  Shared, with no `#if os` guard, for the reason LocationAttachmentView is
//  shared: both platforms draw exactly this, from exactly the same
//  prepared item. It moved out of ConversationView when the Mac composer
//  gained a staging step of its own — paste-to-attach must never send by
//  itself, and the Mac had nowhere to put something that was not sent yet.
//

import SwiftUI

/// Media that is prepared and waiting for the user to press Send.
struct StagedAttachment: Identifiable {
    let id = UUID()
    let prepared: MediaPrep.Prepared

    /// The protocol's ceiling on one message's attachments
    /// (docs/protocol.md, "Limits": `limits.max_attachments_per_message`).
    /// Both composers refuse the eleventh pick against this, with a
    /// notice — the same number the server would refuse it with.
    nonisolated static let maxPerMessage = 10

    /// May one more item be staged beside `count` already staged?
    ///
    /// A one-line rule, extracted so both composers (and the share-import
    /// path, which arrives with a whole batch) ask the same question and
    /// the tests can pin the answer without a composer on screen.
    static func canAdd(to count: Int) -> Bool {
        count < maxPerMessage
    }

    /// The composer's thumbnail: the same JPEG the bubble will draw.
    /// Files and audio have none — a document is a row, and a sound has
    /// nothing to look at.
    var thumbnail: Image? {
        guard let data = prepared.previewJPEG,
              let image = PlatformImage.decode(data, maxPixels: 240)
        else { return nil }
        return PlatformImage.view(image)
    }

    /// What the chip calls it.
    ///
    /// A word per kind rather than "Photo" for everything that is not a
    /// video, which is what this used to do — a voice note has no name (its
    /// identity is its length), and before the recorder stopped handing one
    /// over it read out the scratch file it was recorded into.
    var label: String {
        if let name = prepared.name, !name.isEmpty { return name }
        switch prepared.kind {
        case AttachmentDTO.Kind.video: return String(localized: "Video")
        case AttachmentDTO.Kind.audio: return String(localized: "Audio")
        case AttachmentDTO.Kind.file: return String(localized: "File")
        default: return String(localized: "Photo")
        }
    }

    /// The glyph drawn where there is no thumbnail.
    var placeholderSymbol: String {
        switch prepared.kind {
        case AttachmentDTO.Kind.audio: "waveform"
        default: "doc"
        }
    }
}

/// A staged item's thumbnail-or-glyph square, shared by the full-width
/// chip (one item staged) and the compact tile (several).
private struct StagedThumbnail: View {
    let item: StagedAttachment
    let side: CGFloat

    var body: some View {
        ZStack {
            if let thumbnail = item.thumbnail {
                thumbnail
                    .resizable()
                    .aspectRatio(contentMode: .fill)
            } else {
                Color.appSecondaryFill
                Image(systemName: item.placeholderSymbol)
                    .font(.title3)
                    .foregroundStyle(.secondary)
            }
            if item.prepared.kind == AttachmentDTO.Kind.video {
                Image(systemName: "play.circle.fill")
                    .font(.title3)
                    .foregroundStyle(.white, .black.opacity(0.35))
            }
        }
        // Pinned on BOTH axes, and that is load-bearing: a
        // height-flexible tile beside a Text turns the composer's
        // stack into height distribution and truncates the label.
        .frame(width: side, height: side)
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

/// The one staged attachment sitting above the field, with the way out —
/// the shape this row has always had, kept for the single-item case.
struct StagedAttachmentChip: View {
    let item: StagedAttachment
    let onRemove: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            StagedThumbnail(item: item, side: 44)

            VStack(alignment: .leading, spacing: 1) {
                Text(item.label)
                    .font(.caption.weight(.medium))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Text("Add a message, or send it on its own.")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
            Button {
                onRemove()
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .font(.title3)
                    .foregroundStyle(.secondary)
                    // Tap slack without a layout change: the chip's height
                    // is part of the input bar's, which the thread re-pins
                    // against.
                    .contentShape(Rectangle().inset(by: -12))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Remove attachment")
        }
    }
}

/// One staged item among several: a compact tile for the horizontal row,
/// its own remove X riding the corner. The full-width chip's subtitle
/// would repeat per item, so the tile keeps only what identifies the
/// item — the thumbnail and, through accessibility, its label.
struct StagedAttachmentTile: View {
    let item: StagedAttachment
    let onRemove: () -> Void

    var body: some View {
        StagedThumbnail(item: item, side: 56)
            .overlay(alignment: .topTrailing) {
                Button {
                    onRemove()
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.body)
                        // Two-tone so the X reads on any thumbnail.
                        .foregroundStyle(.white, .black.opacity(0.55))
                        // The glyph stays in the corner; only the tappable
                        // area grows toward the 44pt guideline. The tile
                        // itself has no tap gesture, so the overlap is safe.
                        .frame(width: 28, height: 28, alignment: .topTrailing)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .padding(2)
                .accessibilityLabel("Remove attachment")
            }
            .accessibilityElement(children: .combine)
            .accessibilityLabel(item.label)
    }
}

/// The staged set above the field: the familiar full-width chip while one
/// item is staged, a horizontally scrolling row of tiles for several.
/// Shared by both composers, so the two cannot drift on the cap or the
/// row's shape.
struct StagedAttachmentRow: View {
    let items: [StagedAttachment]
    let onRemove: (StagedAttachment) -> Void

    var body: some View {
        if items.count == 1, let item = items.first {
            StagedAttachmentChip(item: item) { onRemove(item) }
        } else {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(items) { item in
                        StagedAttachmentTile(item: item) { onRemove(item) }
                    }
                }
                // Room for the X riding above the tile's corner.
                .padding(.top, 2)
            }
        }
    }
}
