//
//  MessageEntity.swift
//  FamilyConnect
//
//  One row per message bubble. The dual-id design is the heart of the
//  optimistic-send pipeline and deserves spelling out:
//
//  IDENTITY. `localID` is the stable primary key of a bubble for its whole
//  lifetime, so SwiftUI never sees a bubble disappear and reappear:
//
//    - A message *we* send is born as `"c:<uuid>"` (c = client) with
//      `serverID == nil`, `status == pending`. When the ack (WS) or the
//      201/200 (REST) arrives, the SAME row gains its `serverID`, its
//      `createdAt` is REWRITTEN to the server's authoritative timestamp,
//      and `status` flips to `sent`. The localID never changes — the
//      bubble keeps its identity, only its glyph and position settle.
//
//    - A message that arrives from the server first (someone else's, or
//      our own echoed to another device) is keyed `"s:<serverID>"`.
//      Re-delivery of the same id (resync overlapping a live frame) hits
//      the unique constraint match and becomes an idempotent update.
//
//    - The bridge between the two: an inbound frame whose
//      (chat_id, client_msg_id) matches a local `"c:"` row is OUR message
//      coming back — it reconciles into that row instead of inserting a
//      duplicate `"s:"` row. This is the whole dedup matrix.
//
//  ORDERING. Bubbles sort by (createdAt, localID). Confirmed messages all
//  carry server timestamps, so they order exactly as the server saw them
//  (ids are monotonic with time). A still-pending message carries its
//  local wall-clock time, which places it at the bottom where the user
//  just typed it; the ack's timestamp rewrite then snaps it into the
//  server's order. localID is the tiebreaker so equal timestamps are
//  stable across fetches.
//
//  `clientMsgID` is kept (not derived from localID) because inbound rows
//  also carry the sender's uuid and the dedup lookup needs an indexed
//  field to match on.
//

import Foundation
import SwiftData

/// Delivery state of an outbound message; inbound messages are `sent`.
/// Raw values are the persisted `status` integers.
nonisolated enum MessageStatus: Int, Sendable {
    case pending = 0
    case sent = 1
    case failed = 2
}

