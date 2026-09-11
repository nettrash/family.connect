//
//  NoteKind.swift
//  FamilyConnect
//
//  What a note IS: words on a sticker, or a picture pinned to the wall
//  (docs/protocol.md, "Board").
//
//  A photo note, an event and a task list are notes in every other
//  respect — each takes
//  a slot anyone may move, counts against the same ceiling, rides the same
//  change feed and the same seq, and a block hides it the same way. There
//  is no second board and no second cursor, because a photo or a plan on
//  the family's wall is not a different wall.
//
//  A kind this client has never heard of DRAWS AS TEXT rather than being
//  dropped: the note still has a slot on a shared wall, and a hole in the
//  family's layout is worse than a sticker that says only what it says.
//
//  Android counterpart: NoteKinds in ui/board/BoardScreen.kt.
//
import Foundation

nonisolated enum NoteKind: String, CaseIterable, Sendable {
    case text
    case photo
    case event
    /// Something the family has to get done: the text is the list's title
    /// and `items` are the lines (docs/protocol.md, "Board").
    case tasks

    /// The wire name, exactly as the protocol spells it.
    var name: String { rawValue }

    /// An unknown kind from a newer server falls back to `text` — see the
    /// header for why that, and not "ignore the note".
    init(name: String?) {
        self = name.flatMap(NoteKind.init(rawValue:)) ?? .text
    }
}
