//
//  ChatSyncCoordinator.swift
//  FamilyConnect
//
//  The one place where the wire (REST + WebSocket) meets the store
//  (SwiftData). Owns the APIClient and the ChatSocket, consumes the
//  socket's event stream, and applies every inbound message through a
//  single dedup matrix so nothing else in the app ever has to reason
//  about duplicates:
//
//    inbound MessageDTO →
//      1. (chat_id, client_msg_id) matches a local row  → OUR message
//         coming back: update in place (serverID, server createdAt,
//         status → sent). The bubble keeps its "c:" identity.
//      2. a "s:<id>" row exists                         → re-delivery
//         (resync page overlapping a live frame): idempotent update.
//      3. otherwise                                     → insert "s:<id>".
//
//  SEND PIPELINE (optimistic): `send` inserts a pending row immediately
//  (the bubble appears before any network I/O), then a background deliver
//  races the socket ack against a 10 s clock; on loss it falls back to
//  REST *with the same client_msg_id* — the server dedups, so the worst
//  case of "ack was in flight while we POSTed" is the same message twice
//  on the wire and once in the store. Failure marks the row `failed` for
//  tap-to-retry (same uuid again — still idempotent).
//
//  RESYNC (protocol.md §Best-effort delivery, the socket is a live wire
//  not a queue): on every (re)connect and foregrounding —
//    1. GET /me           — membership reconcile (kicked → AppSession),
//    2. GET /families/mine — roster upsert for name resolution,
//    3. GET /chats        — chat upsert; server unread_count wins; local
//                           chats the server no longer lists are dropped,
//    4. SyncPlan after_id  — per-chat catch-up loops (limit 100) until a
//                           short page,
//    5. SyncPlan after_seq — per-chat reaction catch-up loops where the
//                           server's max_reaction_seq is ahead of ours,
//    6. outbox sweep      — pending rows older than 30 s re-delivered.
//
//  @MainActor because it mutates ModelContainer.mainContext and
//  @Observable state that SwiftUI reads; the blocking work all lives in
//  the APIClient / ChatSocket actors it awaits into.
//

import Foundation
import Observation
import os
import SwiftData

@MainActor @Observable
final class ChatSyncCoordinator {

    enum ConnectionState: Equatable {
        case connected
        case connecting
        case offline
    }

    // MARK: - Observable state

    private(set) var connectionState: ConnectionState = .offline

    /// chatID → (userID → last typing frame). Never persisted; entries
    /// older than 5 s are pruned so "X is typing…" can't get stuck.
    private(set) var typingByChat: [Int64: [Int64: Date]] = [:]

    /// The conversation currently on screen. While set, inbound messages
    /// for that chat advance the read marker instead of the unread badge.
    var activeChatID: Int64? {
        didSet {
            guard let chatID = activeChatID, oldValue != chatID else { return }
            markRead(chatID: chatID)
        }
    }

    // MARK: - Collaborators

    let api: APIClient
    private let socket: ChatSocket
    private let modelContext: ModelContext
    private(set) weak var session: AppSession?

    // MARK: - Internals

    private var socketTask: Task<Void, Never>?
    private var typingPruneTask: Task<Void, Never>?
    /// clientMsgID → the deliver call awaiting its ack.
    private var ackWaiters: [String: CheckedContinuation<Bool, Never>] = [:]
    /// localIDs with a deliver in flight, so the outbox sweep can't
    /// double-race a message that is already being delivered.
    private var deliveriesInFlight: Set<String> = []
    /// chatID → when we last sent a typing frame (client-side throttle).
    private var lastTypingSentAt: [Int64: Date] = [:]
    private var isResyncing = false

    /// How long the socket ack may take before REST wins the race.
    /// Internal + variable so the send-pipeline tests don't wait 10 s.
    var ackTimeout: TimeInterval = 10

    /// Pending rows older than this are re-sent by the outbox sweep.
    private let outboxAge: TimeInterval = 30

    /// Push-registration hook (PushRegistrar.ensureRegistered), injected
    /// at composition time and awaited at the end of every resync — see
    /// step 6 there. A closure so the coordinator stays UIKit-free; the
    /// no-op default keeps coordinator tests inert.
    var ensurePushRegistration: () async -> Void = {}

    /// Test seam: lets tests attribute "mine" without touching the app's
    /// real UserDefaults. The app never sets it.
    var currentUserIDOverride: Int64?
    private var currentUserID: Int64 { currentUserIDOverride ?? AppSettings.currentUserID ?? -1 }

    /// The coordinator owns its network collaborators; tests inject
    /// stub-session-backed instances through the same initializer.
    init(
        modelContainer: ModelContainer,
        api: APIClient? = nil,
        socket: ChatSocket? = nil
    ) {
        self.modelContext = modelContainer.mainContext
        self.api = api ?? APIClient(serverURL: AppSettings.serverURL)
        self.socket = socket ?? ChatSocket()
    }

    func bind(session: AppSession) {
        self.session = session
    }

    // MARK: - Lifecycle