@Model
final class MessageEntity {
    /// Stable bubble identity: "c:<uuid>" for messages born locally,
    /// "s:<serverID>" for messages first seen from the server.
    @Attribute(.unique) var localID: String
    /// Server message id once known. nil only while pending/failed.
    var serverID: Int64?
    /// The idempotency uuid (ours on send; the sender's on inbound).
    var clientMsgID: String?
    var chatID: Int64
    var senderID: Int64
    var body: String
    /// Local wall-clock at enqueue time; rewritten to the server's
    /// authoritative timestamp when the ack/echo reconciles the row.
    var createdAt: Date
    /// 0 pending, 1 sent, 2 failed — see `MessageStatus`.
    var status: Int
    /// Full current reaction state, JSON-encoded in the wire shape
    /// (`[{"user_id":9,"emoji":"❤️"}]`). nil = never reacted to; "[]" =
    /// reacted and later cleared — the distinction the protocol keeps.
    var reactionsJSON: String?
    /// Highest reaction_seq applied to this row (0 = none). The
    /// per-message guard: a state only lands when its seq is greater.
    var reactionSeq: Int64 = 0
    /// The poll on this message, JSON-encoded in the wire shape
    /// (`{"poll_seq":88,"closed":false,"options":[…]}`). nil = not a
    /// poll, which is the ONLY thing absence ever means here: a poll dies
    /// with its message, so nothing on the wire ever takes one off, and
    /// an incoming copy without one must never clear this.
    var pollJSON: String?
    /// Highest poll_seq applied to this row — the per-message guard, the
    /// twin of `reactionSeq`. 0 = no server state applied yet, which is
    /// also what an optimistic just-sent poll carries, so the ack's
    /// authoritative copy (real option ids, a real seq) passes the guard
    /// rather than being dropped as stale.
    var pollSeq: Int64 = 0
    /// The quoted message, when this one is a reply. Held as three flat
    /// properties rather than a relationship: the quote is a SNAPSHOT the
    /// server recomputes on every read (docs/protocol.md, "Replies"), and
    /// the quoted row may not be in this device's cache at all. All three
    /// default to nil, so this is a lightweight SwiftData migration for
    /// anyone upgrading over an existing store.
    var replyToMessageID: Int64?
    var replySenderID: Int64?
    var replyExcerpt: String?
    /// The SECOND level: what the quoted message was itself answering.
    /// Three more nil-defaulted columns, so still a lightweight migration.
    /// Absent is normal and not a half-set row — the quoted message simply
    /// was not a reply, or its own parent has been swept.
    var replyParentMessageID: Int64?
    var replyParentSenderID: Int64?
    var replyParentExcerpt: String?
    /// Set once the body has been edited. `editSeq` is the apply guard:
    /// a stored body is overwritten only by a body at least as new, or a
    /// history page fetched before an edit would restore the old text
    /// (docs/protocol.md, "Editing"). 0 = never edited.
    var editSeq: Int64 = 0
    var editedAt: Date?
    /// The FULL attachment set this message carries, JSON-encoded in the
    /// WIRE shape (`[{"id":34,"kind":"photo",…}]`) — the pollJSON
    /// precedent, for the same reason: the stored value diffs against
    /// protocol.md. nil on a message with no attachments AND on every row
    /// written before plurality; the flat columns below are that older
    /// row's single attachment, which is why `attachmentList` falls back
    /// to them. A nil-defaulted column, so a lightweight migration.
    var attachmentsJSON: String?
    /// The FIRST attachment, as flat columns. Kept beside the JSON for
    /// two reasons: every existing row predates `attachmentsJSON` and has
    /// only these, and a downgrade to a build that predates plurality
    /// still shows the first attachment instead of nothing. All three
    /// write sites write BOTH (the JSON and the first item's columns).
    var attachmentID: Int64?
    var attachmentKind: String?
    var attachmentMIME: String?
    var attachmentSize: Int64 = 0
    var attachmentWidth: Int?
    var attachmentHeight: Int?
    var attachmentDurationMS: Int?
    var attachmentHasPreview: Bool = false
    /// Files only: their name is the whole thing a row shows. Optional on
    /// audio and on a location, where it is a label.
    var attachmentName: String?
    /// Locations only, and always both together (docs/protocol.md,
    /// "Locations"). Stored on the row rather than fetched, because a
    /// location has no bytes at all — these three columns ARE the
    /// attachment, so a cached message draws its pin offline.
    var attachmentLatitude: Double?
    var attachmentLongitude: Double?
    var attachmentAccuracyM: Int?
    /// The call this message records, when it is one (docs/protocol.md,
    /// "Voice calls" and "Video"): `completed`, `missed`, `declined` or
    /// `failed`, the seconds it lasted when it was ever answered, and
    /// whether it was a VIDEO call. Three defaulted columns (two nil, one
    /// false — the `attachmentHasPreview` precedent), so a lightweight
    /// migration. nil outcome = not a call record, which is the ONLY
    /// thing absence means here — nothing on the wire ever takes a
    /// record off a message.
    ///
    /// `callVideo` was the column this row lacked for a release: without
    /// it every stored record rebuilt as voice and a video call's bubble
    /// read "Voice call" the moment it came back out of the store. A row
    /// cached BEFORE the column existed keeps `false` until the server's
    /// copy is applied again (a re-delivered history page goes through
    /// the coordinator's `applyCall`, which rewrites all three).
    var callOutcome: String?
    var callDurationSecs: Int?
    var callVideo: Bool = false

    /// The call record as the views want it, or nil when this is not one.
    var callSnapshot: CallDTO? {
        guard let callOutcome else { return nil }
        return CallDTO(outcome: callOutcome, durationSecs: callDurationSecs, video: callVideo)
    }

    /// Every attachment on this message, in the sender's order.
    ///
    /// Prefers the JSON set; falls back to the flat single-attachment
    /// snapshot for rows written before plurality. [] when the message
    /// carries nothing — the reading twin of the wire's read rule
    /// (MessageDTO.attachmentList).
    var attachmentList: [AttachmentDTO] {
        if let attachmentsJSON, !attachmentsJSON.isEmpty,
           let list = try? Self.attachmentsDecoder.decode(
               [AttachmentDTO].self, from: Data(attachmentsJSON.utf8)),
           !list.isEmpty {
            return list
        }
        return attachmentSnapshot.map { [$0] } ?? []
    }

