//
//  NoteSize.swift
//  FamilyConnect
//
//  The three sizes a board note comes in, shared by both platforms.
//
//  A size is a STEP, not a measurement (docs/protocol.md, "Board"): the
//  wire carries a name — small, medium, large — and each client draws it
//  at its own idiom, the way `color` is a name and not a hex value. So
//  the names and their order live here once, and the pixels live behind
//  a platform check: a large note is bigger on a Mac than on a phone, as
//  everything is, and the phone's square sticker is a landscape card on
//  the Mac. The author label under the text keeps its small style on
//  every size — it is a signature, not the message.
//
//  "medium" is exactly what every note was before sizes existed, on both
//  platforms, so a wall with no sizes on it looks the same as it did.
//
//  Android counterpart: NoteSizes in ui/board/BoardScreen.kt.
//

import SwiftUI

/// Ordered small → large, which is the order a picker shows them in.
nonisolated enum NoteSize: String, CaseIterable, Identifiable, Sendable {
    case small
    case medium
    case large

    /// The wire name, exactly as the protocol spells it.
    var name: String { rawValue }
    var id: String { rawValue }

    /// An unknown name from a newer server falls back rather than failing —
    /// the note still has to be readable, and medium is the size it would
    /// have been before the field existed.
    init(name: String?) {
        self = name.flatMap(NoteSize.init(rawValue:)) ?? .medium
    }

    /// What an edit PATCHes for size: the chosen name when the author
    /// changed it, nothing when they did not. The distinction is for a
    /// name this client does not know — a fourth size from a newer server
    /// DRAWS as medium (the fallback above) but must not be WRITTEN back
    /// as medium because the author fixed a typo in the text. The entity
    /// keeps the raw name for exactly that; an untouched picker sends none.
    func patchName(replacing stored: String?) -> String? {
        self == NoteSize(name: stored) ? nil : name
    }

    /// The picker label. Through the catalog, so "Small" is not shipped
    /// verbatim to a Serbian family.
    var title: LocalizedStringKey {
        switch self {
        case .small: "Small"
        case .medium: "Medium"
        case .large: "Large"
        }
    }

    /// Type size climbs with the sticker: a large note is meant to be read
    /// from across the room, not to hold more of the same small print.
    var font: Font {
        switch self {
        case .small: .footnote
        case .medium: .callout
        case .large: .body
        }
    }

    #if os(iOS)
    /// The phone's sticker is square; medium is the 132pt it always was.
    var side: CGFloat {
        switch self {
        case .small: 100
        case .medium: 132
        case .large: 220
        }
    }

    var frame: CGSize { CGSize(width: side, height: side) }

    var lineLimit: Int {
        switch self {
        case .small: 3
        case .medium: 5
        case .large: 10
        }
    }
    #elseif os(macOS)
    /// The Mac's sticker is a landscape card; medium is the 150×110 it
    /// always was.
    var frame: CGSize {
        switch self {
        case .small: CGSize(width: 120, height: 88)
        case .medium: CGSize(width: 150, height: 110)
        case .large: CGSize(width: 280, height: 200)
        }
    }

    var lineLimit: Int {
        switch self {
        case .small: 3
        case .medium: 4
        case .large: 9
        }
    }
    #endif
}