    /// Enter the connected world: open the socket stream, start consuming
    /// it, and run a full resync. Called when AppSession reaches `.active`.
    func activate() async {
        if socketTask == nil {
            guard let serverURL = await api.currentServerURL,
                  let token = await api.token,
                  let wsURL = Self.webSocketURL(base: serverURL) else {
                AppLog.sync.error("activate() without server/token configured")
                return
            }
            connectionState = .connecting
            let stream = await socket.start(url: wsURL, token: token)
            socketTask = Task { [weak self] in
                for await event in stream {
                    guard let self else { break }
                    self.handle(event: event)
                }
            }
        }
        await resync()
    }

    /// Leave the connected world (logout, kicked, server change).
    func deactivate() {
        socketTask?.cancel()
        socketTask = nil
        typingPruneTask?.cancel()
        typingPruneTask = nil
        typingByChat = [:]
        lastTypingSentAt = [:]
        activeChatID = nil
        connectionState = .offline
        let socket = self.socket
        Task { await socket.stop() }
    }

    /// Scene went to background: drop the socket (iOS would kill it
    /// anyway); the stream and consumer task survive for `resume`.
    func enterBackground() {
        guard socketTask != nil else { return }
        connectionState = .offline
        let socket = self.socket
        Task { await socket.suspend() }
    }

    /// Scene came back: reconnect and resync (the wire missed everything
    /// while suspended — REST is the source of truth).
    func resumeForeground() {
        guard socketTask != nil else { return }
        connectionState = .connecting
        let socket = self.socket
        Task {
            await socket.resume()
            await self.resync()
        }
    }

    /// `{base}/api/v1/ws` with https→wss / http→ws scheme swap.
    static func webSocketURL(base: URL) -> URL? {
        var text = base.absoluteString
        if text.hasSuffix("/") { text.removeLast() }
        guard var components = URLComponents(string: text + "/api/v1/ws") else { return nil }
        components.scheme = (components.scheme == "https") ? "wss" : "ws"
        return components.url
    }

    // MARK: - Socket event handling

    func handle(event: SocketEvent) {
        switch event {
        case .connected:
            connectionState = .connected
            Task { await self.resync() }
        case .disconnected:
            // The socket's own loop is retrying; "offline" is reserved for
            // deliberate suspension.
            if connectionState == .connected { connectionState = .connecting }
        case .frame(let frame):
            handle(frame: frame)
        }
    }

    func handle(frame: ServerFrame) {
        switch frame {
        case .ack(let clientMsgID, let message):
            _ = upsert(message, bumpUnread: false)
            resolveAckWaiter(clientMsgID, delivered: true)

        case .message(let message):
            _ = upsert(message, bumpUnread: true)

        case .messageEdited(let message):
            // bumpUnread: false — an edit is not new mail. The body write
            // itself is guarded by edit_seq inside upsert, and the chat's
            // cursor advances so a later catch-up does not replay it.
            let entity = upsert(message, bumpUnread: false)
            if let seq = message.editSeq,
               let chat = fetchChat(message.chatID),
               seq > chat.maxEditSeq {
                chat.maxEditSeq = seq
            }
            // The preview follows only when this IS the newest message —
            // editing an old one must not reorder or relabel the chat list.
            if let chat = fetchChat(message.chatID),
               chat.lastMessageDate == entity.createdAt {
                chat.lastMessagePreview = message.body
            }
            saveContext()

        case .read(let chatID, let userID, let lastReadMessageID):
            guard let chat = fetchChat(chatID) else { return }
            if userID == currentUserID {
                // Our own read relayed back (or from another device).
                chat.myLastReadID = max(chat.myLastReadID, lastReadMessageID)
            } else {
                chat.othersReadUpTo = max(chat.othersReadUpTo, lastReadMessageID)
            }
            saveContext()

        case .typing(let chatID, let userID):
            guard userID != currentUserID else { return }
            typingByChat[chatID, default: [:]][userID] = Date()
            scheduleTypingPrune()

        case .memberJoined(let payload):
            upsertMember(
                userID: payload.user.id,
                username: payload.user.username,
                displayName: payload.user.displayName,
                role: "member",
                avatarVersion: payload.user.avatarVersion)

        case .reaction(let payload):
            // Full-state apply under the per-message seq guard; the chat
            // cursor MAX-advances regardless of whether the row is held,
            // so resync knows this seq needs no re-fetch (a dropped state
            // is re-delivered embedded on the Message by history paging).
            applyReactionState(
                messageServerID: payload.messageID,
                seq: payload.reactionSeq,
                reactions: payload.reactions)
            if let chat = fetchChat(payload.chatID), payload.reactionSeq > chat.maxReactionSeq {
                chat.maxReactionSeq = payload.reactionSeq
                saveContext()
            }

        case .memberLeft(let userID):
            if userID == currentUserID {
                // We were removed. Resync's /me reconcile routes the
                // session to needsFamily and purges family-scoped data.
                Task { await self.resync() }
            } else if let member = fetchMember(userID) {
                member.hasLeft = true
                saveContext()
            }

        case .pong:
            break // liveness handled inside ChatSocket

        case .error(let code, let message, let clientMsgID):
            AppLog.socket.error("Server error frame: \(code, privacy: .public) \(message, privacy: .public)")
            if let clientMsgID {
                if let row = fetchMessage(clientMsgID: clientMsgID), row.state != .sent {
                    row.state = .failed
                    saveContext()
                }
                resolveAckWaiter(clientMsgID, delivered: false)
            }

        case .unknown(let type):
            // Forward compatibility: never fatal, never noisy above debug.
            AppLog.socket.debug("Ignoring unknown frame type \(type, privacy: .public)")
        }
    }