    /// Write the whole attachment set: the JSON, and the first item into
    /// the flat columns (existing callers keep working; a downgrade keeps
    /// the first attachment). An EMPTY list is a caller bug rather than a
    /// wipe — nothing on the wire ever takes an attachment off a message,
    /// so this never clears (the absent-field rule, again).
    func applyAttachmentList(_ list: [AttachmentDTO]) {
        guard let first = list.first else { return }
        if let data = try? Self.attachmentsEncoder.encode(list) {
            attachmentsJSON = String(decoding: data, as: UTF8.self)
        }
        attachmentID = first.id
        attachmentKind = first.kind
        attachmentMIME = first.mime
        attachmentSize = first.size
        attachmentWidth = first.width
        attachmentHeight = first.height
        attachmentDurationMS = first.durationMS
        attachmentHasPreview = first.hasPreview
        attachmentName = first.name
        attachmentLatitude = first.latitude
        attachmentLongitude = first.longitude
        attachmentAccuracyM = first.accuracyM
    }

    /// The FIRST attachment as the views that predate plurality want it,
    /// or nil when there is none.
    var attachmentSnapshot: AttachmentDTO? {
        guard let attachmentID, let attachmentKind, let attachmentMIME else { return nil }
        return AttachmentDTO(
            id: attachmentID,
            kind: attachmentKind,
            mime: attachmentMIME,
            size: attachmentSize,
            width: attachmentWidth,
            height: attachmentHeight,
            durationMS: attachmentDurationMS,
            hasPreview: attachmentHasPreview,
            name: attachmentName,
            latitude: attachmentLatitude,
            longitude: attachmentLongitude,
            accuracyM: attachmentAccuracyM)
    }

    /// The quote as the views want it, or nil when this is not a reply.
    /// All three columns move together — a half-set row would be a bug
    /// upstream, and is treated as "not a reply" rather than drawn.
    var replySnapshot: ReplyToSnapshot? {
        guard let replyToMessageID, let replySenderID, let replyExcerpt else { return nil }
        // Same all-three-or-nothing rule one level down, and a missing
        // second level is expected rather than a bug.
        var parent: QuotedParentSnapshot?
        if let replyParentMessageID, let replyParentSenderID, let replyParentExcerpt {
            parent = QuotedParentSnapshot(
                messageID: replyParentMessageID,
                senderID: replyParentSenderID,
                excerpt: replyParentExcerpt)
        }
        return ReplyToSnapshot(
            messageID: replyToMessageID,
            senderID: replySenderID,
            excerpt: replyExcerpt,
            parent: parent)
    }

    /// Typed view over the raw `status` int (SwiftData persists the int;
    /// the enum keeps call sites honest).
    var state: MessageStatus {
        get { MessageStatus(rawValue: status) ?? .sent }
        set { status = newValue.rawValue }
    }

    /// Typed view over the raw `reactionsJSON` blob (SwiftData persists
    /// the string; the array keeps call sites honest). Reading a
    /// never-reacted row yields []; writing always stores full state.
    var reactionList: [ReactionSnapshot] {
        get {
            guard let reactionsJSON, !reactionsJSON.isEmpty else { return [] }
            let data = Data(reactionsJSON.utf8)
            return (try? Self.reactionDecoder.decode([ReactionSnapshot].self, from: data)) ?? []
        }
        set {
            guard let data = try? Self.reactionEncoder.encode(newValue) else { return }
            reactionsJSON = String(decoding: data, as: UTF8.self)
        }
    }

    /// Typed view over the raw `pollJSON` blob, in the same idiom the
    /// reaction list uses. nil for a message that is not a poll; writing
    /// nil is how a *local* row gives one up (nothing on the wire does).
    var poll: PollSnapshot? {
        get {
            guard let pollJSON, !pollJSON.isEmpty else { return nil }
            return try? Self.pollDecoder.decode(PollSnapshot.self, from: Data(pollJSON.utf8))
        }
        set {
            guard let newValue else {
                pollJSON = nil
                return
            }
            guard let data = try? Self.pollEncoder.encode(newValue) else { return }
            pollJSON = String(decoding: data, as: UTF8.self)
        }
    }

