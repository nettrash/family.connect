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
    /// One of the protocol's four hands — plain, serif, mono, casual — a
    /// String for the same reason as `color` and `size`, and defaulted for
    /// the same reason too: a store written before the field existed
    /// migrates in place, and "plain" is exactly the face those notes were
    /// drawn in.
    var font: String = "plain"
    /// `text` or `photo` (docs/protocol.md, "Board"), a String for the same
    /// reason as `color`: an unknown kind from a newer server draws as a
    /// text note rather than failing to decode. Defaulted so a store
    /// written before the field migrates in place — every note in it WAS a
    /// text note.
    var kind: String = "text"
    /// The pinned picture's id, on a photo note. The pixels come from
    /// AttachmentStore by this id, exactly as a message's do; nothing about
    /// the file is duplicated here.
    var attachmentID: Int64?
    /// What the tile needs before the bytes arrive: the shape to reserve.
    /// Nil when the server did not say, which is an older server or a
    /// picture whose dimensions were never recorded.
    var attachmentWidth: Int?
    var attachmentHeight: Int?
    /// An event's when and where (docs/protocol.md, "Board"). Nil on every
    /// other kind, and on every note written before events existed — which
    /// is what a lightweight migration fills them with.
    var startsAt: Date?
    var endsAt: Date?
    var place: String?
    /// Who is coming, as the wire's list stored verbatim (RsvpCodec). Nil
    /// on every other kind; "[]" on an event nobody has answered.
    var rsvpsJSON: String?
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
        font: String = "plain",
        kind: String = "text",
        attachmentID: Int64? = nil,
        attachmentWidth: Int? = nil,
        attachmentHeight: Int? = nil,
        startsAt: Date? = nil,
        endsAt: Date? = nil,
        place: String? = nil,
        rsvpsJSON: String? = nil,
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
        self.font = font
        self.kind = kind
        self.attachmentID = attachmentID
        self.attachmentWidth = attachmentWidth
        self.attachmentHeight = attachmentHeight
        self.startsAt = startsAt
        self.endsAt = endsAt
        self.place = place
        self.rsvpsJSON = rsvpsJSON
        self.x = x
        self.y = y
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.boardSeq = boardSeq
        self.contentSeq = contentSeq
    }
}

extension NoteEntity {
    /// The answers, decoded. Empty when there are none and when this is not
    /// an event — a caller that needs to tell those apart asks the kind.
    var rsvpList: [RsvpDTO] {
        RsvpCodec.decode(rsvpsJSON)
    }

    /// What this reader answered, if anything.
    func myAnswer(_ userID: Int64) -> String? {
        rsvpList.first { $0.userID == userID }?.answer
    }

    /// How many said each thing, for the line under an event's title.
    func answerCount(_ answer: String) -> Int {
        rsvpList.count { $0.answer == answer }
    }
}

/// The board's own little codec: the wire's `rsvps` stored verbatim, so the
/// store holds exactly what the server said and nothing is re-derived.
enum RsvpCodec {
    static func encode(_ rsvps: [RsvpDTO]) -> String? {
        guard let data = try? JSONEncoder().encode(rsvps) else { return nil }
        return String(decoding: data, as: UTF8.self)
    }

    static func decode(_ raw: String?) -> [RsvpDTO] {
        guard let raw, let data = raw.data(using: .utf8),
              let list = try? JSONDecoder().decode([RsvpDTO].self, from: data)
        else { return [] }
        return list
    }
}