    // MARK: - Message upsert (the dedup matrix)

    /// Write the body ONLY when the incoming copy is at least as new as
    /// the stored one.
    ///
    /// This is the guard the protocol calls load-bearing. Message
    /// deliveries are not ordered: a history page fetched BEFORE an edit
    /// can arrive after the `message_edited` frame that carries it, and an
    /// unguarded write would quietly restore the old text — on one device
    /// and not another, so the family disagrees about what was said.
    /// Absent seq counts as 0, which is exactly right: a message that was
    /// never edited carries no seq and can never lose to one that was.
    private func applyBody(_ dto: MessageDTO, to entity: MessageEntity) {
        let incoming = dto.editSeq ?? 0
        guard incoming >= entity.editSeq else { return }
        let bodyChanged = entity.body != dto.body
        entity.body = dto.body
        entity.editSeq = incoming
        entity.editedAt = dto.editedAt
        // A quote is a snapshot of the body, so every local reply pointing
        // at this message is now stale. The server recomputes it on its
        // next read; until then this keeps the two consistent on-screen.
        if bodyChanged, let serverID = entity.serverID {
            refreshQuotes(of: serverID, body: dto.body)
        }
    }

    /// Re-cut the excerpt on every locally-held reply that quotes this
    /// message, the same way the server would.
    private func refreshQuotes(of quotedID: Int64, body: String) {
        let descriptor = FetchDescriptor<MessageEntity>(
            predicate: #Predicate { $0.replyToMessageID == quotedID })
        guard let quoting = try? modelContext.fetch(descriptor), !quoting.isEmpty else { return }
        let excerpt = ReplyToSnapshot.excerpt(of: body)
        for row in quoting {
            row.replyExcerpt = excerpt
        }
    }

    /// The server recomputes the quote on every read, so the newest copy
    /// always wins — including when it is absent, which is the honest
    /// answer for a message that is not a reply. (Unlike reactions, where
    /// ABSENT means "no data" and must never wipe: a reply cannot stop
    /// being one, so there is no state to protect here.)
    private func applyReply(_ dto: MessageDTO, to entity: MessageEntity) {
        entity.replyToMessageID = dto.replyTo?.messageID
        entity.replySenderID = dto.replyTo?.senderID
        entity.replyExcerpt = dto.replyTo?.excerpt
    }

    /// Apply one server message. `bumpUnread` is true only for live
    /// `message` frames — resync trusts the server's unread_count from
    /// GET /chats instead of counting for itself.
    @discardableResult
    func upsert(_ dto: MessageDTO, bumpUnread: Bool) -> MessageEntity {
        let entity: MessageEntity
        if let clientMsgID = dto.clientMsgID,
           let existing = fetchMessage(clientMsgID: clientMsgID, chatID: dto.chatID) {
            // Case 1/2a: reconcile in place (ours pending, or re-delivery).
            existing.serverID = dto.id
            existing.createdAt = dto.createdAt
            applyBody(dto, to: existing)
            existing.senderID = dto.senderID
            existing.state = .sent
            entity = existing
            resolveAckWaiter(clientMsgID, delivered: true)
            applyReply(dto, to: existing)
        } else if let existing = fetchMessage(localID: "s:\(dto.id)") {
            // Case 2b: idempotent re-delivery of a server-keyed row.
            applyBody(dto, to: existing)
            existing.createdAt = dto.createdAt
            existing.senderID = dto.senderID
            existing.state = .sent
            entity = existing
            applyReply(dto, to: existing)
        } else {
            // Case 3: first sight — insert under the server key.
            entity = MessageEntity(
                localID: "s:\(dto.id)",
                serverID: dto.id,
                clientMsgID: dto.clientMsgID,
                chatID: dto.chatID,
                senderID: dto.senderID,
                body: dto.body,
                createdAt: dto.createdAt,
                status: .sent,
                replyToMessageID: dto.replyTo?.messageID,
                replySenderID: dto.replyTo?.senderID,
                replyExcerpt: dto.replyTo?.excerpt,
                editSeq: dto.editSeq ?? 0,
                editedAt: dto.editedAt)
            modelContext.insert(entity)
        }
        // Embedded reaction state (history/resync pages): the same seq
        // guard as live frames, so a page fetched before a live frame can
        // never clobber newer state — and ABSENT fields never wipe.
        if let seq = dto.reactionSeq, seq > entity.reactionSeq {
            entity.reactionSeq = seq
            entity.reactionList = Self.reactionSnapshots(dto.reactions ?? [])
        }
        updateChat(after: dto, bumpUnread: bumpUnread)
        saveContext()
        return entity
    }

    private func updateChat(after dto: MessageDTO, bumpUnread: Bool) {
        guard let chat = fetchChat(dto.chatID) else { return }
        if dto.id > chat.maxServerMessageID { chat.maxServerMessageID = dto.id }
        if chat.oldestLoadedMessageID == nil || dto.id < (chat.oldestLoadedMessageID ?? 0) {
            chat.oldestLoadedMessageID = dto.id
        }
        if chat.lastMessageDate == nil || dto.createdAt >= (chat.lastMessageDate ?? .distantPast) {
            chat.lastMessagePreview = dto.body
            chat.lastMessageDate = dto.createdAt
            chat.lastMessageSenderID = dto.senderID
        }
        if dto.senderID != currentUserID {
            if activeChatID == chat.chatID {
                // The user is looking at it: it is read, not unread.
                markRead(chatID: chat.chatID)
            } else if bumpUnread {
                chat.unreadCount += 1
            }
        }
    }

    // MARK: - Reactions

    /// [ReactionDTO] → the entity's stored value shape.
    private static func reactionSnapshots(_ reactions: [ReactionDTO]) -> [ReactionSnapshot] {
        reactions.map { ReactionSnapshot(userID: $0.userID, emoji: $0.emoji) }
    }

    /// THE seq-guarded apply path for one message's full reaction state —
    /// live `reaction` frames, catch-up pages and toggle responses all
    /// land here (embedded fields on a fetched Message get the same guard
    /// inside `upsert`). The state is full, never a delta, so applying is
    /// a plain rewrite; a seq at or below what the row holds is a stale
    /// re-delivery and a no-op. Returns false when the message isn't held
    /// locally — the state is dropped silently (history paging
    /// re-delivers it embedded on the Message objects).
    @discardableResult
    func applyReactionState(messageServerID: Int64, seq: Int64, reactions: [ReactionDTO]) -> Bool {
        guard let row = fetchMessage(serverID: messageServerID) else { return false }
        if seq > row.reactionSeq {
            row.reactionSeq = seq
            row.reactionList = Self.reactionSnapshots(reactions)
            saveContext()
        }
        return true
    }

    /// Toggle the current user's reaction from the UI: tapping the emoji
    /// they already set removes it (DELETE), anything else sets/replaces
    /// it (PUT — the server keeps one reaction per user per message).
    /// Optimistic: the local list is rewritten before the REST call, but
    /// `reactionSeq` is NOT touched — only the server mints seqs — so the
    /// authoritative response (seq greater) applies through the normal
    /// guarded path, and a no-op response (seq equal) changes nothing the
    /// optimistic write didn't already show. On a REST error the
    /// pre-toggle state is restored — unless a newer authoritative state
    /// landed meanwhile (the seq moved), which the revert must not
    /// clobber. No retry: reactions are cheap to re-tap, and APIClient
    /// deliberately auto-retries GETs only.
    func toggleReaction(localID: String, emoji: String) async {
        guard let row = fetchMessage(localID: localID),
              let serverID = row.serverID else { return }
        let chatID = row.chatID
        let userID = currentUserID
        let previousJSON = row.reactionsJSON
        let seqAtToggle = row.reactionSeq

        var list = row.reactionList
        let removing = list.first(where: { $0.userID == userID })?.emoji == emoji
        list.removeAll { $0.userID == userID }
        if !removing { list.append(ReactionSnapshot(userID: userID, emoji: emoji)) }
        row.reactionList = list
        saveContext()

        do {
            let state = removing
                ? try await api.removeReaction(chatID: chatID, messageID: serverID)
                : try await api.setReaction(chatID: chatID, messageID: serverID, emoji: emoji)
            applyReactionState(
                messageServerID: state.messageID,
                seq: state.reactionSeq,
                reactions: state.reactions)
        } catch APIError.unauthorized {
            session?.handleUnauthorized()
        } catch {
            AppLog.sync.info("Reaction toggle failed for \(localID, privacy: .public): \(String(describing: error))")
            if let current = fetchMessage(localID: localID), current.reactionSeq == seqAtToggle {
                current.reactionsJSON = previousJSON
                saveContext()
            }
        }
    }

    // MARK: - Send pipeline

    /// Optimistic send: the pending bubble exists before this returns.
    /// Returns the new row's localID (nil for an empty body).
    @discardableResult
    func send(body: String, in chatID: Int64, replyTo: ReplyToDTO? = nil) -> String? {
        guard let localID = enqueue(body: body, in: chatID, replyTo: replyTo) else { return nil }
        Task { await self.deliver(localID: localID) }
        return localID
    }

    /// Edit a message's body. Author-only server-side; the UI hides the
    /// action for anyone else, and this is the ack path for the author's
    /// own device — the other members hear it as `message_edited`.
    ///
    /// Deliberately NOT optimistic: an edit that the server refuses (too
    /// long, not the author, message gone) would otherwise leave the
    /// wrong text on screen with nothing to reconcile it, and unlike a
    /// send there is no pending row to mark failed.
    func edit(messageServerID: Int64, in chatID: Int64, body: String) async -> Bool {
        let trimmed = body.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return false }
        do {
            let dto = try await api.editMessage(chatID: chatID, messageID: messageServerID, body: trimmed)
            _ = upsert(dto, bumpUnread: false)
            if let seq = dto.editSeq, let chat = fetchChat(chatID), seq > chat.maxEditSeq {
                chat.maxEditSeq = seq
            }
            saveContext()
            return true
        } catch APIError.unauthorized {
            session?.handleUnauthorized()
            return false
        } catch {
            AppLog.sync.info("Edit failed for \(messageServerID, privacy: .public): \(String(describing: error))")
            return false
        }
    }

