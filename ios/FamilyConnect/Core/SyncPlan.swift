//
//  SyncPlan.swift
//  FamilyConnect
//
//  The pure "what do we need to fetch" half of resync step 3
//  (protocol.md §Best-effort delivery): given the server's chat list and
//  our local per-chat cursors, produce the `after_id` fetch steps — and,
//  by the same rule over the reaction cursors, the `after_seq` reaction
//  catch-up steps.
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
        /// max_reaction_seq from GET /chats; 0 when the server omitted it
        /// (no message in the chat was ever reacted to).
        let serverMaxReactionSeq: Int64
        /// max_edit_seq from GET /chats; 0 when the server omitted it
        /// (nothing in the chat has been edited).
        let serverMaxEditSeq: Int64
        /// max_poll_seq from GET /chats; 0 when the server omitted it
        /// (the chat holds no poll at all).
        let serverMaxPollSeq: Int64

        init(
            chatID: Int64,
            serverLatestMessageID: Int64?,
            serverMaxReactionSeq: Int64 = 0,
            serverMaxEditSeq: Int64 = 0,
            serverMaxPollSeq: Int64 = 0
        ) {
            self.chatID = chatID
            self.serverLatestMessageID = serverLatestMessageID
            self.serverMaxReactionSeq = serverMaxReactionSeq
            self.serverMaxEditSeq = serverMaxEditSeq
            self.serverMaxPollSeq = serverMaxPollSeq
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

    /// One reaction catch-up loop to run: GET /chats/{chatID}/reactions?
    /// after_seq={afterSeq}, repeated (advancing afterSeq) until a short
    /// page.
    struct ReactionFetchStep: Equatable, Sendable {
        let chatID: Int64
        let afterSeq: Int64

        init(chatID: Int64, afterSeq: Int64) {
            self.chatID = chatID
            self.afterSeq = afterSeq
        }
    }

    /// Plan the edit catch-ups — the exact shape of the reaction plan,
    /// against the second cursor. A chat nothing has been edited in costs
    /// no request.
    static func makeEditSteps(
        chats: [ChatCursor],
        localCursors: [Int64: Int64]
    ) -> [ReactionFetchStep] {
        chats.compactMap { chat in
            let local = localCursors[chat.chatID] ?? 0
            guard chat.serverMaxEditSeq > local else { return nil }
            return ReactionFetchStep(chatID: chat.chatID, afterSeq: local)
        }
    }

    /// Plan the poll catch-ups — the reaction plan's shape again, against
    /// the third cursor. A chat that holds no poll (server 0) or is
    /// already caught up costs no request.
    static func makePollSteps(
        chats: [ChatCursor],
        localCursors: [Int64: Int64]
    ) -> [ReactionFetchStep] {
        chats.compactMap { chat in
            let local = localCursors[chat.chatID] ?? 0
            guard chat.serverMaxPollSeq > local else { return nil }
            return ReactionFetchStep(chatID: chat.chatID, afterSeq: local)
        }
    }

    /// Plan the reaction catch-ups. `localCursors` maps chatID → the
    /// stored ChatEntity.maxReactionSeq (absent = nothing processed,
    /// cursor 0). A step is emitted only when the server's
    /// max_reaction_seq is ahead of the stored cursor — chats with no
    /// reaction activity (server 0 or equal) cost no request.
    static func makeReactionSteps(
        chats: [ChatCursor],
        localCursors: [Int64: Int64]
    ) -> [ReactionFetchStep] {
        chats.compactMap { chat in
            let local = localCursors[chat.chatID] ?? 0
            guard chat.serverMaxReactionSeq > local else { return nil }
            return ReactionFetchStep(chatID: chat.chatID, afterSeq: local)
        }
    }
}
