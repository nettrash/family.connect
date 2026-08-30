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

// iOS only — the Mac has its own views (MacViews/).
#if os(iOS)

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
    let items: [Any]

    init(text: String) {
        items = [text]
    }

    /// A file (and optionally its caption). A URL is what lets the sheet
    /// offer "Save to Files", AirDrop and every document app — sharing an
    /// attachment as a UIImage would strip it back to pixels.
    init(items: [Any]) {
        self.items = items
    }

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: items, applicationActivities: nil)
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
    /// A photo sent without a caption has nothing to copy.
    var canCopy: Bool = true
    /// Somebody else's acked message, from a real member. Never the
    /// assistant: its reserved account is deliberately absent from the
    /// roster, so reporting or blocking it would name a non-member and the
    /// server would refuse — a VISIBLE refusal in a feature whose whole
    /// design is that every refusal is aimed at the blocker and looks
    /// innocent.
    var canReport: Bool = false
    /// `nil` when blocking does not apply to this message at all (own, or
    /// the assistant's); otherwise which way the row reads.
    var blockState: BlockState?
    var onReport: () -> Void = {}
    var onBlock: () -> Void = {}
    var onUnblock: () -> Void = {}

    enum BlockState { case blocked, notBlocked }

    /// The rows this menu draws, in order.
    ///
    /// ONE list, read by both the body and `size`. They used to be
    /// separate — a sum of booleans here and a stack of `if`s there — and
    /// that only added up because Share was UNCONDITIONAL and LAST: the
    /// body emits a `Divider()` after every row except the last, so the
    /// `(n - 1)` hairline term was exact only by that accident. Appending
    /// anything after Share broke it two ways at once, a missing divider
    /// (1pt, invisible) and a stale row count (45pt, not).
    private enum Item: Hashable { case reply, edit, copy, share, report, block, unblock }

    private static func items(
        canReply: Bool, canEdit: Bool, canCopy: Bool, canReport: Bool, blockState: BlockState?
    ) -> [Item] {
        var items: [Item] = []
        if canReply { items.append(.reply) }
        if canEdit { items.append(.edit) }
        if canCopy { items.append(.copy) }
        items.append(.share)
        if canReport { items.append(.report) }
        switch blockState {
        case .blocked: items.append(.unblock)
        case .notBlocked: items.append(.block)
        case nil: break
        }
        return items
    }

    private static let rowHeight: CGFloat = 44
    private static let menuWidth: CGFloat = 220

    /// Exact rendered size — fixed-height rows and their hairlines. The
    /// overlay needs the size up front to place the menu, BEFORE layout, so
    /// a row the body draws and this does not count mis-places the whole
    /// panel with no error anywhere.
    ///
    /// The maximum is FIVE rows: `canEdit` requires the message to be the
    /// reader's own and `canReport`/`blockState` require it not to be, so
    /// Edit can never coexist with Report or Block.
    static func size(
        canReply: Bool,
        canEdit: Bool = false,
        canCopy: Bool = true,
        canReport: Bool = false,
        blockState: BlockState? = nil
    ) -> CGSize {
        let n = CGFloat(
            items(
                canReply: canReply, canEdit: canEdit, canCopy: canCopy,
                canReport: canReport, blockState: blockState
            ).count)
        return CGSize(width: menuWidth, height: rowHeight * n + (n - 1))
    }

    var body: some View {
        let items = Self.items(
            canReply: canReply, canEdit: canEdit, canCopy: canCopy,
            canReport: canReport, blockState: blockState)
        return VStack(spacing: 0) {
            ForEach(Array(items.enumerated()), id: \.element) { index, item in
                self.row(for: item)
                // BETWEEN adjacent rows only — never after the last, which
                // is what `size`'s `(n - 1)` counts.
                if index < items.count - 1 { Divider() }
            }
        }
        .frame(width: Self.menuWidth)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .shadow(color: .black.opacity(0.15), radius: 12, y: 4)
    }

    /// Titles are BARE — "Report…", "Block", never "Block Anna". The
    /// VStack is a hard 220pt with 16pt padding and a symbol, leaving about
    /// 188pt, and `rowHeight` is a hard 44 so a truncated label cannot
    /// recover onto a second line. A truncated destructive row is the one
    /// place somebody must be certain what they are pressing; the member's
    /// name goes in the confirmation, where there is room.
    @ViewBuilder
    private func row(for item: Item) -> some View {
        switch item {
        case .reply:
            row("Reply", systemImage: "arrowshape.turn.up.left", action: onReply)
        case .edit:
            row("Edit", systemImage: "pencil", action: onEdit)
        case .copy:
            row("Copy", systemImage: "doc.on.doc", action: onCopy)
        case .share:
            // Share covers saving too: the sheet's own "Save Image" /
            // "Save Video" put it in the library, and "Save to Files" puts
            // it anywhere else. A second row would be the same action.
            row("Share", systemImage: "square.and.arrow.up", action: onShare)
        case .report:
            row("Report…", systemImage: "exclamationmark.bubble", action: onReport)
        case .block:
            row("Block", systemImage: "hand.raised", action: onBlock, isDestructive: true)
        case .unblock:
            row("Unblock", systemImage: "hand.raised.slash", action: onUnblock)
        }
    }

    /// `LocalizedStringKey`, NOT `String`. With a `String` variable
    /// `Label(_:systemImage:)` picks the VERBATIM `StringProtocol`
    /// overload, so every row here drew English in all nine languages —
    /// invisibly, because the keys exist in the catalogue anyway, having
    /// been extracted from the Mac's native `Button("Reply")` calls. The
    /// same trap `LocationAttachmentView` records for `Text`.
    private func row(
        _ title: LocalizedStringKey, systemImage: String, action: @escaping () -> Void,
        isDestructive: Bool = false
    ) -> some View {
        Button(action: action) {
            Label(title, systemImage: systemImage)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 16)
                .frame(height: Self.rowHeight)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(isDestructive ? AnyShapeStyle(.red) : AnyShapeStyle(.primary))
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

#endif