    /// Insert the pending row only (split from `send` so tests can drive
    /// `deliver` deterministically).
    func enqueue(body: String, in chatID: Int64, replyTo: ReplyToDTO? = nil) -> String? {
        let trimmed = body.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        let uuid = UUID().uuidString.lowercased()
        let now = Date()
        let entity = MessageEntity(
            localID: "c:\(uuid)",
            serverID: nil,
            clientMsgID: uuid,
            chatID: chatID,
            senderID: currentUserID,
            body: trimmed,
            createdAt: now,
            status: .pending,
            // Held on the pending row so the bubble shows its quote the
            // instant it appears, not once the server answers.
            replyToMessageID: replyTo?.messageID,
            replySenderID: replyTo?.senderID,
            replyExcerpt: replyTo?.excerpt)
        modelContext.insert(entity)
        if let chat = fetchChat(chatID) {
            chat.lastMessagePreview = trimmed
            chat.lastMessageDate = now
            chat.lastMessageSenderID = currentUserID
        }
        saveContext()
        return entity.localID
    }

    /// Deliver one pending row: socket send → ack race → REST fallback →
    /// failed. Safe to call again for the same row (idempotent server-side
    /// and guarded by `deliveriesInFlight` client-side).
    func deliver(localID: String) async {
        guard !deliveriesInFlight.contains(localID) else { return }
        deliveriesInFlight.insert(localID)
        defer { deliveriesInFlight.remove(localID) }

        guard let row = fetchMessage(localID: localID),
              row.state != .sent,
              let clientMsgID = row.clientMsgID else { return }
        let chatID = row.chatID
        let body = row.body
        let replyToMessageID = row.replyToMessageID
        row.state = .pending
        saveContext()

        // Leg 1: the socket, if it is up. A thrown notConnected skips the
        // ack race entirely and goes straight to REST.
        do {
            try await socket.send(.send(
                chatID: chatID,
                clientMsgID: clientMsgID,
                body: body,
                replyToMessageID: replyToMessageID))
            if await waitForAck(clientMsgID: clientMsgID, timeout: ackTimeout) { return }
        } catch {
            // fall through to REST
        }

        // A racing echo/resync may have confirmed the row meanwhile.
        if let current = fetchMessage(localID: localID), current.state == .sent { return }

        // Leg 2: REST with the SAME client_msg_id — the server dedups, so
        // this can never create a duplicate no matter what leg 1 did.
        do {
            let dto = try await api.sendMessage(
                chatID: chatID,
                clientMsgID: clientMsgID,
                body: body,
                replyToMessageID: replyToMessageID)
            _ = upsert(dto, bumpUnread: false)
        } catch APIError.unauthorized {
            session?.handleUnauthorized()
        } catch {
            AppLog.sync.info("Send failed for \(localID, privacy: .public): \(String(describing: error))")
            if let current = fetchMessage(localID: localID), current.state != .sent {
                current.state = .failed
                saveContext()
            }
        }
    }

