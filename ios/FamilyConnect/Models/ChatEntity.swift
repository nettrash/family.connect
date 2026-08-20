//
//  ChatEntity.swift
//  FamilyConnect
//
//  One row per chat the server knows about (the family chat plus any
//  direct chats). The server's GET /chats is the source of truth for the
//  *list*; this entity additionally carries the client-side sync state
//  that the server does not track for us:
//
//    - `maxServerMessageID` — the resync cursor. Message ids are globally
//      monotonic (protocol.md §Best-effort delivery), so "the largest id
//      we have ever stored for this chat" is exactly the `after_id` to
//      catch up from on reconnect.
//    - `oldestLoadedMessageID` / `hasFullHistory` — the *other* end of
//      the locally-loaded window, used by history pagination
//      (`before_id` pages) so scrolling up knows where to fetch from and
//      when to stop.
//    - `myLastReadID` — the largest id we've reported via the read
//      endpoint/frame. Doubles as the client-side throttle: a read
//      marker is only sent when it would advance this value.
//    - `othersReadUpTo` — max(last_read_message_id) observed from *other*
//      members' read frames; drives the double-checkmark on own bubbles.
//    - `maxReactionSeq` — the reaction resync cursor, the reaction twin
//      of `maxServerMessageID`: compared against the server's
//      max_reaction_seq (GET /chats) to decide whether a reaction
//      catch-up loop is needed.
//
//  `pinRank` (0 family / 1 direct) exists so the chat list can sort
//  "family chat always first, then direct chats by recency" with a plain
//  two-key sort instead of special-casing the family chat in the view.
//

import Foundation
import SwiftData

@Model
final class ChatEntity {
    /// Server chat id — the natural key; upserts match on it.
    @Attribute(.unique) var chatID: Int64
    /// "family" | "direct" (mirrors the wire values; kept as String so an
    /// unknown future kind round-trips instead of crashing a decode).
    var kind: String
    /// 0 = family chat (always pinned on top), 1 = direct chat.
    var pinRank: Int
    /// The other participant for direct chats; nil for the family chat.
    var peerUserID: Int64?
    /// Family name (family chat) or the peer's display name (direct).
    var title: String
    var lastMessagePreview: String?
    var lastMessageDate: Date?
    var lastMessageSenderID: Int64?
    /// Server-authoritative after every resync; incremented locally by
    /// live socket messages between resyncs.
    var unreadCount: Int = 0
    /// Resync cursor: the largest server message id stored locally.
    var maxServerMessageID: Int64 = 0
    /// The smallest server message id stored locally, i.e. the top of the
    /// loaded window. nil until the first page of history is loaded.
    var oldestLoadedMessageID: Int64?
    /// True once a history page came back short — there is nothing older.
    var hasFullHistory: Bool = false
    /// Largest id we've told the server we read (monotonic, throttled).
    var myLastReadID: Int64 = 0
    /// Largest id any *other* member reported reading.
    var othersReadUpTo: Int64 = 0
    /// Reaction resync cursor: the largest reaction_seq this client has
    /// *processed* for the chat — advanced by catch-up pages (even when
    /// the referenced message isn't held locally) and by live frames.
    /// Never derived from held messages; compared against the server's
    /// max_reaction_seq from GET /chats to decide whether to catch up.
    var maxReactionSeq: Int64 = 0

    init(
        chatID: Int64,
        kind: String,
        pinRank: Int,
        peerUserID: Int64? = nil,
        title: String,
        lastMessagePreview: String? = nil,
        lastMessageDate: Date? = nil,
        lastMessageSenderID: Int64? = nil,
        unreadCount: Int = 0,
        maxServerMessageID: Int64 = 0,
        oldestLoadedMessageID: Int64? = nil,
        hasFullHistory: Bool = false,
        myLastReadID: Int64 = 0,
        othersReadUpTo: Int64 = 0,
        maxReactionSeq: Int64 = 0
    ) {
        self.chatID = chatID
        self.kind = kind
        self.pinRank = pinRank
        self.peerUserID = peerUserID
        self.title = title
        self.lastMessagePreview = lastMessagePreview
        self.lastMessageDate = lastMessageDate
        self.lastMessageSenderID = lastMessageSenderID
        self.unreadCount = unreadCount
        self.maxServerMessageID = maxServerMessageID
        self.oldestLoadedMessageID = oldestLoadedMessageID
        self.hasFullHistory = hasFullHistory
        self.myLastReadID = myLastReadID
        self.othersReadUpTo = othersReadUpTo
        self.maxReactionSeq = maxReactionSeq
    }
}
