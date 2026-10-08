//
//  PackItemEntity.swift
//  FamilyConnect
//
//  One sticker of the family's pack, cached locally so the panel draws
//  instantly and a sticker can be sent with no network at all
//  (docs/protocol.md, "Sticker pack").
//
//  A WORD ABOUT THE WORD. Elsewhere in this app "sticker" is a note on the
//  board — `NoteEntity` says so in its first line. This is the other thing:
//  a small picture sent in a chat. The wire calls the collection a PACK
//  precisely so that no table has to be read twice, and the types here
//  follow it; "sticker" is what the screens say to people.
//
//  THE BOARD'S MACHINERY, UNCHANGED. `packSeq` is the apply guard — an item
//  is written only when the incoming seq is greater than the one held — and
//  a tombstone is not stored here: the row is deleted and its id goes into
//  `GonePackItemEntity`, exactly as a note's does.
//
//  The PIXELS are not in this row. They live in AttachmentStore under
//  `attachmentID`, which never names different bytes; this row is what a
//  panel needs to lay out before a single byte arrives, and what a send
//  needs to say about the copy it uploads.
//
//  Android counterpart: PackItemEntity in data/db/Entities.kt.
//

import Foundation
import SwiftData

@Model
final class PackItemEntity {
    /// Server item id — the natural key, and the panel's order: ids are
    /// handed out in the order stickers were added.
    @Attribute(.unique) var itemID: Int64
    /// Who added it. Half of the removal rule (the other half is being the
    /// family's owner), and still set after they have left.
    var addedBy: Int64
    /// The picture's id in AttachmentStore.
    var attachmentID: Int64
    /// `image/webp` or `image/png`.
    var mime: String
    /// The picture's byte count, as the server recorded it — the cheap half
    /// of "does the pack already hold this?", asked before any bytes are
    /// compared (`StickerPack.holds`).
    var size: Int64
    var width: Int?
    var height: Int?
    /// A few words for a screen reader, when whoever added it gave some.
    var label: String?
    var createdAt: Date
    /// Highest pack_seq applied to this row. The per-item guard.
    var packSeq: Int64

    init(
        itemID: Int64,
        addedBy: Int64,
        attachmentID: Int64,
        mime: String,
        size: Int64,
        width: Int? = nil,
        height: Int? = nil,
        label: String? = nil,
        createdAt: Date,
        packSeq: Int64
    ) {
        self.itemID = itemID
        self.addedBy = addedBy
        self.attachmentID = attachmentID
        self.mime = mime
        self.size = size
        self.width = width
        self.height = height
        self.label = label
        self.createdAt = createdAt
        self.packSeq = packSeq
    }
}

extension PackItemEntity {
    /// The item as a value a view or a send can hold without holding a
    /// model object — which does not leave the main actor, and may be
    /// deleted under a sheet that is still on screen.
    var snapshot: PackItemSnapshot {
        PackItemSnapshot(
            id: itemID,
            addedBy: addedBy,
            attachmentID: attachmentID,
            mime: mime,
            size: size,
            width: width,
            height: height,
            label: label)
    }
}

/// One pack item, as the panel and the send path read it.
nonisolated struct PackItemSnapshot: Equatable, Hashable, Identifiable, Sendable {
    let id: Int64
    let addedBy: Int64
    let attachmentID: Int64
    let mime: String
    let size: Int64
    let width: Int?
    let height: Int?
    let label: String?

    init(
        id: Int64,
        addedBy: Int64,
        attachmentID: Int64,
        mime: String,
        size: Int64,
        width: Int? = nil,
        height: Int? = nil,
        label: String? = nil
    ) {
        self.id = id
        self.addedBy = addedBy
        self.attachmentID = attachmentID
        self.mime = mime
        self.size = size
        self.width = width
        self.height = height
        self.label = label
    }
}

/// A pack item a TOMBSTONE has taken, and it never comes back.
///
/// `GoneNoteEntity`'s twin, for its reason: a removal is the last thing
/// that happens to an item, but pack seqs can cross on the wire, so a copy
/// already in flight — a page, a frame, the answer to somebody's own add —
/// may carry a seq ABOVE the tombstone's and the per-item guard would let
/// it straight through. Item ids are never reused, so the id alone is the
/// whole row (docs/protocol.md, "Sticker pack": "the gone set, exactly as
/// the board keeps it").
@Model
final class GonePackItemEntity {
    /// The removed item's server id — the natural key, and the whole row.
    @Attribute(.unique) var itemID: Int64

    init(itemID: Int64) {
        self.itemID = itemID
    }
}
