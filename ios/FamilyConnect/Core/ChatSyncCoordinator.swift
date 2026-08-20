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
//    5. outbox sweep      — pending rows older than 30 s re-delivered.
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
                role: "member")

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
            existing.body = dto.body
            existing.senderID = dto.senderID
            existing.state = .sent
            entity = existing
            resolveAckWaiter(clientMsgID, delivered: true)
        } else if let existing = fetchMessage(localID: "s:\(dto.id)") {
            // Case 2b: idempotent re-delivery of a server-keyed row.
            existing.body = dto.body
            existing.createdAt = dto.createdAt
            existing.senderID = dto.senderID
            existing.state = .sent
            entity = existing
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
                status: .sent)
            modelContext.insert(entity)
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

    // MARK: - Send pipeline

    /// Optimistic send: the pending bubble exists before this returns.
    /// Returns the new row's localID (nil for an empty body).
    @discardableResult
    func send(body: String, in chatID: Int64) -> String? {
        guard let localID = enqueue(body: body, in: chatID) else { return nil }
        Task { await self.deliver(localID: localID) }
        return localID
    }

    /// Insert the pending row only (split from `send` so tests can drive
    /// `deliver` deterministically).
    func enqueue(body: String, in chatID: Int64) -> String? {
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
            status: .pending)
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
        row.state = .pending
        saveContext()

        // Leg 1: the socket, if it is up. A thrown notConnected skips the
        // ack race entirely and goes straight to REST.
        do {
            try await socket.send(.send(chatID: chatID, clientMsgID: clientMsgID, body: body))
            if await waitForAck(clientMsgID: clientMsgID, timeout: ackTimeout) { return }
        } catch {
            // fall through to REST
        }

        // A racing echo/resync may have confirmed the row meanwhile.
        if let current = fetchMessage(localID: localID), current.state == .sent { return }

        // Leg 2: REST with the SAME client_msg_id — the server dedups, so
        // this can never create a duplicate no matter what leg 1 did.
        do {
            let dto = try await api.sendMessage(chatID: chatID, clientMsgID: clientMsgID, body: body)
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
            SyncPlan.ChatCursor(chatID: $0.chat.id, serverLatestMessageID: $0.lastMessage?.id)
        }
        for step in SyncPlan.make(chats: cursors, localCursors: localCursors()) {
            await runCatchUp(step)
        }

        // 5. Outbox sweep: anything still pending after 30 s gets re-sent
        // (same client_msg_id — the server dedups).
        await sweepOutbox()

        // 6. Push registration: the first pass asks for notification
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
                role: member.role)
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

    private func upsertMember(userID: Int64, username: String, displayName: String, role: String) {
        if let existing = fetchMember(userID) {
            existing.username = username
            existing.displayName = displayName
            existing.role = role
            existing.isCurrentUser = userID == currentUserID
            existing.hasLeft = false
        } else {
            modelContext.insert(MemberEntity(
                userID: userID,
                username: username,
                displayName: displayName,
                role: role,
                isCurrentUser: userID == currentUserID))
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

    private func saveContext() {
        do {
            try modelContext.save()
        } catch {
            AppLog.sync.error("ModelContext save failed: \(String(describing: error))")
        }
    }
}
