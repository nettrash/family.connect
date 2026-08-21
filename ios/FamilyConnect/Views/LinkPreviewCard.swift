//
//  LinkPreviewCard.swift
//  FamilyConnect
//
//  The preview under a message's first web link: image (when the page
//  offers one), title, description, host.
//
//  It renders only once the fetch has landed — no skeleton, no reserved
//  space. A placeholder would make every linked bubble change height
//  twice, and this thread's scroll position is anchored on real bubble
//  heights (see ConversationView); one growth, at load, is enough.
//
//  Cut out of the balloon in the app's own background colour for the
//  same reason the reaction chips are: on my balloon the content colour
//  is white over a saturated accent, and washes of it lose contrast.
//
//  Android counterpart: the LinkPreviewCard composable in
//  android/…/ui/chat/ChatScreen.kt.
//

import SwiftUI

struct LinkPreviewCard: View {
    let preview: LinkPreview
    /// Already-decoded card image, if it arrived.
    let image: Image?
    let onOpen: (URL) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let image {
                image
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .frame(height: 120)
                    .frame(maxWidth: .infinity)
                    .clipped()
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(preview.siteName)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Text(preview.title)
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
                if let description = preview.description {
                    Text(description)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
        }
        .background(Color(.systemBackground), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(Color(.separator), lineWidth: 1)
        }
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .contentShape(Rectangle())
        .onTapGesture { onOpen(preview.url) }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel("\(preview.title), \(preview.siteName)")
        .accessibilityHint("Opens the link")
    }
}