    /// Tap-to-retry on a failed bubble: same row, same client_msg_id.
    func retry(localID: String) {
        guard let row = fetchMessage(localID: localID), row.state == .failed else { return }
        row.state = .pending
        saveContext()
        Task { await self.deliver(localID: localID) }
    }

    /// Delete a (typically failed) local message. Only rows without a
    /// serverID can truly disappear — the server has no delete endpoint.
    func deleteLocalMessage(localID: String) {
        guard let row = fetchMessage(localID: localID), row.serverID == nil else { return }
        modelContext.delete(row)
        saveContext()
    }

    // MARK: - Ack race plumbing

    private func waitForAck(clientMsgID: String, timeout: TimeInterval) async -> Bool {
        await withCheckedContinuation { continuation in
            ackWaiters[clientMsgID] = continuation
            Task { [weak self] in
                try? await Task.sleep(nanoseconds: UInt64(timeout * 1_000_000_000))
                self?.resolveAckWaiter(clientMsgID, delivered: false)
            }
        }
    }

    private func resolveAckWaiter(_ clientMsgID: String, delivered: Bool) {
        guard let continuation = ackWaiters.removeValue(forKey: clientMsgID) else { return }
        continuation.resume(returning: delivered)
    }

    // MARK: - Resync

    func resync() async {
        guard !isResyncing else { return }
        isResyncing = true
        defer { isResyncing = false }

        // 1. Membership reconcile. A missing family routes the session to
        // needsFamily (kicked) and there is nothing further to sync.
        do {
            let me = try await api.me()
            session?.apply(me: me)
            guard me.family != nil else { return }
        } catch APIError.unauthorized {
            session?.handleUnauthorized()
            return
        } catch {
            AppLog.sync.info("Resync /me failed: \(String(describing: error))")
            return
        }

        // 2. Roster, for sender-name resolution and the member picker.
        if let mine = try? await api.myFamily() {
            upsertMembers(mine.members)
        }

        // 3. Chat list: server unread wins; chats the server dropped go.
        guard let chatList = try? await api.chats() else { return }
        for item in chatList.chats { upsertChat(item) }
        dropChats(absentFrom: chatList.chats)
        saveContext()

        // 4. Per-chat catch-up: after_id loops until a short page.
        let cursors = chatList.chats.map {
            SyncPlan.ChatCursor(
                chatID: $0.chat.id,
                serverLatestMessageID: $0.lastMessage?.id,
                serverMaxReactionSeq: $0.maxReactionSeq ?? 0,
                serverMaxEditSeq: $0.maxEditSeq ?? 0)
        }
        for step in SyncPlan.make(chats: cursors, localCursors: localCursors()) {
            await runCatchUp(step)
        }

        // 5. Reaction catch-up, after the messages so freshly fetched
        // rows exist to receive their states: after_seq loops for every
        // chat whose server max_reaction_seq is ahead of our stored
        // cursor.
        for step in SyncPlan.makeReactionSteps(chats: cursors, localCursors: localReactionCursors()) {
            await runReactionCatchUp(step)
        }

        // 6. Edit catch-up, on its own cursor. `after_id` is WHERE id >
        // cursor and can never see a change to an OLDER row, so this is
        // the only way a client that was away learns a message it already
        // holds was rewritten.
        for step in SyncPlan.makeEditSteps(chats: cursors, localCursors: localEditCursors()) {
            await runEditCatchUp(step)
        }

        // 6. Outbox sweep: anything still pending after 30 s gets re-sent
        // (same client_msg_id — the server dedups).
        await sweepOutbox()

        // 7. Push registration: the first pass asks for notification
        // permission (we're .active, so the user is in a family and the
        // prompt has context); later passes re-POST a rotated token or
        // retry a registration that failed.
        await ensurePushRegistration()

        AppLog.sync.info("Resync complete")
    }