    /// One decoder, not one per read. This is called once per message per
    /// view-body pass — building a fresh `JSONDecoder` there made a cheap
    /// property allocation-bound, which showed up as real cost on the Mac
    /// where the whole thread used to be mapped on every pass. Both are
    /// stateless and configured with nothing, so sharing them is safe.
    private static let reactionDecoder = JSONDecoder()
    private static let reactionEncoder = JSONEncoder()
    /// The poll pair, cached for exactly the reason above — a poll bubble
    /// decodes its state on every body pass, and a fresh coder per read
    /// was the measured cost that made this a rule rather than a
    /// preference.
    private static let pollDecoder = JSONDecoder()
    private static let pollEncoder = JSONEncoder()
    /// The attachments pair, cached for exactly the same measured reason:
    /// `attachmentList` runs once per message per view-body pass.
    private static let attachmentsDecoder = JSONDecoder()
    private static let attachmentsEncoder = JSONEncoder()

    init(
        localID: String,
        serverID: Int64? = nil,
        clientMsgID: String? = nil,
        chatID: Int64,
        senderID: Int64,
        body: String,
        createdAt: Date,
        status: MessageStatus,
        reactionsJSON: String? = nil,
        reactionSeq: Int64 = 0,
        pollJSON: String? = nil,
        pollSeq: Int64 = 0,
        replyToMessageID: Int64? = nil,
        replySenderID: Int64? = nil,
        replyExcerpt: String? = nil,
        editSeq: Int64 = 0,
        editedAt: Date? = nil,
        attachment: AttachmentDTO? = nil,
        attachments: [AttachmentDTO]? = nil,
        call: CallDTO? = nil
    ) {
        self.localID = localID
        self.serverID = serverID
        self.clientMsgID = clientMsgID
        self.chatID = chatID
        self.senderID = senderID
        self.body = body
        self.createdAt = createdAt
        self.status = status.rawValue
        self.reactionsJSON = reactionsJSON
        self.reactionSeq = reactionSeq
        // The third of the three write sites a new message field has (the
        // other two are the coordinator's first-sight insert and its
        // apply-a-frame path). Missing this one drops the poll off every
        // message this device sees for the FIRST time in a history page —
        // silently, because the row is otherwise perfectly correct.
        self.pollJSON = pollJSON
        self.pollSeq = pollSeq
        self.replyToMessageID = replyToMessageID
        self.replySenderID = replySenderID
        self.replyExcerpt = replyExcerpt
        self.editSeq = editSeq
        self.editedAt = editedAt
        self.attachmentID = attachment?.id
        self.attachmentKind = attachment?.kind
        self.attachmentMIME = attachment?.mime
        self.attachmentSize = attachment?.size ?? 0
        self.attachmentWidth = attachment?.width
        self.attachmentHeight = attachment?.height
        self.attachmentDurationMS = attachment?.durationMS
        self.attachmentHasPreview = attachment?.hasPreview ?? false
        self.attachmentName = attachment?.name
        // A location IS these three columns — it has no bytes to fall back
        // on, so dropping them here does not degrade the bubble, it empties
        // it. This is the path a message ARRIVES on, which is why the bug
        // it caused was invisible to whoever sent the location and total
        // for everybody else.
        self.attachmentLatitude = attachment?.latitude
        self.attachmentLongitude = attachment?.longitude
        self.attachmentAccuracyM = attachment?.accuracyM
        // The first-sight path, same reason as the poll above: a record
        // that arrives in a history page has no other way in.
        self.callOutcome = call?.outcome
        self.callDurationSecs = call?.durationSecs
        self.callVideo = call?.video ?? false
        // The FIRST-SIGHT write site for the plural set (the other two
        // are the coordinator's apply-a-frame path and its own-send
        // path). Written after the flat columns above so that a set
        // handed in overrides the single `attachment` spelling — and a
        // caller passing only the single still gets a JSON set of one,
        // so new rows always carry both forms.
        if let attachments, !attachments.isEmpty {
            applyAttachmentList(attachments)
        } else if let attachment {
            applyAttachmentList([attachment])
        }
    }
}
