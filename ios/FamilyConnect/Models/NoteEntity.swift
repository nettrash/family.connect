//
//  NoteEntity.swift
//  FamilyConnect
//
//  One sticker note on the family board, cached locally so the board draws
//  instantly and survives a launch offline.
//
//  A TOMBSTONE is not stored. The server keeps one so its change feed can
//  say "this note is gone"; a client that has been told simply deletes its
//  row — there is nothing left to remember, and a tombstone kept locally
//  would only have to be filtered out of every read.
//
//  `boardSeq` is the apply guard, the same shape as reactionSeq on a
//  message: a note is written only when the incoming seq is greater than
//  the one held, so an out-of-order frame cannot undo a newer move.
//
//  Android counterpart: NoteEntity in data/db/Entities.kt.
//

import Foundation
import SwiftData

@Model
final class NoteEntity {
    /// Server note id — the natural key; upserts match on it.
    @Attribute(.unique) var noteID: Int64
    var authorID: Int64
    var text: String
    /// One of the protocol's six names. Kept as a String: an unknown value
    /// from a newer server must render as *something* rather than fail to
    /// decode.
    var color: String
    /// One of the protocol's three step names — small, medium, large — a
    /// String for the same reason as `color`. Defaulted so a store written
    /// before the field existed migrates in place: SwiftData adds a new
    /// attribute with a default as a lightweight migration and fills every
    /// existing row with it, and "medium" is exactly the size those rows
    /// had when they were drawn.
    var size: String = "medium"
    /// Fractions of the board, 0…1 from the top-left, so a note sits in the
    /// same relative place on a phone and a tablet.
    var x: Double
    var y: Double
    var createdAt: Date
    var updatedAt: Date
    /// Highest board_seq applied to this row. The per-note guard.
    var boardSeq: Int64
    /// The seq of the last change to what this note SAYS, and the only
    /// number the board badge counts (docs/protocol.md, "Board").
    ///
    /// ZERO MEANS UNKNOWN, and it is the default for exactly two reasons at
    /// once: a row written before this field existed (SwiftData fills it in
    /// as a lightweight migration) and a note from a server that predates
    /// the field. Both are "nothing here can say when this text was
    /// written", and both fall back to the note-id rule the badge used
    /// before — see BoardBadge.
    var contentSeq: Int64 = 0

    init(
        noteID: Int64,
        authorID: Int64,
        text: String,
        color: String,
        size: String = "medium",
        x: Double,
        y: Double,
        createdAt: Date,
        updatedAt: Date,
        boardSeq: Int64,
        contentSeq: Int64 = 0
    ) {
        self.noteID = noteID
        self.authorID = authorID
        self.text = text
        self.color = color
        self.size = size
        self.x = x
        self.y = y
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.boardSeq = boardSeq
        self.contentSeq = contentSeq
    }
}
