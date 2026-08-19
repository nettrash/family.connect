//
//  SyncPlan.swift
//  FamilyConnect
//
//  The pure "what do we need to fetch" half of resync step 3
//  (protocol.md §Best-effort delivery): given the server's chat list and
//  our local per-chat cursors, produce the `after_id` fetch steps.
//
//  Message ids are globally monotonic, so comparing the server's
//  last-message id against the largest id we hold locally decides, per
//  chat, whether a catch-up loop is needed at all — chats with nothing
//  new produce no step and cost no request. Termination on a short page
//  is the CALLER's job (the coordinator loops each step until a page
//  comes back shorter than the limit); this type stays a pure function
//  so the planning rules are unit-testable as a table.
//

import Foundation

nonisolated enum SyncPlan {

    /// What the server told us about one chat in GET /chats.
    struct ChatCursor: Equatable, Sendable {
        let chatID: Int64
        /// id of the chat's newest message, nil when the chat is empty.
        let serverLatestMessageID: Int64?

        init(chatID: Int64, serverLatestMessageID: Int64?) {
            self.chatID = chatID
            self.serverLatestMessageID = serverLatestMessageID
        }
    }

    /// One catch-up loop to run: GET /chats/{chatID}/messages?after_id=
    /// {afterID}, repeated (advancing afterID) until a short page.
    struct FetchStep: Equatable, Sendable {
        let chatID: Int64
        let afterID: Int64

        init(chatID: Int64, afterID: Int64) {
            self.chatID = chatID
            self.afterID = afterID
        }
    }

    /// Plan the catch-up fetches. `localCursors` maps chatID → the largest
    /// server message id stored locally (absent = nothing stored, cursor 0).
    /// A step is emitted only when the server holds something newer.
    static func make(
        chats: [ChatCursor],
        localCursors: [Int64: Int64]
    ) -> [FetchStep] {
        chats.compactMap { chat in
            guard let latest = chat.serverLatestMessageID else { return nil }
            let local = localCursors[chat.chatID] ?? 0
            guard latest > local else { return nil }
            return FetchStep(chatID: chat.chatID, afterID: local)
        }
    }
}
