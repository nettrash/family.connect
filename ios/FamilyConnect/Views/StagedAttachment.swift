//
//  StagedAttachment.swift
//  FamilyConnect
//
//  Something prepared and waiting for Send, and the chip that shows it.
//
//  Staging separates "what to send" from "when": picking used to send
//  immediately, which meant a caption had to be typed BEFORE choosing the
//  photo — and once picked there was no way out. One attachment per
//  message, so a second pick supersedes the first rather than queueing
//  behind it (docs/protocol.md, "Photos, videos and files").
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

/// The staged attachment sitting above the field, with the way out.
///
/// No outer padding: each composer insets it to match its own bar.
struct StagedAttachmentChip: View {
    let item: StagedAttachment
    let onRemove: () -> Void

    var body: some View {
        HStack(spacing: 10) {
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
            // height-flexible tile beside this Text turns the composer's
            // stack into height distribution and truncates the label.
            .frame(width: 44, height: 44)
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))

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
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Remove attachment")
        }
    }
}
