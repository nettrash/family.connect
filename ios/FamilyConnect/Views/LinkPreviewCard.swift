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

// PLATFORM-FREE, and it did not start that way: this was iOS-only, so a
// Mac showed a bare URL where a phone showed a card for the very same
// message. The only iOS-specific things in it were two colours, which
// PlatformStyle already names for both platforms.
//
// The Mac forwards no gesture callbacks (its balloon uses a context menu
// rather than a floating tapback), so those default to no-ops there.

import SwiftUI

struct LinkPreviewCard: View {
    let preview: LinkPreview
    /// Already-decoded card image, if it arrived.
    let image: Image?
    let onOpen: (URL) -> Void
    /// The bubble's own gestures, forwarded — same reason AttachmentView
    /// takes them: a bare `.onTapGesture` on a CHILD masks the parent's
    /// count-2 gesture outright, so long-pressing to react or
    /// double-tapping to heart worked only on the padding around the card.
    var onLongPress: () -> Void = {}
    var onDoubleTap: () -> Void = {}

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
        // A material, not the window colour: `appBackground` is pure black
        // in dark mode, a hard rectangle cut out of a grey or blue balloon.
        // The material takes the balloon's tone behind it and keeps its
        // own text legible through vibrancy on both balloon colours.
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(Color.appSeparator, lineWidth: 1)
        }
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .contentShape(Rectangle())
        // Count 2 BEFORE count 1, and both as onTapGesture: that is what
        // makes them exclusive. As a single bare tap this fired onOpen
        // TWICE on a double tap — the link opened, then opened again — and
        // the balloon's heart never got a look in.
        .onTapGesture(count: 2) { onDoubleTap() }
        .onTapGesture(count: 1) { onOpen(preview.url) }
        .simultaneousGesture(LongPressGesture().onEnded { _ in onLongPress() })
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel("\(preview.title), \(preview.siteName)")
        .accessibilityHint("Opens the link")
        // The trait alone is a LIE: a bare gesture publishes no
        // accessibility action, so VoiceOver announced "button" and
        // double-tapping did nothing (measured — ZZAXProbeTests). Same
        // lesson as the reply quote and the reaction chips.
        .accessibilityAction { onOpen(preview.url) }
    }
}
