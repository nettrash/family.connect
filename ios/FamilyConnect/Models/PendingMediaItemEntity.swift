//
//  PendingMediaItemEntity.swift
//  FamilyConnect
//
//  One attachment of a media send that has not been claimed yet.
//
//  WHY THIS EXISTS. A media send used to live nowhere but a running Task:
//  `sendMedia` uploaded every attachment and only then enqueued the message
//  row, so suspending the app, losing the process or running out of
//  background allowance lost the whole send with no bubble, no error and
//  nothing to retry — and for a camera capture or a voice note the staging
//  file in `tmp` was the only copy of the content. A text message never had
//  that problem, because it inserts its row first and the outbox re-sends
//  it.
//
//  THE FIX IS TO MAKE A MEDIA SEND THE SAME SHAPE AS A TEXT SEND. The
//  message row IS the send: `sendMedia` enqueues a `.pending`
//  `MessageEntity` before a single byte leaves, and these rows are what it
//  still owes the server. Everything the outbox gained — the transient
//  classification, the retry ladder, the flush on connect, on foreground,
//  on the network returning — then applies to media for free, because
//  there is only one queue and it is the one that was already there
//  (docs/protocol.md, "Sending on an unreliable network").
//
//  Flat foreign key rather than a `@Relationship`, following the rule the
//  reply columns already state in MessageEntity: nothing in this store uses
//  relationships, and a to-many would change delete semantics on the one
//  entity every view reads.
//

import Foundation
import SwiftData

@Model
final class PendingMediaItemEntity {

    /// Stable identity, and the name of this item's staging directory.
    @Attribute(.unique) var itemID: String
    /// The `"c:<uuid>"` localID of the `MessageEntity` this belongs to.
    var messageLocalID: String
    /// Denormalised so a sweep never needs a join.
    var chatID: Int64
    /// The sender's order, 0-based. `attachment_ids` is order-significant
    /// (docs/protocol.md, "Photos, videos, audio, files and locations"), so
    /// the order is stored rather than taken from a fetch's row order.
    var position: Int

    // MARK: - What to upload

    /// RELATIVE path under the staging root: `<itemID>/<real name>`. Never
    /// an absolute URL — the container path changes across reinstalls and
    /// OS upgrades, and a stored absolute path is a dangling one waiting to
    /// happen. nil for a location, which has no bytes at all.
    var fileName: String?
    /// The preview JPEG written beside it, `<itemID>/preview.jpg`, or nil
    /// where there is nothing to draw. It used to live only in memory on
    /// `MediaPrep.Prepared`, which is why a poster could not survive a
    /// relaunch.
    var previewFileName: String?
    var mime: String
    /// "photo" | "video" | "audio" | "file" | "location".
    var kind: String
    var width: Int?
    var height: Int?
    var durationMS: Int?
    /// Required for a file, a label for audio and locations.
    var name: String?

    // MARK: - Locations

    /// A location IS these three numbers (docs/protocol.md, "Locations"),
    /// and there are no bytes to fall back on, so an unsaved fix is a lost
    /// one.
    var latitude: Double?
    var longitude: Double?
    var accuracyM: Int?

    // MARK: - What has already happened

    /// Set the instant `POST /attachments` answers, and saved in the same
    /// transaction as the parent's `pendingAttachmentCount` decrement.
    /// Non-nil means: do not upload this again. The id stays good for the
    /// server's unclaimed grace, so a resumed send pushes only what it
    /// still owes rather than the whole set again.
    var attachmentID: Int64?
    /// Whether the preview PUT landed. Best-effort, so it never blocks a
    /// send; recorded so a resumed send neither re-PUTs a preview that
    /// arrived nor claims one that did not.
    var previewUploaded: Bool = false
    /// Bounded re-uploads of THIS item, so one unsendable video cannot be
    /// pushed forever. The parent's `sendAttempts` counts DELIVERY
    /// attempts, which is a different budget spent on a different request.
    var uploadAttempts: Int = 0
    /// When the set was queued. The clock a stale-send policy reads: bytes
    /// held in durable storage are bytes the system can no longer reclaim
    /// on its own, so something has to give up eventually.
    var createdAt: Date

    init(
        itemID: String = UUID().uuidString.lowercased(),
        messageLocalID: String,
        chatID: Int64,
        position: Int,
        fileName: String? = nil,
        previewFileName: String? = nil,
        mime: String,
        kind: String,
        width: Int? = nil,
        height: Int? = nil,
        durationMS: Int? = nil,
        name: String? = nil,
        latitude: Double? = nil,
        longitude: Double? = nil,
        accuracyM: Int? = nil,
        createdAt: Date = Date()
    ) {
        self.itemID = itemID
        self.messageLocalID = messageLocalID
        self.chatID = chatID
        self.position = position
        self.fileName = fileName
        self.previewFileName = previewFileName
        self.mime = mime
        self.kind = kind
        self.width = width
        self.height = height
        self.durationMS = durationMS
        self.name = name
        self.latitude = latitude
        self.longitude = longitude
        self.accuracyM = accuracyM
        self.createdAt = createdAt
    }
}

extension PendingMediaItemEntity {

    /// The id this item's bubble draws under before the server has given
    /// it a real one.
    ///
    /// NEGATIVE, so it can never collide with a server id (a positive
    /// BIGSERIAL), and DETERMINISTIC, so it survives a relaunch — Swift's
    /// own `hashValue` is seeded per process and would hand the resumed
    /// send a different id from the one its cached pixels were written
    /// under. The same trick the optimistic poll plays with its option ids.
    var provisionalAttachmentID: Int64 {
        var hash: UInt64 = 5381
        for byte in itemID.utf8 { hash = hash &* 33 &+ UInt64(byte) }
        let magnitude = Int64(hash & 0x7FFF_FFFF_FFFF_FFFF)
        return magnitude == 0 ? -1 : -magnitude
    }

    /// What the bubble draws while the bytes are still going up: the real
    /// shape, the real dimensions, a provisional id. The pixels come from
    /// the staging directory by way of `AttachmentStore.seed`, so this
    /// device never fetches back what it is in the middle of sending.
    var provisionalDTO: AttachmentDTO {
        AttachmentDTO(
            id: provisionalAttachmentID,
            kind: kind,
            mime: mime,
            size: 0,
            width: width,
            height: height,
            durationMS: durationMS,
            hasPreview: previewFileName != nil,
            name: name,
            latitude: latitude,
            longitude: longitude,
            accuracyM: accuracyM)
    }

    /// This item as the wire shape, once its bytes are on the server.
    ///
    /// `size` is 0 rather than the file's length: the row is a local
    /// record and the server's own copy replaces it whole on the ack.
    /// `hasPreview` reports what actually landed, never what was intended.
    var uploadedDTO: AttachmentDTO? {
        guard let attachmentID else { return nil }
        return AttachmentDTO(
            id: attachmentID,
            kind: kind,
            mime: mime,
            size: 0,
            width: width,
            height: height,
            durationMS: durationMS,
            hasPreview: previewUploaded,
            name: name,
            latitude: latitude,
            longitude: longitude,
            accuracyM: accuracyM)
    }
}
