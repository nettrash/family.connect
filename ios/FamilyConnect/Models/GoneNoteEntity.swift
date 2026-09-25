//
//  GoneNoteEntity.swift
//  FamilyConnect
//
//  A note a TOMBSTONE has taken, and it never comes back (docs/protocol.md,
//  "Board").
//
//  A delete is the last thing that happens to a note — but board seqs commit
//  out of order and a catch-up page carries the PRE-delete copy, so without
//  this a client that merely deleted the row was talked out of it by the next
//  answer that mentioned the note, and nothing but a full read took it off
//  the wall again. The protocol says it outright: "a note a client has seen
//  deleted — by a tombstone, by a full read that left it out, or by its own
//  DELETE — is never brought back by an older copy of itself arriving late,
//  from a page, a frame or a reply that was already in flight."
//
//  THE PER-NOTE SEQ GUARD IS NO DEFENCE HERE, which is the part that is easy
//  to miss: the copy still in flight may carry a seq ABOVE the tombstone's
//  (another member's change to the same note, serialised before the delete
//  committed), so "refuse anything older" lets it straight through. What
//  makes remembering cheap is that note ids are never reused — the id alone
//  is the whole row.
//
//  A SEPARATE ENTITY rather than a flag on NoteEntity, for BlockEntity's
//  first reason: `applyNote` rewrites every field of a note on each upsert,
//  so a flag living there would be reset by the very answer it exists to
//  refuse. And a tombstone for a note this device never held has no row to
//  put a flag on at all.
//
//  One row per note the family has ever deleted, which is a handful a week at
//  worst and never pruned: an id that can never be reused cannot stop being
//  gone. The web client keeps the same set in memory; Android and Windows
//  keep the same table.
//

import Foundation
import SwiftData

@Model
final class GoneNoteEntity {
    /// The deleted note's server id — the natural key, and the whole row.
    @Attribute(.unique) var noteID: Int64

    init(noteID: Int64) {
        self.noteID = noteID
    }
}
