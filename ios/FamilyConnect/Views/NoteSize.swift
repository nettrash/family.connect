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
//  A size no longer decides how many LINES show. The text fits the sticker
//  (docs/protocol.md, "Board"): the type scales down from the size's own
//  until the whole note is inside, and the per-size line counts that used
//  to cut it off are gone.
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
    ///
    /// This is the CEILING, not the size the text is always drawn at: a
    /// sticker shows all of what it says, so the type scales down from here
    /// until the whole text fits (docs/protocol.md, "Board").
    var font: Font { Font.system(textStyle) }

    /// The same step as a TEXT STYLE, which is what a hand can be applied
    /// to: `Font.system(_:design:)` takes one of these, and only a semantic
    /// style keeps the note tracking Dynamic Type (see NoteFont).
    var textStyle: Font.TextStyle {
        switch self {
        case .small: .footnote
        case .medium: .callout
        case .large: .body
        }
    }

    /// How far the type may shrink before the text is cut instead.
    ///
    /// A FLOOR, because type small enough to be unreadable communicates no
    /// better than an ellipsis — below it the text truncates as it always
    /// did, and that is the one case a reader opens the note for. Expressed
    /// as a fraction of the size's own type so it tracks Dynamic Type: the
    /// three sizes shrink by the same proportion rather than to the same
    /// absolute point size, which on a large accessibility setting would be
    /// a floor above the ceiling.
    ///
    /// 0.6 is the smallest that keeps the 280-character cap readable at
    /// every step: a full note fits `small` at roughly footnote × 0.6, and
    /// anything less buys room for text nobody could read anyway.
    var minimumTextScale: CGFloat { 0.6 }

    /// The lines the fitted text may take.
    ///
    /// Deliberately generous rather than the old per-size count: fitting
    /// works by making the type smaller, and a cap of five lines would stop
    /// it long before the sticker was full. The number is a backstop for a
    /// single unbroken word, not a layout rule.
    var fittedLineLimit: Int { 20 }

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
    #endif
}
