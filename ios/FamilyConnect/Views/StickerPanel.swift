//
//  StickerPanel.swift
//  FamilyConnect
//
//  The family's stickers as a grid, behind the composer's sticker button.
//  ONE TAP SENDS: no caption, no confirmation, the sticker is the message
//  (docs/protocol.md, "In the panel, one tap sends").
//
//  Shared by both platforms — the phone raises it as a sheet, the Mac as a
//  popover off the button — because nothing in it is platform-specific: it
//  draws the cached pack, and what a tap DOES is the caller's.
//
//  The order is the order the stickers were added (item id ascending, which
//  is the order `GET /families/mine/pack` returns), under a row of the ones
//  THIS DEVICE sent most recently. That row is the device's own business
//  and never on the wire.
//
//  NOT filtered by blocks. A sticker a blocked member added is the family's
//  picture, not their words — the message they SEND is hidden like any
//  other of theirs, and that is decided in the chat, not here.
//

import SwiftData
import SwiftUI

struct StickerPanel: View {
    /// A sticker was chosen. The caller sends it and takes the panel down.
    let onPick: (PackItemSnapshot) -> Void

    // Added order. `itemID` is the server's id, handed out in that order.
    @Query(sort: \PackItemEntity.itemID) private var stored: [PackItemEntity]

    private let columns = [GridItem(.adaptive(minimum: 68), spacing: 8)]

    private var items: [PackItemSnapshot] { stored.map(\.snapshot) }

    var body: some View {
        let items = items
        let recent = StickerRecents.recent(of: items, recents: AppSettings.packRecents)
        Group {
            if items.isEmpty {
                // An empty pack is not a broken panel, and the way out of
                // it is on another screen — so say which.
                ContentUnavailableView(
                    "No stickers yet",
                    systemImage: "face.smiling",
                    description: Text("Add some on the Family screen. Everyone in the family can send them."))
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 8) {
                        if !recent.isEmpty {
                            header("Recently used")
                            grid(recent)
                            header("All stickers")
                        }
                        grid(items)
                    }
                    .padding(12)
                }
            }
        }
    }

    private func header(_ title: LocalizedStringKey) -> some View {
        Text(title)
            .font(.footnote.weight(.semibold))
            .foregroundStyle(.secondary)
            .accessibilityAddTraits(.isHeader)
    }

    private func grid(_ items: [PackItemSnapshot]) -> some View {
        LazyVGrid(columns: columns, spacing: 8) {
            ForEach(items) { item in
                Button {
                    onPick(item)
                } label: {
                    // Frame zero: a panel of two hundred moving pictures is
                    // noise, and the sticker moves once it is in the chat.
                    StickerImage(attachmentID: item.attachmentID, animates: false)
                        .frame(maxWidth: .infinity)
                        .aspectRatio(1, contentMode: .fit)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(StickerPanel.accessibilityName(item))
                .accessibilityHint("Sends this sticker")
            }
        }
    }

    /// What a screen reader calls one sticker: the few words whoever added
    /// it gave, or the plain word when they gave none.
    static func accessibilityName(_ item: PackItemSnapshot) -> String {
        if let label = item.label, !label.isEmpty {
            return String(localized: "Sticker: \(label)")
        }
        return String(localized: "Sticker")
    }
}

/// The composer's sticker button and the panel it opens, as one control.
///
/// Its own view, holding its own presentation state, for a mundane reason
/// both conversation views already give: their modifier chains are at what
/// the type checker will solve, and a sheet added there is a build that
/// stops finishing. The composer passes what a tap should do and nothing
/// else.
struct StickerComposerButton: View {
    /// The side of the composer's other controls, so the three are level.
    let side: CGFloat
    /// The glyph's point size, which differs between the two composers.
    let glyph: CGFloat
    let onPick: (PackItemSnapshot) -> Void

    @State private var showsPanel = false

    var body: some View {
        Button {
            showsPanel = true
        } label: {
            Image(systemName: "face.smiling")
                .font(.system(size: glyph))
                .frame(width: side, height: side)
                .contentShape(Rectangle())
        }
        #if os(macOS)
        .buttonStyle(.borderless)
        .help("Stickers")
        .popover(isPresented: $showsPanel, arrowEdge: .top) {
            panel.frame(width: 380, height: 360)
        }
        #else
        .foregroundStyle(.tint)
        .sheet(isPresented: $showsPanel) {
            NavigationStack {
                panel
                    .navigationTitle("Stickers")
                    .inlineNavigationTitle()
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Done") { showsPanel = false }
                        }
                    }
            }
            // Half the screen, so the chat the sticker is going into stays
            // in view — and draggable up for a pack that needs the room.
            .presentationDetents([.medium, .large])
        }
        #endif
        .accessibilityLabel("Stickers")
    }

    private var panel: some View {
        StickerPanel { item in
            // Down first: the send appends a row, and the thread scrolling
            // to it under a sheet that is still up is a send nobody sees.
            showsPanel = false
            onPick(item)
        }
    }
}