    private func runCatchUp(_ step: SyncPlan.FetchStep) async {
        let limit = 100
        var afterID = step.afterID
        while true {
            guard let page = try? await api.messages(chatID: step.chatID, afterID: afterID, limit: limit) else { return }
            for dto in page { _ = upsert(dto, bumpUnread: false) }
            if let last = page.last { afterID = max(afterID, last.id) }
            if page.count < limit { return }
        }
    }

    /// One reaction catch-up loop: after_seq pages until a short page,
    /// each state applied under the per-message seq guard. The chat's
    /// stored cursor advances to every page's max seq EVEN when entries
    /// referenced messages we don't hold (those states are dropped —
    /// history paging re-delivers them embedded on the Message objects);
    /// the cursor is what we've processed, never what we've kept.
    /// One edit catch-up loop: after_seq pages until a short page, each
    /// message applied through the ordinary upsert — so the edit_seq guard
    /// and the quote refresh come for free. The cursor advances to every
    /// page's max seq, whether or not we held the messages it named.
    private func runEditCatchUp(_ step: SyncPlan.ReactionFetchStep) async {
        let limit = 100
        var afterSeq = step.afterSeq
        while true {
            guard let page = try? await api.edits(chatID: step.chatID, afterSeq: afterSeq, limit: limit) else { return }
            for dto in page { _ = upsert(dto, bumpUnread: false) }
            if let last = page.last, let seq = last.editSeq { afterSeq = max(afterSeq, seq) }
            if let chat = fetchChat(step.chatID), afterSeq > chat.maxEditSeq {
                chat.maxEditSeq = afterSeq
            }
            saveContext()
            if page.count < limit { return }
        }
    }

    private func runReactionCatchUp(_ step: SyncPlan.ReactionFetchStep) async {
        let limit = 100
        var afterSeq = step.afterSeq
        while true {
            guard let page = try? await api.reactions(chatID: step.chatID, afterSeq: afterSeq, limit: limit) else { return }
            for state in page {
                applyReactionState(
                    messageServerID: state.messageID,
                    seq: state.reactionSeq,
                    reactions: state.reactions)
            }
            if let last = page.last { afterSeq = max(afterSeq, last.reactionSeq) }
            if let chat = fetchChat(step.chatID), afterSeq > chat.maxReactionSeq {
                chat.maxReactionSeq = afterSeq
                saveContext()
            }
            if page.count < limit { return }
        }
    }

