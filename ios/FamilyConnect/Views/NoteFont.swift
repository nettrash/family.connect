//
//  NoteFont.swift
//  FamilyConnect
//
//  The four hands a board note can be written in, shared by both platforms.
//
//  A font is an INTENT, not a typeface (docs/protocol.md, "Board"): the wire
//  carries a name — plain, serif, mono, casual — and each client resolves it
//  to a system face of its own, exactly as `size` is a step and `color` a
//  name. A family name on the wire would name a font one platform has and
//  another does not, and would leave a note unreadable on the phone it was
//  not written on. Nothing here is bundled and nothing is downloaded: all
//  four are `Font.Design`s the system already draws.
//
//  `casual` is the one that looks most different across platforms, and
//  deliberately so — it is "not the plain one", which every platform can
//  keep, rather than a promise that two devices draw the same curves.
//
//  "plain" is exactly what every note was before fonts existed, so a wall
//  with no fonts on it looks the same as it did.
//
//  Android counterpart: NoteFonts in ui/board/BoardScreen.kt.
//

import SwiftUI

/// Ordered as the picker shows them, plainest first.
nonisolated enum NoteFont: String, CaseIterable, Identifiable, Sendable {
    case plain
    case serif
    case mono
    case casual

    /// The wire name, exactly as the protocol spells it.
    var name: String { rawValue }
    var id: String { rawValue }

    /// An unknown name from a newer server falls back rather than failing —
    /// the note still has to be readable, and plain is the face it would
    /// have been in before the field existed.
    init(name: String?) {
        self = name.flatMap(NoteFont.init(rawValue:)) ?? .plain
    }

    /// What an edit PATCHes for font: the chosen name when the author
    /// changed it, nothing when they did not — the same rule `size` follows,
    /// and for the same reason. A fifth face from a newer server DRAWS as
    /// plain but must not be WRITTEN BACK as plain because the author fixed
    /// a typo in the text.
    func patchName(replacing stored: String?) -> String? {
        self == NoteFont(name: stored) ? nil : name
    }

    /// The picker label. Through the catalog, so "Plain" is not shipped
    /// verbatim to a Serbian family.
    var title: LocalizedStringKey {
        switch self {
        case .plain: "Plain"
        case .serif: "Serif"
        case .mono: "Mono"
        case .casual: "Casual"
        }
    }

    /// The system design each intent resolves to. `.rounded` is Apple's
    /// friendly face — the informal one this platform has to hand.
    var design: Font.Design {
        switch self {
        case .plain: .default
        case .serif: .serif
        case .mono: .monospaced
        case .casual: .rounded
        }
    }

    /// The size's own type, in this hand.
    ///
    /// Through `Font.system(_:design:)` rather than a modifier chain so the
    /// note keeps a SEMANTIC size and goes on tracking Dynamic Type: a
    /// point size fixed here would stop the wall growing with the reader's
    /// setting, which is a regression the fitting rule would then hide.
    func font(for size: NoteSize) -> Font {
        Font.system(size.textStyle, design: design)
    }
}
