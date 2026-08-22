//
//  ReactionPickerOverlay.swift
//  FamilyConnect
//
//  The pieces of the Tapback-style floating reaction picker. Bubbles
//  publish their frames through `BubbleAnchorKey` (localID → bounds
//  anchor); ConversationView reads the pressed bubble's anchor in an
//  `overlayPreferenceValue` and floats one of the two menus here over a
//  dimmed scrim, clamped to the screen and growing out of the bubble.
//
//  Two menus because the long-press gates are disjoint (a failed message
//  never has a server id, and reacting needs one):
//
//    ReactionCapsule    the quick-emoji row + "+" for the full picker —
//                       messages the server knows
//    FailedMessageMenu  Retry / Delete — failed local messages
//
//  Both expose their exact size as statics (fixed per-item frames, no
//  measurement pass) so the overlay can place and clamp them before the
//  first layout — a measured size would land a frame late and jump.
//

import SwiftUI

/// localID → bubble bounds, merged across all visible bubbles. Only the
/// pressed bubble's entry is ever read, so lazily-recycled rows dropping
/// out of the dictionary is fine.
nonisolated struct BubbleAnchorKey: PreferenceKey {
    static var defaultValue: [String: Anchor<CGRect>] { [:] }

    static func reduce(value: inout [String: Anchor<CGRect>], nextValue: () -> [String: Anchor<CGRect>]) {
        value.merge(nextValue()) { _, new in new }
    }
}

/// The floating quick-reaction capsule: the quick set (plus the user's
/// current off-set reaction as an extra item, so it can be tapped off),
/// then "+" for the full picker. The parent owns dismissal.
struct ReactionCapsule: View {
    /// The emoji to offer, in order — quick set, plus my non-quick
    /// current reaction appended by the parent when there is one.
    let emojis: [String]
    /// My current reaction, highlighted (tap = remove).
    let selected: String?
    let onPick: (String) -> Void
    let onMore: () -> Void

    private static let itemSide: CGFloat = 40
    private static let itemSpacing: CGFloat = 2
    private static let capsulePadding: CGFloat = 6

    /// Exact rendered size for `emojiCount` emoji plus the "+" item —
    /// every item has a fixed frame, so this is arithmetic, not layout.
    static func size(emojiCount: Int) -> CGSize {
        let items = CGFloat(emojiCount + 1)
        return CGSize(
            width: items * itemSide + (items - 1) * itemSpacing + capsulePadding * 2,
            height: itemSide + capsulePadding * 2)
    }

    var body: some View {
        HStack(spacing: Self.itemSpacing) {
            ForEach(emojis, id: \.self) { emoji in
                Button {
                    onPick(emoji)
                } label: {
                    Text(emoji)
                        .font(.system(size: 26))
                        .frame(width: Self.itemSide, height: Self.itemSide)
                        .background {
                            if selected == emoji {
                                Circle().fill(.tint.opacity(0.2))
                            }
                        }
                }
                .buttonStyle(.plain)
                .accessibilityLabel(emoji)
            }
            Button {
                onMore()
            } label: {
                Image(systemName: "plus.circle.fill")
                    .font(.system(size: 26))
                    .foregroundStyle(.secondary)
                    .frame(width: Self.itemSide, height: Self.itemSide)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("More reactions")
        }
        .padding(Self.capsulePadding)
        .background(.regularMaterial, in: Capsule())
        .shadow(color: .black.opacity(0.15), radius: 12, y: 4)
    }
}

/// UIActivityViewController in SwiftUI clothing. ShareLink would do for
/// a link, but this shares plain message text from a `.sheet(item:)`
/// rather than from a button the user taps directly.
struct ShareSheet: UIViewControllerRepresentable {
    let text: String

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: [text], applicationActivities: nil)
    }

    func updateUIViewController(_ controller: UIActivityViewController, context: Context) {}
}

/// The actions under the bubble, beside the reaction capsule above it.
/// All three are local — the message text is already in hand, and Reply
/// only primes the composer — so none waits on the network. The parent
/// owns dismissal.
struct MessageContextMenu: View {
    let onReply: () -> Void
    let onEdit: () -> Void
    let onCopy: () -> Void
    let onShare: () -> Void
    /// Reply needs a server id to quote, so it is hidden on a message that
    /// has not been acked yet rather than shown and doing nothing. The
    /// menu's height follows, since the overlay places it by size.
    var canReply: Bool = true
    /// Only the author may edit, and only once the message has an id.
    var canEdit: Bool = false

    private static let rowHeight: CGFloat = 44
    private static let menuWidth: CGFloat = 220

    /// Exact rendered size — fixed-height rows and their hairlines. The
    /// overlay needs the size up front to place the menu.
    static func size(canReply: Bool, canEdit: Bool = false) -> CGSize {
        let rows = 2.0 + (canReply ? 1.0 : 0.0) + (canEdit ? 1.0 : 0.0)
        return CGSize(width: menuWidth, height: rowHeight * rows + (rows - 1))
    }

    var body: some View {
        VStack(spacing: 0) {
            if canReply {
                row("Reply", systemImage: "arrowshape.turn.up.left", action: onReply)
                Divider()
            }
            if canEdit {
                row("Edit", systemImage: "pencil", action: onEdit)
                Divider()
            }
            row("Copy", systemImage: "doc.on.doc", action: onCopy)
            Divider()
            row("Share", systemImage: "square.and.arrow.up", action: onShare)
        }
        .frame(width: Self.menuWidth)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .shadow(color: .black.opacity(0.15), radius: 12, y: 4)
    }

    private func row(_ title: String, systemImage: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Label(title, systemImage: systemImage)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 16)
                .frame(height: Self.rowHeight)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// The floating menu for a failed bubble: Retry / Delete (the actions a
/// message without a server id can offer). The parent owns dismissal.
struct FailedMessageMenu: View {
    let onRetry: () -> Void
    let onDelete: () -> Void

    private static let rowHeight: CGFloat = 44
    private static let menuWidth: CGFloat = 220

    /// Exact rendered size — two fixed-height rows and a hairline.
    static var size: CGSize {
        CGSize(width: menuWidth, height: rowHeight * 2 + 1)
    }

    var body: some View {
        VStack(spacing: 0) {
            Button {
                onRetry()
            } label: {
                Label("Try Again", systemImage: "arrow.clockwise")
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16)
                    .frame(height: Self.rowHeight)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            Divider()
            Button(role: .destructive) {
                onDelete()
            } label: {
                Label("Delete", systemImage: "trash")
                    .foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16)
                    .frame(height: Self.rowHeight)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .frame(width: Self.menuWidth)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .shadow(color: .black.opacity(0.15), radius: 12, y: 4)
    }
}