    private func sweepOutbox() async {
        let cutoff = Date().addingTimeInterval(-outboxAge)
        let pendingRaw = MessageStatus.pending.rawValue
        let descriptor = FetchDescriptor<MessageEntity>(
            predicate: #Predicate { $0.status == pendingRaw && $0.createdAt < cutoff })
        guard let stale = try? modelContext.fetch(descriptor), !stale.isEmpty else { return }
        AppLog.sync.info("Outbox sweep re-sending \(stale.count, privacy: .public) message(s)")
        for row in stale {
            await deliver(localID: row.localID)
        }
    }

    // MARK: - History pagination (scroll-up)

    /// Load one older page for a chat. Returns the number of messages
    /// fetched; flips `hasFullHistory` on a short page.
    func loadOlder(chatID: Int64, limit: Int = 50) async -> Int {
        guard let chat = fetchChat(chatID), !chat.hasFullHistory else { return 0 }
        let beforeID = chat.oldestLoadedMessageID
        do {
            // No cursor yet (fresh chat): fetch the newest page instead.
            let page = try await api.messages(chatID: chatID, beforeID: beforeID, limit: limit)
            for dto in page { _ = upsert(dto, bumpUnread: false) }
            if page.count < limit {
                chat.hasFullHistory = true
                saveContext()
            }
            return page.count
        } catch {
            AppLog.sync.info("loadOlder(\(chatID)) failed: \(String(describing: error))")
            return 0
        }
    }

    // MARK: - Read markers

    /// Report everything currently held for `chatID` as read. Throttled by
    /// monotonicity: nothing is sent unless the marker would advance.
    func markRead(chatID: Int64) {
        guard let chat = fetchChat(chatID) else { return }
        chat.unreadCount = 0
        let target = chat.maxServerMessageID
        guard target > chat.myLastReadID else {
            saveContext()
            return
        }
        chat.myLastReadID = target
        saveContext()
        let socket = self.socket
        let api = self.api
        Task {
            do {
                try await socket.send(.read(chatID: chatID, lastReadMessageID: target))
            } catch {
                try? await api.markRead(chatID: chatID, lastReadMessageID: target)
            }
        }
    }

    // MARK: - Typing

    /// Called by the conversation view on every keystroke; throttled here
    /// to one frame per chat per 4 s (the server throttles at 3 s — ours
    /// is deliberately coarser so we never hit the server's limiter).
    func sendTyping(in chatID: Int64) {
        let now = Date()
        if let last = lastTypingSentAt[chatID], now.timeIntervalSince(last) < 4 { return }
        lastTypingSentAt[chatID] = now
        let socket = self.socket
        Task { try? await socket.send(.typing(chatID: chatID)) }
    }

    /// userIDs typing in a chat right now (frames younger than 5 s).
    func typingUserIDs(in chatID: Int64) -> [Int64] {
        let cutoff = Date().addingTimeInterval(-5)
        return (typingByChat[chatID] ?? [:])
            .filter { $0.value > cutoff }
            .keys.sorted()
    }

    private func scheduleTypingPrune() {
        typingPruneTask?.cancel()
        typingPruneTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 5_000_000_000)
            guard !Task.isCancelled else { return }
            self?.pruneTyping()
        }
    }

    private func pruneTyping() {
        let cutoff = Date().addingTimeInterval(-5)
        for (chatID, users) in typingByChat {
            let live = users.filter { $0.value > cutoff }
            typingByChat[chatID] = live.isEmpty ? nil : live
        }
        if !typingByChat.isEmpty { scheduleTypingPrune() }
    }

    // MARK: - Chats & members upserts

    /// Get-or-create a direct chat and mirror it locally; returns the
    /// chatID for navigation.
    func openDirectChat(with userID: Int64) async throws -> Int64 {
        let dto = try await api.createDirectChat(userID: userID)
        upsertChat(ChatListItemDTO(chat: dto, lastMessage: nil, unreadCount: 0), resetWhenNoLastMessage: false)
        saveContext()
        return dto.id
    }

    private func upsertChat(_ item: ChatListItemDTO, resetWhenNoLastMessage: Bool = true) {
        let dto = item.chat
        let chat: ChatEntity
        if let existing = fetchChat(dto.id) {
            chat = existing
        } else {
            chat = ChatEntity(
                chatID: dto.id,
                kind: dto.kind,
                pinRank: dto.kind == "family" ? 0 : 1,
                title: dto.title)
            modelContext.insert(chat)
        }
        chat.kind = dto.kind
        chat.pinRank = dto.kind == "family" ? 0 : 1
        chat.peerUserID = dto.peerUserID
        chat.title = dto.title
        chat.unreadCount = item.unreadCount // server-authoritative
        if let last = item.lastMessage {
            chat.lastMessagePreview = last.body
            chat.lastMessageDate = last.createdAt
            chat.lastMessageSenderID = last.senderID
        } else if resetWhenNoLastMessage {
            chat.lastMessagePreview = nil
            chat.lastMessageDate = nil
            chat.lastMessageSenderID = nil
            // An empty chat has no history to page for.
            chat.hasFullHistory = true
        }
    }

    /// Delete local chats (and their messages) the server no longer
    /// lists — e.g. a direct-chat peer left the family.
    private func dropChats(absentFrom items: [ChatListItemDTO]) {
        let serverIDs = Set(items.map { $0.chat.id })
        guard let locals = try? modelContext.fetch(FetchDescriptor<ChatEntity>()) else { return }
        for chat in locals where !serverIDs.contains(chat.chatID) {
            let chatID = chat.chatID
            let messages = (try? modelContext.fetch(
                FetchDescriptor<MessageEntity>(predicate: #Predicate { $0.chatID == chatID }))) ?? []
            for message in messages { modelContext.delete(message) }
            modelContext.delete(chat)
            if activeChatID == chatID { activeChatID = nil }
        }
    }

    private func upsertMembers(_ members: [MemberDTO]) {
        let present = Set(members.map(\.id))
        for member in members {
            upsertMember(
                userID: member.id,
                username: member.username,
                displayName: member.displayName,
                role: member.role,
                avatarVersion: member.avatarVersion)
        }
        // Anyone we know locally but the roster omits has left; keep the
        // row (name resolution on old bubbles) but flag it.
        if let locals = try? modelContext.fetch(FetchDescriptor<MemberEntity>()) {
            for local in locals where !present.contains(local.userID) {
                local.hasLeft = true
            }
        }
        saveContext()
    }

    private func upsertMember(
        userID: Int64,
        username: String,
        displayName: String,
        role: String,
        avatarVersion: Int64
    ) {
        if let existing = fetchMember(userID) {
            existing.username = username
            existing.displayName = displayName
            existing.role = role
            existing.isCurrentUser = userID == currentUserID
            existing.hasLeft = false
            existing.avatarVersion = avatarVersion
        } else {
            modelContext.insert(MemberEntity(
                userID: userID,
                username: username,
                displayName: displayName,
                role: role,
                isCurrentUser: userID == currentUserID,
                avatarVersion: avatarVersion))
        }
        saveContext()
    }

    // MARK: - Fetch helpers

    private func fetchChat(_ chatID: Int64) -> ChatEntity? {
        var descriptor = FetchDescriptor<ChatEntity>(predicate: #Predicate { $0.chatID == chatID })
        descriptor.fetchLimit = 1
        return (try? modelContext.fetch(descriptor))?.first
    }

    private func fetchMember(_ userID: Int64) -> MemberEntity? {
        var descriptor = FetchDescriptor<MemberEntity>(predicate: #Predicate { $0.userID == userID })
        descriptor.fetchLimit = 1
        return (try? modelContext.fetch(descriptor))?.first
    }

    func fetchMessage(localID: String) -> MessageEntity? {
        var descriptor = FetchDescriptor<MessageEntity>(predicate: #Predicate { $0.localID == localID })
        descriptor.fetchLimit = 1
        return (try? modelContext.fetch(descriptor))?.first
    }

    /// By server id, regardless of localID prefix: a reaction can target
    /// a message we sent (a "c:" row that has since gained its serverID)
    /// just as well as one first seen from the server ("s:" row).
    private func fetchMessage(serverID: Int64) -> MessageEntity? {
        let target: Int64? = serverID
        var descriptor = FetchDescriptor<MessageEntity>(predicate: #Predicate { $0.serverID == target })
        descriptor.fetchLimit = 1
        return (try? modelContext.fetch(descriptor))?.first
    }

    private func fetchMessage(clientMsgID: String, chatID: Int64) -> MessageEntity? {
        let target: String? = clientMsgID
        var descriptor = FetchDescriptor<MessageEntity>(
            predicate: #Predicate { $0.clientMsgID == target && $0.chatID == chatID })
        descriptor.fetchLimit = 1
        return (try? modelContext.fetch(descriptor))?.first
    }

    private func fetchMessage(clientMsgID: String) -> MessageEntity? {
        let target: String? = clientMsgID
        var descriptor = FetchDescriptor<MessageEntity>(
            predicate: #Predicate { $0.clientMsgID == target })
        descriptor.fetchLimit = 1
        return (try? modelContext.fetch(descriptor))?.first
    }

    private func localCursors() -> [Int64: Int64] {
        guard let chats = try? modelContext.fetch(FetchDescriptor<ChatEntity>()) else { return [:] }
        return Dictionary(uniqueKeysWithValues: chats.map { ($0.chatID, $0.maxServerMessageID) })
    }

    private func localReactionCursors() -> [Int64: Int64] {
        guard let chats = try? modelContext.fetch(FetchDescriptor<ChatEntity>()) else { return [:] }
        return Dictionary(uniqueKeysWithValues: chats.map { ($0.chatID, $0.maxReactionSeq) })
    }

    /// chatID → stored maxEditSeq, for planning the edit catch-up.
    private func localEditCursors() -> [Int64: Int64] {
        let chats = (try? modelContext.fetch(FetchDescriptor<ChatEntity>())) ?? []
        return Dictionary(uniqueKeysWithValues: chats.map { ($0.chatID, $0.maxEditSeq) })
    }

    private func saveContext() {
        do {
            try modelContext.save()
        } catch {
            AppLog.sync.error("ModelContext save failed: \(String(describing: error))")
        }
    }
}
