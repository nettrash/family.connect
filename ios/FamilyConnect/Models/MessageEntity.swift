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

    /// Typed view over the raw `status` int (SwiftData persists the int;
    /// the enum keeps call sites honest).
    var state: MessageStatus {
        get { MessageStatus(rawValue: status) ?? .sent }
        set { status = newValue.rawValue }
    }

    init(
        localID: String,
        serverID: Int64? = nil,
        clientMsgID: String? = nil,
        chatID: Int64,
        senderID: Int64,
        body: String,
        createdAt: Date,
        status: MessageStatus
    ) {
        self.localID = localID
        self.serverID = serverID
        self.clientMsgID = clientMsgID
        self.chatID = chatID
        self.senderID = senderID
        self.body = body
        self.createdAt = createdAt
        self.status = status.rawValue
    }
}
