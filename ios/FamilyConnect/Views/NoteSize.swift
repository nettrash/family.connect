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

/// The wall's own look: a cork ground, and a pin through every note.
///
/// Decoration, and nowhere on the wire (docs/protocol.md, "Board"): where a
/// pin sits is not a fact about the note, and a client that draws neither
/// is not wrong. The colours are fixed rather than taken from the
/// appearance — a corkboard is a corkboard with the lamp on or off, which
/// is the same reason the pastels on top are fixed light colours and the
/// ink on them is forced dark.
///
/// Web counterpart: `.board-wall` and `.sticker::after` in web/styles.css.
/// Android counterpart: `BoardGround` in ui/board/BoardScreen.kt.
struct BoardGround: View {
    @Environment(\.colorScheme) private var scheme

    private var cork: Color {
        scheme == .dark
            ? Color(red: 0.357, green: 0.290, blue: 0.212)
            : Color(red: 0.796, green: 0.702, blue: 0.569)
    }

    private var shade: Color {
        scheme == .dark
            ? Color(red: 0.298, green: 0.239, blue: 0.173)
            : Color(red: 0.749, green: 0.639, blue: 0.510)
    }

    var body: some View {
        // Lit from the top-left, the way a wall in a room is.
        LinearGradient(
            colors: [cork, cork, shade],
            startPoint: .topLeading,
            endPoint: .bottomTrailing)
    }
}

/// The pin, drawn over a note's top edge.
struct NotePin: View {
    var body: some View {
        Circle()
            .fill(
                RadialGradient(
                    colors: [
                        Color(red: 0.847, green: 0.325, blue: 0.298),
                        Color(red: 0.478, green: 0.102, blue: 0.082),
                    ],
                    center: UnitPoint(x: 0.35, y: 0.3),
                    startRadius: 0,
                    endRadius: 9))
            .frame(width: 10, height: 10)
            .shadow(color: .black.opacity(0.35), radius: 1, y: 1)
            // It is not a control and not content: a reader is told about
            // the note, never about its pin.
            .accessibilityHidden(true)
    }
}

/// How the wall itself is sized (docs/protocol.md, "Board").
///
/// The wall is TALLER than the window and it scrolls: a wall the size of
/// the window is a wall that fills up, and then a family has to take
/// something down before it can say anything. `x` and `y` stay fractions
/// of the WALL, so making it taller moves nothing relative to anything
/// else.
///
/// The factor is the same on all four clients even though the wire says
/// nothing about it — a note two thirds of the way down should be two
/// thirds of the way down on the phone and on the Mac.
///
/// Web counterpart: `fc_text::board::WALL_SCREENS`.
/// Android counterpart: `BoardWall.screens` in ui/board/BoardScreen.kt.
nonisolated enum BoardWall {
    static let screens: CGFloat = 1.6

    /// The wall's height for a window of `visible` height — never shorter
    /// than the window, or fractions of the wall would sit behind its
    /// edges.
    static func height(visible: CGFloat) -> CGFloat {
        max(visible * screens, visible)
    }

    /// The wall's own size, for the fractions to be read against.
    static func size(visible: CGSize) -> CGSize {
        CGSize(width: visible.width, height: height(visible: visible.height))
    }
}

/// How much of a task list a STICKER draws (docs/protocol.md, "Board").
///
/// One number for all four clients, like `BoardWall.screens`, and for the
/// same reason: a list that ran to a different point on the phone and on
/// the Mac would be a different list. The note itself always has them all.
///
/// Web counterpart: `fc_text::board::WALL_TASK_LINES`.
nonisolated enum BoardTasks {
    static let onWall = 5

    /// The lines a sticker draws, and how many it had to leave — `left` is
    /// zero on a list that fits.
    static func drawn(of total: Int) -> (shown: Int, left: Int) {
        let shown = min(total, onWall)
        return (shown, total - shown)
    }
}

/// How a PICTURE is drawn on the wall (docs/protocol.md, "Board").
///
/// A PHOTO IS DRAWN WHOLE: fitted in both dimensions and never cropped to
/// fill its box. Issue #71 is what filling costs — a portrait photograph
/// from a phone lost more than half its height on the board, faces and all,
/// on every client that fitted the width alone.
///
/// Web counterpart: `fc_text::board::fitted_picture`.
/// Android counterpart: `BoardPicture.fitted` in ui/board/BoardScreen.kt.
nonisolated enum BoardPicture {
    /// The size a picture of `picture` pixels takes inside `space`, fitted
    /// in both dimensions — a tall photograph on a wide card comes back
    /// narrow, a wide one short, and neither comes back cropped.
    ///
    /// Also the size of a BARE photo's card, which is the picture itself:
    /// the note's box hugs this, so the pin sits on the photograph rather
    /// than over bare wall.
    ///
    /// A picture the server never gave dimensions for takes the whole
    /// space, which costs a margin at worst — the picture is still drawn
    /// fitted inside it and never cropped.
    static func fitted(space: CGSize, picture: CGSize) -> CGSize {
        guard picture.width > 0, picture.height > 0, space.width > 0, space.height > 0 else {
            return space
        }
        let scale = min(space.width / picture.width, space.height / picture.height)
        // Never below a hairline: a panorama 20 000 pixels wide would round
        // its height to nothing, and a card of no height is a note nobody
        // can tap.
        return CGSize(
            width: max(1, min(space.width, picture.width * scale)),
            height: max(1, min(space.height, picture.height * scale)))
    }
}

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
