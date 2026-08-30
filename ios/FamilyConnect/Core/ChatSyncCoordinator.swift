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
//    3. GET /chats        — chat upsert; server unread_count wins; the
//                           caller's own last_read_message_id is applied
//                           monotonically and any chat whose marker MOVED
//                           has its stale banners taken down (it was read
//                           on another of this person's devices); local
//                           DIRECT chats the response does not list are
//                           dropped with their messages (the general
//                           repair for an account deletion this device
//                           was offline for),
//    4. SyncPlan after_id  — per-chat catch-up loops (limit 100) until a
//                           short page,
//    5. SyncPlan after_seq — per-chat reaction, edit and poll catch-up
//                           loops, each where the server's max_*_seq is
//                           ahead of the cursor we hold for it,
//    6. outbox sweep      — pending rows older than 30 s re-delivered.
//
//  UNREAD: a chat is read when — and only when — someone is looking at
//  its newest message with the app in front of them. That is ChatPresence,
//  published by the conversation views and consulted here in exactly two
//  places: the live-message path (which suppresses the bump instead of
//  bumping and clearing, so the icon badge does not flicker) and
//  `markRead`. Nothing else in this file may mark anything read — see
//  ChatPresence for what a mistaken read costs, which is the badge on
//  every device that person owns, forever.
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

    /// Who is looking at what, right now — see ChatPresence, which is the
    /// only definition of that in this app. Published by whichever
    /// conversation view is on screen; read by the unread rule in
    /// `updateChat` and by the Mac's local notification in `announce`,
    /// which must never disagree about it.
    ///
    /// This replaced a plain `activeChatID` whose `didSet` marked the chat
    /// read, i.e. a chat was read by APPEARING. Mounting the view on a Mac
    /// that launched behind every other window read the family chat;
    /// clicking the Dock icon to see what had arrived destroyed the count
    /// in the same gesture; and because the claim survived backgrounding on
    /// iOS, the next resync walked every missed message through the read
    /// path and threw away the count it had just fetched from the server.
    private(set) var presence: ChatPresence?

    /// Publish what the person in front of `chatID` can currently see, and
    /// read the chat when all of it is true.
    ///
    /// The view calls this from every hook that can change one of the three
    /// facts — appearing, the bottom sentinel's geometry, the opening
    /// settle, new messages, and the scene or window becoming (un)frontmost
    /// — rather than deciding for itself what "read" means. Unconditional
    /// on purpose: `markRead` is a cheap no-op when there is nothing to
    /// read, and re-asking on an unchanged presence is what catches the
    /// case where the thread scrolled to a new message without the sentinel
    /// ever leaving the viewport.
    func updatePresence(chatID: Int64, isAtNewest: Bool, isFrontmost: Bool) {
        presence = ChatPresence(chatID: chatID, isAtNewest: isAtNewest, isFrontmost: isFrontmost)
        if isAtNewest && isFrontmost { markRead(chatID: chatID) }
    }

    /// Give up the claim — but only if it is still ours, because on the Mac
    /// another window may have taken it since. Without that guard, closing
    /// one window clears the claim of another that is still on screen, and
    /// every message arriving in it starts bumping the badge of a chat the
    /// user is looking at.
    func releasePresence(chatID: Int64) {
        if presence?.chatID == chatID { presence = nil }
    }

    /// Is the user genuinely reading this chat this instant? The one
    /// question, asked in the one way.
    func isReading(_ chatID: Int64) -> Bool {
        presence?.isReading(chatID) ?? false
    }

    // MARK: - Collaborators

    let api: APIClient
    private let socket: ChatSocket
    private let modelContext: ModelContext
    private(set) weak var session: AppSession?
    private weak var callManager: CallManager?
    /// Where the scene is, as `enterBackground`/`resumeForeground` last
    /// said — consulted when a call ends to decide whether the socket it
    /// kept alive should now go.
    private var isInBackground = false
    private weak var attachmentStore: AttachmentStore?

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

    /// chatID → the ids of the messages a live `message` frame has bumped
    /// its unread count for, and which no `GET /chats` response has been
    /// shown to have counted yet.
    ///
    /// IDS RATHER THAN A COUNT, and that is the whole correction. The
    /// repair in resync step 3 used to be a counter difference across the
    /// await, which assumes the server's `unread_count` always predates a
    /// message delivered mid-flight. It does not: the server commits a
    /// message and THEN broadcasts it, and the chat-list query is served
    /// concurrently — so a message can be in the count AND arrive as a
    /// frame before the response lands, and adding a blind delta counted it
    /// twice. A badge of 2 for one unread message, until something else
    /// refreshed it. An id can be checked against what the response says
    /// the server had seen; a number cannot.
    ///
    /// See `uncountedBumps`, which is also where they are forgotten.
    private var liveBumpedMessageIDs: [Int64: Set<Int64>] = [:]

    /// Chats with a read report on the wire. The local marker no longer
    /// advances before the post succeeds (see `markRead`), so this is what
    /// stops a reader who scrolls while the network is slow from stacking a
    /// report per scroll event.
    private var readPostsInFlight: Set<Int64> = []

    /// chatID → the highest read target that became due while that chat's
    /// report was still on the wire.
    ///
    /// Coalescing is the point of the in-flight set; DROPPING is not, and
    /// dropping is what it did. Two messages landing 200 ms apart in a chat
    /// somebody is reading produce two `markRead` calls, and the second one
    /// used to return and be forgotten: nothing re-enters `markRead` on its
    /// own afterwards, because the sentinel's geometry did not change, the
    /// scene did not change, and no further message arrived. The server's
    /// marker then stayed one message behind a reader who had seen it — a
    /// stale read receipt for the sender, and a badge the next `GET /chats`
    /// re-inflated onto the conversation they were looking at.
    private var pendingReadTargets: [Int64: Int64] = [:]

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

    /// Test seam: `connectionState` is `private(set)` and every real path to
    /// it runs through the socket, which unit tests do not start. The app
    /// never calls this.
    func overrideConnectionState(_ state: ConnectionState) {
        connectionState = state
    }
    /// Who "mine" means. Internal rather than private: the Mac's message
    /// row decides which side a balloon sits on, and asking the session
    /// for it there would be a second source of the same truth.
    var currentUserID: Int64 { currentUserIDOverride ?? AppSettings.currentUserID ?? -1 }

    /// The delivery started by the most recent `send`/`sendMedia`.
    ///
    /// Test seam, and a sharp one: those two return as soon as the row is
    /// enqueued, leaving delivery running in a detached Task. A test whose
    /// ModelContainer goes out of scope while that Task is still touching
    /// the context does not fail — SwiftData TRAPS, taking the whole test
    /// process with it. Awaiting this is how a test stays alive until the
    /// row has settled. Nothing in the app reads it.
    private(set) var pendingDelivery: Task<Void, Never>?

    /// The read markers the most recent resync TO REACH THE CHAT LIST
    /// found further along than this device had recorded — the chats
    /// somebody read on another of this person's devices, and so exactly
    /// the chats whose delivered banners it took down. A resync that fell
    /// over before GET /chats leaves the previous answer standing, because
    /// it learned nothing to replace it with.
    ///
    /// Test seam, and the only way to assert that TRIGGER: what it drives
    /// is UNUserNotificationCenter, which a unit test has nothing to look
    /// at and no way to put anything into. Nothing in the app reads it.
    private(set) var lastResyncReadElsewhere: [Int64: Int64] = [:]

    /// The read report started by the most recent `markRead`.
    ///
    /// Test seam, the twin of `pendingDelivery` and sharp for the same
    /// reason: the local marker now advances INSIDE that task, once the
    /// server has actually taken the read, so a test asserting on
    /// `myLastReadID` without awaiting this is asserting on a value that
    /// has not been written yet. Nothing in the app reads it.
    private(set) var pendingReadPost: Task<Void, Never>?

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

    /// The call state machine, so signalling frames reach it and so the
    /// socket knows to stay up for the life of a call. Weak: the manager
    /// is owned by the composition root, like the session.
    func bind(callManager: CallManager) {
        self.callManager = callManager
    }

    /// True while a call is ringing, being rung, or talking — the one
    /// thing that keeps the socket open in the background.
    var isCallInProgress: Bool { callManager.map { !$0.isIdle } ?? false }

    /// Bring the socket up from wherever it is: never started (a launch
    /// straight into the background — a VoIP push woke the process, and
    /// no scene has asked for the connected world yet), suspended (the app
    /// was backgrounded), or already live. The replayed `call_offer` only
    /// arrives on a registered socket (docs/protocol.md, "Late arrivals"),
    /// so the incoming-call path calls this before anything else.
    func ensureConnected() async {
        if socketTask == nil {
            await activate()
        } else {
            resumeForeground()
        }
    }

    /// The call ended: if the scene is in the background, do now what
    /// `enterBackground` deliberately did not — suspend the socket.
    func callDidEnd() {
        guard isInBackground, socketTask != nil else { return }
        connectionState = .offline
        let socket = self.socket
        Task { await socket.suspend() }
    }

    /// The attachment cache, so a sent photo can be drawn from the bytes
    /// this device already made rather than fetched back. Weak: the store
    /// is built from `api`, which this object owns.
    func bind(attachmentStore: AttachmentStore) {
        self.attachmentStore = attachmentStore
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
        presence = nil
        connectionState = .offline
        let socket = self.socket
        Task { await socket.stop() }
    }

    /// Scene went to background: drop the socket (iOS would kill it
    /// anyway); the stream and consumer task survive for `resume`.
    func enterBackground() {
        // Backgrounding revokes the AUTHORITY to read without disturbing
        // the view's claim on the chat — the view is still the owner, it
        // just cannot see anything from here. It has to happen centrally,
        // and before the socket guard: `ConversationView.onDisappear` does
        // NOT fire when the app goes to the background, so the claim used
        // to survive with `isFrontmost` intact and the resync that follows
        // the next foreground read everything it had just downloaded.
        if let presence {
            self.presence = ChatPresence(
                chatID: presence.chatID, isAtNewest: presence.isAtNewest, isFrontmost: false)
        }
        isInBackground = true
        guard socketTask != nil else { return }
        // A call holds the socket open: its `call_end`, its candidates and
        // an ICE restart all arrive over it, and the audio session (or the
        // VoIP background mode) is what lets it stay up. `callDidEnd`
        // suspends it afterwards if the scene is still in the background.
        guard SocketHold.decide(isInBackground: true, isCallInProgress: isCallInProgress) == .suspend else {
            return
        }
        connectionState = .offline
        let socket = self.socket
        Task { await socket.suspend() }
    }

    /// Scene came back: reconnect and resync (the wire missed everything
    /// while suspended — REST is the source of truth).
    func resumeForeground() {
        isInBackground = false
        guard socketTask != nil else { return }
        let socket = self.socket
        Task {
            // Ask the socket what actually happened rather than assuming.
            // This used to set `.connecting` unconditionally — but `resume()`
            // is a no-op on a socket that was never suspended (coming back
            // from a brief interruption that never tore it down), and nothing
            // else re-emits `.connected`. The banner then read "Connecting…"
            // indefinitely while every message went through perfectly.
            switch await socket.resume() {
            case .alreadyLive:
                self.connectionState = .connected
            case .reconnecting:
                self.connectionState = .connecting
            case .notStarted:
                break
            }
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
        case .unauthorized:
            // The token is dead (deleted account, revoked session): the
            // same 401 handling every REST call gets, so this device
            // returns to the sign-in screen instead of reconnecting
            // forever against a session that no longer exists.
            AppLog.sync.info("Socket reports the session is gone; signing out")
            session?.handleUnauthorized()
        case .frame(let frame):
            // A frame in hand is proof the connection is live, so this is
            // what makes a stuck banner self-heal whatever caused it.
            // Deliberately NOT lifting `.offline`: that means the app
            // suspended the socket on purpose, and a frame decoded before
            // teardown can still be sitting in the (unbounded) stream buffer.
            if connectionState == .connecting { connectionState = .connected }
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
            announce(message)

        case .boardNote(let note):
            applyNote(note)
            saveContext()

        case .aiDelta(_, let messageID, let text):
            appendAssistantDelta(messageID: messageID, text: text)

        case .aiError(_, let messageID):
            // Whatever arrived is already on the row and stays there — a
            // partial answer is worth more than a bubble that never
            // resolves. Just stop showing it as still being written.
            streamingMessageIDs.remove(messageID)

        case .messageEdited(let message):
            // The authoritative body: whatever was accumulated from deltas
            // is replaced, and the row stops being "still being written".
            streamingMessageIDs.remove(message.id)
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
                chat.lastMessagePreview = Self.preview(
                    body: message.body, attachments: message.attachmentList,
                    call: message.call, isMine: message.senderID == currentUserID)
            }
            saveContext()

        case .read(let chatID, let userID, let lastReadMessageID):
            guard let chat = fetchChat(chatID) else { return }
            if userID == currentUserID {
                // Our own read relayed back (or from another device).
                chat.myLastReadID = max(chat.myLastReadID, lastReadMessageID)
            } else {
                // Stored in EVERY chat kind, drawn in only one. In a family
                // chat this is roster data that no bubble may draw
                // (docs/protocol.md, "Frames") — `MessagePresentation.isRead`
                // is the gate, and it takes `isFamilyChat` undefaulted so a
                // new surface cannot start drawing it by accident. Keeping
                // the value is deliberate: it is a real fact about the
                // roster, and dropping it here would only move the question.
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

        case .poll(let payload):
            // The reaction case's twin, and deliberately identical: full
            // state under the per-message seq guard, and the chat cursor
            // MAX-advances whether or not the row is held, so a resync
            // does not re-fetch a seq this client has already seen. A
            // dropped state comes back embedded on the Message when
            // history pages there.
            applyPollState(messageServerID: payload.messageID, poll: payload.poll)
            advancePollCursor(chatID: payload.chatID, seq: payload.poll.pollSeq)

        case .memberDeleted(let payload):
            // The one frame whose job is to WIPE stored fields, so it is
            // applied deliberately and NOT through upsertMember — that
            // path exists to make sure an absent field never clears a
            // stored one, which is the opposite of what is wanted here.
            applyMemberTombstone(payload.member)
            // Their direct chat with us went with the account, both halves
            // (protocol.md, "Deleting an account"), so it goes here too —
            // now, rather than at the next resync. Left standing it is a
            // row in the list under the peer's OLD name that answers 404
            // to everything, including the message somebody types into it.
            // The tombstone is written FIRST and survives this: their
            // messages are still in the family chat and still need a name.
            dropDirectChat(peerUserID: payload.member.id)

        case .familyOwner(let familyID, let userID):
            // An owner deleted their account and ownership passed on. The
            // roster row moves first, then the session — a client that has
            // just become the owner gains the owner-only screens now
            // rather than at its next GET /me.
            applyFamilyOwner(familyID: familyID, userID: userID)

        case .memberBlocked(let userID, let blocked):
            // A state-set, not an event: an unblock is this same frame with
            // `false`. It reaches this device and this account only — never
            // the person blocked — so there is nothing to fan out and
            // nothing to reconcile against a roster.
            applyBlock(userID: userID, blocked: blocked)

        case .memberLeft(let userID):
            if userID == currentUserID {
                // We were removed. Resync's /me reconcile routes the
                // session to needsFamily and purges family-scoped data.
                Task { await self.resync() }
            } else if let member = fetchMember(userID) {
                member.hasLeft = true
                saveContext()
            }

        case .callOffer, .callRinging, .callAnswer, .callIce, .callEnd:
            // Signalling never touches the store: the manager applies it to
            // the one call it holds, or ignores it (docs/protocol.md,
            // "Voice calls"). The record of the call arrives later as an
            // ordinary `message`.
            callManager?.handle(frame: frame)

        case .pong:
            break // liveness handled inside ChatSocket

        case .error(let code, let message, let clientMsgID, let callID):
            AppLog.socket.error("Server error frame: \(code, privacy: .public) \(message, privacy: .public)")
            if callID != nil {
                callManager?.handle(frame: frame)
            }
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

    // MARK: - Board

    /// Assistant replies still being written, by server id.
    ///
    /// Read by the bubble to show a cursor while text is arriving. Cleared
    /// when the final `message_edited` lands (or an `ai_error` does), so a
    /// row that was mid-stream when the app was killed is not stuck looking
    /// live forever — nothing here survives a launch.
    private(set) var streamingMessageIDs: Set<Int64> = []

    /// Append one fragment to the assistant's row.
    ///
    /// Deltas carry no `edit_seq`, so this never fights the edit guard: the
    /// final body arrives as an edit with a real seq and overwrites
    /// whatever was accumulated, which is also how a client that missed
    /// every delta ends up correct.
    private func appendAssistantDelta(messageID: Int64, text: String) {
        guard let row = fetchMessage(serverID: messageID) else { return }
        row.body += text
        streamingMessageIDs.insert(messageID)
        saveContext()
    }

    /// The board catch-up cursor, persisted so a relaunch resumes rather
    /// than re-reading the whole wall.
    var boardCursor: Int64 {
        get { AppSettings.boardCursor }
        set { AppSettings.boardCursor = newValue }
    }

    /// Apply one note under the per-note seq guard.
    ///
    /// A TOMBSTONE deletes the local row: the server keeps one so its feed
    /// can say "gone", but a client that has been told has nothing left to
    /// remember. The guard still applies — an out-of-order tombstone must
    /// not remove a note that has since been re-created… which cannot
    /// happen (ids are never reused), but the same rule covers an
    /// out-of-order MOVE, which very much can.
    @discardableResult
    func applyNote(_ dto: NoteDTO) -> Bool {
        let existing = fetchNote(dto.id)
        if let existing, dto.boardSeq <= existing.boardSeq { return false }

        if dto.isTombstone {
            if let existing { modelContext.delete(existing) }
            return true
        }
        guard let authorID = dto.authorID,
              let text = dto.text,
              let color = dto.color,
              let x = dto.x,
              let y = dto.y
        else {
            // A live note missing content is a server bug; dropping it
            // beats drawing a blank sticker.
            return false
        }
        // A server from before sizes never sends one; "medium" is the size
        // every note had then, so the wall does not change under it.
        let size = dto.size ?? NoteSize.medium.name
        if let existing {
            existing.authorID = authorID
            existing.text = text
            existing.color = color
            existing.size = size
            existing.x = x
            existing.y = y
            existing.updatedAt = dto.updatedAt ?? existing.updatedAt
            existing.boardSeq = dto.boardSeq
        } else {
            modelContext.insert(NoteEntity(
                noteID: dto.id,
                authorID: authorID,
                text: text,
                color: color,
                size: size,
                x: x,
                y: y,
                createdAt: dto.createdAt ?? Date(),
                updatedAt: dto.updatedAt ?? Date(),
                boardSeq: dto.boardSeq))
        }
        return true
    }

    private func fetchNote(_ noteID: Int64) -> NoteEntity? {
        let descriptor = FetchDescriptor<NoteEntity>(predicate: #Predicate { $0.noteID == noteID })
        return (try? modelContext.fetch(descriptor))?.first
    }

    /// Full board read — used the first time a board is opened, and
    /// whenever the local cursor is 0 (nothing applied yet).
    func loadBoard() async {
        guard let response = try? await api.board() else { return }
        for note in response.notes { applyNote(note) }
        boardCursor = max(boardCursor, response.maxBoardSeq)
        saveContext()
    }

    /// Board catch-up: after_seq pages until a short page, tombstones
    /// included. Mirrors the reaction and edit loops.
    func catchUpBoard(serverMaxSeq: Int64) async {
        if boardCursor == 0 {
            // Nothing applied yet: one full read is cheaper than replaying
            // the whole history of every note that ever existed.
            await loadBoard()
            return
        }
        guard serverMaxSeq > boardCursor else { return }
        let limit = 100
        while true {
            guard let page = try? await api.boardChanges(afterSeq: boardCursor, limit: limit) else { return }
            for note in page { applyNote(note) }
            if let last = page.last { boardCursor = max(boardCursor, last.boardSeq) }
            saveContext()
            if page.count < limit { return }
        }
    }

    func addNote(text: String, color: String, size: String, x: Double, y: Double) async -> Bool {
        guard let dto = try? await api.createNote(text: text, color: color, size: size, x: x, y: y)
        else {
            return false
        }
        applyNote(dto)
        boardCursor = max(boardCursor, dto.boardSeq)
        saveContext()
        return true
    }

    /// Move (anyone) or rewrite (the author) — which fields are sent is
    /// what the server checks permission against. Text, colour and size
    /// are the author's; a MOVE must therefore send x/y and nothing else,
    /// or a non-author's drag comes back `not_note_author`.
    @discardableResult
    func updateNote(
        id: Int64,
        text: String? = nil,
        color: String? = nil,
        size: String? = nil,
        x: Double? = nil,
        y: Double? = nil
    ) async -> Bool {
        guard let dto = try? await api.patchNote(
            id: id, text: text, color: color, size: size, x: x, y: y)
        else {
            return false
        }
        applyNote(dto)
        boardCursor = max(boardCursor, dto.boardSeq)
        saveContext()
        return true
    }

    func deleteNote(id: Int64) async -> Bool {
        do {
            try await api.deleteNote(id: id)
        } catch {
            return false
        }
        if let existing = fetchNote(id) { modelContext.delete(existing) }
        saveContext()
        return true
    }

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
        let excerpt = ReplyToSnapshot.excerpt(of: body)
        let direct = FetchDescriptor<MessageEntity>(
            predicate: #Predicate { $0.replyToMessageID == quotedID })
        for row in (try? modelContext.fetch(direct)) ?? [] {
            row.replyExcerpt = excerpt
        }
        // The edited message may also be the SECOND level of somebody
        // else's quote. Without this pass that excerpt silently goes stale
        // — the quote block would show text its author had already changed,
        // which is the exact failure recomputing-on-read exists to prevent.
        let grand = FetchDescriptor<MessageEntity>(
            predicate: #Predicate { $0.replyParentMessageID == quotedID })
        for row in (try? modelContext.fetch(grand)) ?? [] {
            row.replyParentExcerpt = excerpt
        }
    }

    /// The server recomputes the quote on every read, so the newest copy
    /// always wins — including when it is absent, which is the honest
    /// answer for a message that is not a reply. (Unlike reactions, where
    /// ABSENT means "no data" and must never wipe: a reply cannot stop
    /// being one, so there is no state to protect here.)
    ///
    /// The SECOND level is blanket-overwritten too, and unlike the first it
    /// legitimately BECOMES absent: retention sweeping a grandparent leaves
    /// the reply intact and its `parent` gone. Leaving a stale copy behind
    /// would draw a quote of a message that no longer exists.
    private func applyReply(_ dto: MessageDTO, to entity: MessageEntity) {
        entity.replyToMessageID = dto.replyTo?.messageID
        entity.replySenderID = dto.replyTo?.senderID
        entity.replyExcerpt = dto.replyTo?.excerpt
        entity.replyParentMessageID = dto.replyTo?.parent?.messageID
        entity.replyParentSenderID = dto.replyTo?.parent?.senderID
        entity.replyParentExcerpt = dto.replyTo?.parent?.excerpt
        applyAttachment(dto, to: entity)
    }

    /// An attachment set is fixed at send time and never changes — except
    /// `has_preview`, which flips once the client's preview upload lands,
    /// so a non-empty incoming copy always wins wholesale. The read rule
    /// is MessageDTO.attachmentList (prefer `attachments`, fall back to
    /// the legacy `attachment`), and an ABSENT field never wipes stored
    /// state — the third-time rule, now on its fourth field.
    private func applyAttachment(_ dto: MessageDTO, to entity: MessageEntity) {
        let list = dto.attachmentList
        guard !list.isEmpty else { return }
        entity.applyAttachmentList(list)
    }

    /// A call record is written once by the server and never changes, and
    /// an incoming copy WITHOUT one says nothing about a record already
    /// stored — the absent-field rule, third time this project has been
    /// bitten by it. So the record is only ever set, never cleared. All
    /// three columns move together: the video flag is part of the record,
    /// and this is also the path that repairs a row cached before the
    /// flag was stored at all.
    private func applyCall(_ dto: MessageDTO, to entity: MessageEntity) {
        guard let call = dto.call else { return }
        entity.callOutcome = call.outcome
        entity.callDurationSecs = call.durationSecs
        entity.callVideo = call.video
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
            applyCall(dto, to: existing)
        } else if let existing = fetchMessage(localID: "s:\(dto.id)") {
            // Case 2b: idempotent re-delivery of a server-keyed row.
            applyBody(dto, to: existing)
            existing.createdAt = dto.createdAt
            existing.senderID = dto.senderID
            existing.state = .sent
            entity = existing
            applyReply(dto, to: existing)
            applyCall(dto, to: existing)
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
                pollJSON: Self.pollJSON(dto.poll),
                pollSeq: dto.poll?.pollSeq ?? 0,
                replyToMessageID: dto.replyTo?.messageID,
                replySenderID: dto.replyTo?.senderID,
                replyExcerpt: dto.replyTo?.excerpt,
                editSeq: dto.editSeq ?? 0,
                editedAt: dto.editedAt,
                attachment: dto.attachment,
                attachments: dto.attachmentList,
                call: dto.call)
            modelContext.insert(entity)
        }
        // Embedded reaction state (history/resync pages): the same seq
        // guard as live frames, so a page fetched before a live frame can
        // never clobber newer state — and ABSENT fields never wipe.
        if let seq = dto.reactionSeq, seq > entity.reactionSeq {
            entity.reactionSeq = seq
            entity.reactionList = Self.reactionSnapshots(dto.reactions ?? [])
        }
        // An embedded POLL gets the same guard and one extra rule: it must
        // NOT advance the chat's poll cursor. A history page proves
        // nothing about other polls' lower seqs, and a cursor moved on
        // that evidence would skip states the catch-up feed still owes us.
        // An absent poll is silence, never an erasure — a poll dies only
        // with its message.
        if let poll = dto.poll, poll.pollSeq > entity.pollSeq {
            entity.pollSeq = poll.pollSeq
            entity.pollJSON = Self.pollJSON(poll)
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
            chat.lastMessagePreview = Self.preview(
                body: dto.body, attachments: dto.attachmentList,
                call: dto.call, isMine: dto.senderID == currentUserID)
            chat.lastMessageDate = dto.createdAt
            chat.lastMessageSenderID = dto.senderID
        }
        guard dto.senderID != currentUserID else { return }
        // A catch-up page is HISTORY, not mail arriving in front of
        // anybody: `bumpUnread: false` means the server's unread_count from
        // GET /chats is the truth, so this path must neither add to it nor
        // — and this is the one that cost the badge — clear it. The resync
        // that follows a foregrounding used to walk every missed message
        // through here while the view still held the claim, marking them
        // read one at a time and throwing away the count step 3 had written
        // from the server seconds earlier.
        guard bumpUnread else { return }
        if isReading(chat.chatID) {
            // Seen as it lands: the newest message is on screen with the
            // app in front of the reader, and the thread follows it down.
            // Suppressing the bump rather than bumping and then clearing is
            // deliberate — the round trip flickers the app-icon badge.
            markRead(chatID: chat.chatID)
        } else {
            chat.unreadCount += 1
            liveBumpedMessageIDs[chat.chatID, default: []].insert(dto.id)
        }
    }

    /// Tell the person at the Mac that something arrived, when they are not
    /// already looking at it.
    ///
    /// A phone and an Android device hear about this from APNs/FCM. A Mac
    /// cannot: it holds its socket open for as long as the app runs, and
    /// the server pushes only to a device whose session is not live — so
    /// the app that received the frame is the only thing that can say
    /// anything, and until now it said nothing at all.
    ///
    /// "Not already looking at it" is `isReading` and nothing else, which
    /// is the same answer the unread rule above just used. A message that
    /// counts as unread is exactly a message worth telling somebody about;
    /// if the two ever disagreed the loser would be a message silently
    /// marked read AND silently not announced.
    private func announce(_ dto: MessageDTO) {
        #if os(macOS)
        guard dto.senderID != currentUserID, !isReading(dto.chatID) else { return }
        guard let chat = fetchChat(dto.chatID) else { return }
        ChatNotifier.announce(
            chatID: dto.chatID,
            messageID: dto.id,
            title: ChatNotifier.title(
                chatKind: chat.kind, chatTitle: chat.title, senderName: displayName(of: dto.senderID)),
            body: ChatNotifier.body(text: dto.body, attachments: dto.attachmentList, call: dto.call))
        #endif
    }

    /// Who a sender is, by the same rules the views use: the roster, then
    /// the assistant (which belongs to no family and is therefore in no
    /// roster by design), then a name for somebody we have not met yet.
    private func displayName(of userID: Int64) -> String {
        if let assistantID = AppSettings.assistantUserID, userID == assistantID {
            return AppSettings.assistantName ?? String(localized: "Assistant")
        }
        return fetchMember(userID)?.resolvedDisplayName ?? String(localized: "Someone")
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

    // MARK: - Polls

    /// A poll as the store holds it: the wire object, verbatim.
    ///
    /// Re-encoded rather than passed through as the received bytes,
    /// because the same function has to serve a poll this device made up
    /// optimistically — but the KEYS are the wire's (PollSnapshot spells
    /// them out), so a stored poll still diffs against protocol.md.
    private static func pollJSON(_ poll: PollDTO?) -> String? {
        guard let poll else { return nil }
        return pollJSON(pollSnapshot(poll))
    }

    private static func pollJSON(_ poll: PollSnapshot) -> String? {
        guard let data = try? pollEncoder.encode(poll) else { return nil }
        return String(decoding: data, as: UTF8.self)
    }

    /// One encoder, not one per apply — the rule MessageEntity's reaction
    /// coders already record, for the same measured reason.
    private static let pollEncoder = JSONEncoder()

    /// PollDTO → the entity's stored value shape.
    private static func pollSnapshot(_ poll: PollDTO) -> PollSnapshot {
        PollSnapshot(
            pollSeq: poll.pollSeq,
            closed: poll.closed,
            options: poll.options.map {
                PollOptionSnapshot(id: $0.id, text: $0.text, votes: $0.votes)
            })
    }

    /// THE seq-guarded apply path for one message's full poll state —
    /// live `poll` frames, catch-up pages and the vote/close responses all
    /// land here (a poll embedded on a fetched Message gets the same guard
    /// inside `upsert`). The state is full, never a delta, so applying is
    /// a plain rewrite; a seq at or below what the row holds is a stale
    /// re-delivery and a no-op. Returns false when the message isn't held
    /// locally — the state is dropped silently, exactly as a reaction
    /// state is (history paging re-delivers it embedded on the Message).
    @discardableResult
    func applyPollState(messageServerID: Int64, poll: PollDTO) -> Bool {
        guard let row = fetchMessage(serverID: messageServerID) else { return false }
        if poll.pollSeq > row.pollSeq {
            row.pollSeq = poll.pollSeq
            row.pollJSON = Self.pollJSON(poll)
            saveContext()
        }
        return true
    }

    /// MAX-advance a chat's poll cursor. Live frames and catch-up pages
    /// only: never an embedded poll (see `upsert`).
    private func advancePollCursor(chatID: Int64, seq: Int64) {
        guard let chat = fetchChat(chatID), seq > chat.maxPollSeq else { return }
        chat.maxPollSeq = seq
        saveContext()
    }

    /// A tap on an option: the one this user already holds retracts,
    /// anything else sets.
    ///
    /// The protocol's vote is an idempotent state-set rather than a toggle
    /// and says outright that whether tapping your current choice means
    /// "keep it" or "clear it" is each client's decision — this one
    /// clears, because the bubble has no other way to un-vote.
    ///
    /// Optimistic, the exact shape `toggleReaction` has: the local poll is
    /// rewritten before the REST call but `pollSeq` is NOT touched — only
    /// the server mints sequences, so the authoritative response (a
    /// greater seq) still applies through the guarded path, where a
    /// bumped one would have been dropped as stale. On failure the
    /// pre-vote state is restored unless a newer authoritative state
    /// landed meanwhile, which the revert must not clobber. No retry:
    /// a vote is cheap to re-tap.
    func vote(localID: String, optionID: Int64) async {
        guard let row = fetchMessage(localID: localID),
              let serverID = row.serverID,
              let poll = row.poll
        else { return }
        // A closed poll refuses votes server-side (`poll_closed`), so
        // there is nothing to be optimistic about.
        guard !poll.closed, poll.options.contains(where: { $0.id == optionID }) else { return }

        let chatID = row.chatID
        let userID = currentUserID
        let previousJSON = row.pollJSON
        let seqAtVote = row.pollSeq
        let retracting = PollPresentation.tapRetracts(
            optionID: optionID, in: poll, currentUserID: userID)

        row.poll = PollPresentation.applyingVote(
            poll, optionID: optionID, userID: userID, retracting: retracting)
        saveContext()

        do {
            let state = retracting
                ? try await api.retractVote(chatID: chatID, messageID: serverID)
                : try await api.vote(chatID: chatID, messageID: serverID, optionID: optionID)
            // The ROW moves; the CHAT CURSOR deliberately does not — the
            // rule the reaction reply follows, and the one the protocol
            // asks for ("the cursor advancing exactly as the reaction one
            // does"). A cursor is a chat-wide watermark, and one poll's
            // seq is no evidence about another's: REST works while the
            // socket is down, so a vote answered with seq 100 would push
            // the cursor past somebody else's seq 99 whose frame was
            // never delivered, and the next resync — comparing
            // max_poll_seq against a cursor already at 100 — would ask
            // for nothing. That state would then be lost until the poll
            // holding it next changed. One redundant catch-up page is the
            // cheaper mistake.
            applyPollState(messageServerID: state.messageID, poll: state.poll)
        } catch APIError.unauthorized {
            session?.handleUnauthorized()
        } catch {
            AppLog.sync.info("Vote failed for \(localID, privacy: .public): \(String(describing: error))")
            if let current = fetchMessage(localID: localID), current.pollSeq == seqAtVote {
                current.pollJSON = previousJSON
                saveContext()
            }
        }
    }

    /// Close a poll: the author's act, and one-way. The family owner does
    /// not outrank them here, exactly as with editing — the UI hides the
    /// action for everybody else, and this is the author's own device.
    ///
    /// Deliberately NOT optimistic: a refusal (not the author, message
    /// gone) would otherwise leave a poll on screen refusing votes it
    /// would in fact still take. Returns false so the caller can say so.
    @discardableResult
    func closePoll(localID: String) async -> Bool {
        guard let row = fetchMessage(localID: localID), let serverID = row.serverID else {
            return false
        }
        let chatID = row.chatID
        do {
            let state = try await api.closePoll(chatID: chatID, messageID: serverID)
            // The row only — see `vote` for why a REST reply must not move
            // the chat-wide cursor.
            applyPollState(messageServerID: state.messageID, poll: state.poll)
            return true
        } catch APIError.unauthorized {
            session?.handleUnauthorized()
            return false
        } catch {
            AppLog.sync.info("Close poll failed for \(localID, privacy: .public): \(String(describing: error))")
            return false
        }
    }

    /// Start a poll: an ordinary message whose BODY is the question, with
    /// the options riding beside it (docs/protocol.md, "Polls").
    ///
    /// Optimistic like `send` and unlike `sendMedia`, because there is
    /// nothing to upload first — and the pending row carries a poll of its
    /// own, so the bubble draws as a poll straight away rather than as a
    /// bare question that turns into one. That local copy uses NEGATIVE
    /// option ids (the server's are positive, so the two can never be
    /// confused) and `pollSeq: 0`: nothing may be voted on before the
    /// message has a server id anyway, and a zero seq is exactly what lets
    /// the ack's authoritative poll pass the guard.
    ///
    /// The question is the body, so the chat-list preview, the push and a
    /// reply excerpt all need no new case.
    ///
    /// It takes a quote like every other send door, because a poll may be
    /// a reply: `POST /chats/{id}/messages` accepts `reply_to_message_id`
    /// beside `poll` (only `poll` and `attachment_id` are mutually
    /// exclusive). Without it a poll started while a reply was primed
    /// dropped the quote AND left the banner armed, so the next ordinary
    /// message silently became that reply — the pair of bugs the media and
    /// location doors were fixed for.
    @discardableResult
    func sendPoll(
        question: String,
        options: [String],
        in chatID: Int64,
        replyTo: ReplyToDTO? = nil
    ) -> String? {
        // A poll's body may NOT be empty, unlike a message carrying an
        // attachment: `message_empty` applies to a poll with no question.
        guard !question.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              let sanitized = PollPresentation.sanitizedOptions(options)
        else { return nil }
        guard let localID = enqueue(
            body: question, in: chatID, replyTo: replyTo, pollOptions: sanitized)
        else {
            return nil
        }
        pendingDelivery = Task { await self.deliver(localID: localID) }
        return localID
    }

    /// The options a not-yet-acked poll must be re-sent with — read back
    /// off the row, so a retry after a relaunch still carries them.
    private func pendingPollOptions(of row: MessageEntity) -> [String]? {
        guard row.serverID == nil, let poll = row.poll, !poll.options.isEmpty else { return nil }
        return poll.options.map(\.text)
    }

    /// One poll catch-up loop: after_seq pages until a short page, each
    /// state applied under the per-message seq guard. Byte for byte the
    /// shape the reaction and edit loops have — the cursor advances to
    /// every page's max seq EVEN when the message it names is not held
    /// (those states are dropped; history paging re-delivers them embedded
    /// on the Message objects), because the cursor is what we have
    /// PROCESSED, never what we have kept.
    private func runPollCatchUp(_ step: SyncPlan.ReactionFetchStep) async {
        let limit = 100
        var afterSeq = step.afterSeq
        while true {
            guard let page = try? await api.polls(chatID: step.chatID, afterSeq: afterSeq, limit: limit)
            else { return }
            for state in page {
                applyPollState(messageServerID: state.messageID, poll: state.poll)
            }
            if let last = page.last { afterSeq = max(afterSeq, last.poll.pollSeq) }
            advancePollCursor(chatID: step.chatID, seq: afterSeq)
            if page.count < limit { return }
        }
    }

    // MARK: - Send pipeline

    /// Optimistic send: the pending bubble exists before this returns.
    /// Returns the new row's localID (nil for an empty body).
    @discardableResult
    func send(body: String, in chatID: Int64, replyTo: ReplyToDTO? = nil) -> String? {
        guard let localID = enqueue(body: body, in: chatID, replyTo: replyTo) else { return nil }
        pendingDelivery = Task { await self.deliver(localID: localID) }
        return localID
    }

    /// Send a photo or video.
    ///
    /// The upload happens FIRST and the message is enqueued only once it
    /// has an id — a message referring to bytes that never arrived would be
    /// a bubble pointing at nothing. Which is also why this is not
    /// optimistic: the composer holds the picked media, with progress,
    /// until the server has it.
    ///
    /// The preview upload is deliberately best-effort and AFTER the send:
    /// a bubble with no preview is a bubble that fetches the full image,
    /// which is worse but not broken — whereas delaying the message on a
    /// thumbnail would be silly.
    /// What went wrong, with nothing identifying in it.
    ///
    /// A `URLError`'s description embeds the failing URL, which for an
    /// attachment upload is metadata in a query string. The code alone says
    /// what happened without saying what it was about.
    nonisolated static func reason(_ error: Error) -> String {
        switch error {
        case let APIError.transport(urlError):
            return "transport(\(urlError.code.rawValue))"
        case let apiError as APIError:
            // The protocol's own error shapes carry a machine code and no
            // payload, so these are safe as they are.
            return String(describing: apiError)
        default:
            return String(describing: type(of: error))
        }
    }

    /// Share a place.
    ///
    /// The same shape as `sendMedia` — upload first, enqueue the message
    /// only once the attachment has an id — and for the same reason: a
    /// bubble pointing at an upload that failed is worse than a composer
    /// that is visibly busy. What differs is that there is nothing to
    /// upload, no file to seed a cache with and none to delete afterwards
    /// (docs/protocol.md, "Locations").
    func sendLocation(
        latitude: Double,
        longitude: Double,
        accuracyM: Int?,
        label: String?,
        caption: String = "",
        replyTo: ReplyToDTO? = nil,
        in chatID: Int64
    ) async -> Bool {
        let attachment: AttachmentDTO
        do {
            attachment = try await api.uploadLocation(
                latitude: latitude,
                longitude: longitude,
                accuracyM: accuracyM,
                name: label)
        } catch APIError.unauthorized {
            session?.handleUnauthorized()
            return false
        } catch {
            // The ERROR only, never `String(describing:)`.
            //
            // A location's coordinates ride in the upload's query string,
            // and a URLError carries the failing URL in its userInfo — so
            // describing the error writes a family member's position, to
            // seven decimal places, into the unified log where any
            // profile-enabled Mac can read it. Every other upload here logs
            // a describable error safely because its URL carries only an
            // id; this one does not.
            AppLog.sync.error("Location upload failed: \(Self.reason(error), privacy: .public)")
            return false
        }

        guard let localID = enqueue(
            body: caption, in: chatID, replyTo: replyTo, allowEmpty: true)
        else {
            return false
        }
        if let row = fetchMessage(localID: localID) {
            // The own-send write site: JSON set plus flat first-item
            // columns, like every other write site. A location is always
            // alone (docs/protocol.md), so the set is one.
            row.applyAttachmentList([attachment])
            saveContext()
        }
        pendingDelivery = Task { await self.deliver(localID: localID) }
        return true
    }

    /// The one-attachment spelling, kept for its callers and tests. Same
    /// contract as before plurality: the prepared file is consumed either
    /// way — deleted on whole-send success by the array path, and
    /// deleted here on failure.
    func sendMedia(
        _ prepared: MediaPrep.Prepared,
        caption: String,
        replyTo: ReplyToDTO? = nil,
        in chatID: Int64
    ) async -> Bool {
        let sent = await sendMedia([prepared], caption: caption, replyTo: replyTo, in: chatID)
        if !sent {
            try? FileManager.default.removeItem(at: prepared.fileURL)
        }
        return sent
    }

    /// Send up to ten attachments as ONE message, in the order given.
    ///
    /// Each item is uploaded in order (bytes, then its best-effort
    /// preview), the ids are collected, and ONE message is enqueued
    /// claiming the whole array — all-or-nothing on the server's side
    /// (docs/protocol.md, "Photos, videos, audio, files and locations").
    ///
    /// Failure anywhere returns false with the message never enqueued
    /// and EVERY prepared file left on disk — the files are consumed
    /// only on whole-send success — so the composer can re-stage the
    /// entire set and a retry cannot silently send a subset; ids already
    /// uploaded but never claimed are simply abandoned — the server
    /// sweeps unclaimed attachments after 24 hours, which is exactly
    /// what the sweep exists for.
    ///
    /// `onItemStart` reports (index, total) as each upload begins, so the
    /// composer can say "Uploading 2 of 5…" through its existing strip.
    func sendMedia(
        _ prepared: [MediaPrep.Prepared],
        caption: String,
        replyTo: ReplyToDTO? = nil,
        in chatID: Int64,
        onItemStart: ((Int, Int) -> Void)? = nil
    ) async -> Bool {
        guard !prepared.isEmpty else { return false }
        var uploaded: [AttachmentDTO] = []
        for (index, item) in prepared.enumerated() {
            onItemStart?(index + 1, prepared.count)
            let attachment: AttachmentDTO
            do {
                attachment = try await api.uploadAttachment(
                    fileURL: item.fileURL,
                    mime: item.mime,
                    kind: item.kind,
                    width: item.width,
                    height: item.height,
                    durationMS: item.durationMS,
                    name: item.name)
            } catch APIError.unauthorized {
                session?.handleUnauthorized()
                return false
            } catch {
                // .error, not .info: an upload that failed is the one thing
                // the sender will come asking about, and info-level os_log
                // does not reach Xcode's console.
                AppLog.sync.error("Attachment upload failed: \(String(describing: error), privacy: .public)")
                return false
            }

            // Seed the cache with what we already hold, so this device
            // draws its own bubble immediately instead of fetching back
            // bytes it just produced.
            if let preview = item.previewJPEG {
                attachmentStore?.seed(preview, id: attachment.id, preview: true)
            }
            if item.kind == "photo", let full = try? Data(contentsOf: item.fileURL) {
                attachmentStore?.seed(full, id: attachment.id, preview: false)
            }

            var hasPreview = false
            if let preview = item.previewJPEG {
                // Best-effort: a bubble with no preview fetches the full
                // image, which is worse but not broken. The RESULT is what
                // the pending row records — claiming a preview that failed
                // would leave the bubble waiting for bytes that are not
                // there until the ack corrects it.
                do {
                    try await api.uploadPreview(attachmentID: attachment.id, jpeg: preview)
                    hasPreview = true
                } catch {
                    AppLog.sync.error("Preview upload failed: \(String(describing: error), privacy: .public)")
                }
            }
            uploaded.append(attachment.withPreviewFlag(hasPreview))
        }

        guard let localID = enqueue(
            body: caption, in: chatID, replyTo: replyTo, allowEmpty: true)
        else {
            return false
        }
        if let row = fetchMessage(localID: localID) {
            // The own-send write site: the pending bubble draws its whole
            // set straight away; the ack replaces it with the server's copy.
            row.applyAttachmentList(uploaded)
            saveContext()
        }
        // The prepared files are consumed ONLY here, on whole-send
        // success. Any failure above left every file on disk — even the
        // ones whose uploads landed — so the composer restores the
        // ENTIRE set and a retry offers exactly what was composed; the
        // ids of uploads that landed on a failed send are abandoned to
        // the server's 24-hour sweep of unclaimed attachments.
        for item in prepared {
            try? FileManager.default.removeItem(at: item.fileURL)
        }
        pendingDelivery = Task { await self.deliver(localID: localID) }
        return true
    }

    /// A local URL for a file attachment, downloading it if this device
    /// does not have it yet.
    ///
    /// The file is written under its REAL NAME inside a per-attachment
    /// directory, because the name is what Quick Look puts in its title
    /// bar and what Share hands to the next app — a cache keyed
    /// "34.bin" would send "34.bin" to the recipient. The directory is
    /// what keeps two files called `Invoice.pdf` apart.
    ///
    /// Returns nil when the bytes could not be fetched; the caller says so.
    func localFileURL(for attachment: AttachmentDTO) async -> URL? {
        let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        let directory = caches
            .appendingPathComponent("files", isDirectory: true)
            .appendingPathComponent(String(attachment.id), isDirectory: true)
        let name = Self.safeFileName(attachment.name ?? Self.fallbackName(for: attachment))
        let destination = directory.appendingPathComponent(name)

        if FileManager.default.fileExists(atPath: destination.path) {
            return destination
        }
        guard let data = try? await api.attachmentData(id: attachment.id, preview: false) ?? nil
        else {
            // A 404 (gone, or not ours) and a transport failure look the
            // same to the caller: it says "couldn't download" either way.
            return nil
        }
        do {
            try FileManager.default.createDirectory(
                at: directory, withIntermediateDirectories: true)
            try data.write(to: destination, options: .atomic)
        } catch {
            AppLog.sync.info("Caching attachment failed: \(String(describing: error))")
            return nil
        }
        return destination
    }

    /// What to call a photo or video, which carry no name of their own.
    ///
    /// The EXTENSION is the part that matters: Photos refuses a video
    /// whose file does not look like one, and a share sheet decides what
    /// it can offer from it.
    nonisolated static func fallbackName(for attachment: AttachmentDTO) -> String {
        let ext = switch attachment.mime {
        case "image/jpeg": "jpg"
        case "image/png": "png"
        case "image/heic": "heic"
        case "image/heif": "heif"
        case "video/mp4": "mp4"
        case "video/quicktime": "mov"
        default: attachment.isVideo ? "mp4" : "jpg"
        }
        return "\(attachment.isVideo ? "video" : "photo")-\(attachment.id).\(ext)"
    }

    /// A filename safe to create on this device. The server sanitises what
    /// goes in its header; this is about the local filesystem — a name
    /// with a slash in it would silently become a path.
    nonisolated static func safeFileName(_ name: String) -> String {
        let cleaned = name
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: ":", with: "_")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        // Leading dots hide the file from every picker it is handed to —
        // all of them, so "..\u{2009}/x" style names cannot leave one behind.
        let visible = String(cleaned.drop(while: { $0 == "." }))
        return visible.isEmpty ? "file" : String(visible.prefix(255))
    }

    /// What the chat list shows on its second line.
    ///
    /// A photo is normally sent with no caption, and an empty string is not
    /// nil — so the row rendered blank rather than falling back to "No
    /// messages yet". What arrived is the useful thing to say.
    ///
    /// The single-attachment spelling, kept for the callers (and tests)
    /// that hold one attachment or none. The array version below is the
    /// real rule.
    nonisolated static func preview(
        body: String,
        attachment: AttachmentDTO?,
        call: CallDTO? = nil,
        isMine: Bool = false
    ) -> String {
        preview(
            body: body, attachments: attachment.map { [$0] } ?? [],
            call: call, isMine: isMine)
    }

    /// The plural rule, mirroring the server's push summaries
    /// (docs/protocol.md, "Push notifications"): a caption always wins;
    /// ONE attachment says what it is by name or kind; several of one
    /// kind become a count ("3 Photos" — names give way to the count);
    /// a mixed set is "N attachments".
    nonisolated static func preview(
        body: String,
        attachments: [AttachmentDTO],
        call: CallDTO? = nil,
        isMine: Bool = false
    ) -> String {
        // A call record's body is an English placeholder for clients that
        // predate calls; this one knows the object and never shows it.
        if let call { return CallRecordText.label(call, isMine: isMine) }
        if !body.isEmpty { return body }
        guard let attachment = attachments.first else { return body }
        if attachments.count > 1 {
            let kinds = Set(attachments.map(\.kind))
            guard kinds.count == 1 else {
                return String(localized: "\(attachments.count) attachments")
            }
            switch attachment.kind {
            case AttachmentDTO.Kind.photo:
                return String(localized: "\(attachments.count) Photos")
            case AttachmentDTO.Kind.video:
                return String(localized: "\(attachments.count) Videos")
            case AttachmentDTO.Kind.audio:
                return String(localized: "\(attachments.count) Audio")
            case AttachmentDTO.Kind.file:
                return String(localized: "\(attachments.count) Files")
            default:
                // A location is always alone (the server refuses it in
                // company), so this is a future kind — the honest word.
                return String(localized: "\(attachments.count) attachments")
            }
        }
        if attachment.isVideo { return String(localized: "Video") }
        if attachment.isAudio {
            return attachment.name.flatMap { $0.isEmpty ? nil : $0 }
                ?? String(localized: "Audio")
        }
        // A location's label, or the word — never its coordinates. A chat
        // list is one line, and a row of digits says nothing about where.
        if attachment.isLocation {
            return attachment.name.flatMap { $0.isEmpty ? nil : $0 }
                ?? String(localized: "Location")
        }
        if attachment.isFile {
            return attachment.name.flatMap { $0.isEmpty ? nil : $0 }
                ?? String(localized: "File")
        }
        return String(localized: "Photo")
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
            // The ROW only. `upsert` applies `edit_seq` under the
            // per-message guard; the CHAT-WIDE cursor deliberately does not
            // move, which is the rule `vote`, `closePoll` and
            // `toggleReaction` already follow and the one protocol.md sets
            // out under "Best-effort delivery": only a live frame and a
            // catch-up page may advance a chat cursor, never the HTTP reply
            // to this client's own change.
            //
            // It used to move here, and that is a lost edit rather than a
            // tidiness point. REST goes on working while the socket is
            // down, which is exactly when the frames carrying LOWER seqs
            // were missed: my edit answered with `edit_seq` 100 pushed this
            // cursor past somebody else's 99, the next resync's
            // `max_edit_seq > cursor` test then asked for nothing, and
            // their rewrite never arrived — until that message happened to
            // be edited again. One redundant catch-up page is the cheaper
            // mistake.
            _ = upsert(dto, bumpUnread: false)
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
    func enqueue(
        body: String,
        in chatID: Int64,
        replyTo: ReplyToDTO? = nil,
        allowEmpty: Bool = false,
        /// The options that make this message a poll. The row gets a
        /// provisional poll built from them — negative ids, no votes,
        /// `pollSeq: 0` — so the bubble draws as a poll immediately and
        /// the ack's real one still passes the guard. See `sendPoll`.
        pollOptions: [String]? = nil
    ) -> String? {
        let trimmed = body.trimmingCharacters(in: .whitespacesAndNewlines)
        // A photo needs no caption (docs/protocol.md) — but only an
        // attachment may waive the non-empty rule.
        guard !trimmed.isEmpty || allowEmpty else { return nil }
        let uuid = UUID().uuidString.lowercased()
        let now = Date()
        let provisionalPoll = pollOptions.map { options in
            PollSnapshot(
                pollSeq: 0,
                closed: false,
                options: options.enumerated().map { index, text in
                    PollOptionSnapshot(id: -Int64(index + 1), text: text)
                })
        }
        let entity = MessageEntity(
            localID: "c:\(uuid)",
            serverID: nil,
            clientMsgID: uuid,
            chatID: chatID,
            senderID: currentUserID,
            body: trimmed,
            createdAt: now,
            status: .pending,
            pollJSON: provisionalPoll.flatMap(Self.pollJSON),
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
        // The whole set's ids, in the sender's order — the array spelling
        // is the only one this client sends (`attachment_ids`).
        let ids = row.attachmentList.map(\.id)
        let attachmentIDs = ids.isEmpty ? nil : ids
        // Read off the row rather than passed in, so a retry — a sweep, a
        // tap on a failed bubble, a relaunch — still carries the options a
        // poll cannot be created without.
        let pollOptions = pendingPollOptions(of: row)
        row.state = .pending
        saveContext()

        // Leg 1: the socket, if it is up. A thrown notConnected skips the
        // ack race entirely and goes straight to REST.
        do {
            try await socket.send(.send(
                chatID: chatID,
                clientMsgID: clientMsgID,
                body: body,
                replyToMessageID: replyToMessageID,
                attachmentIDs: attachmentIDs,
                pollOptions: pollOptions))
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
                replyToMessageID: replyToMessageID,
                attachmentIDs: attachmentIDs,
                pollOptions: pollOptions)
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

    /// Re-fetch locations this device stored without their coordinates.
    ///
    /// **Catch-up only ever ADDS.** `after_id` asks for messages newer than
    /// the newest one held, so a message already in the cache is never read
    /// again — which means a row written by a build that dropped the
    /// coordinates stays broken FOREVER, on a device that has otherwise
    /// been fixed. A location has no bytes to fall back on, so such a row
    /// is a bubble with nothing in it at all.
    ///
    /// `before_id = serverID + 1, limit = 1` asks for exactly that one
    /// message through an endpoint that already exists, and `upsert` puts
    /// it back through the same path a live delivery takes. Bounded: at
    /// most `repairBatch` of them per resync, so a cache full of them
    /// cannot turn a reconnect into a storm of requests.
    private func repairLocationsMissingCoordinates() async {
        let kind = AttachmentDTO.Kind.location
        var descriptor = FetchDescriptor<MessageEntity>(
            predicate: #Predicate { row in
                row.attachmentKind == kind && row.attachmentLatitude == nil && row.serverID != nil
            })
        descriptor.fetchLimit = Self.repairBatch
        guard let broken = try? modelContext.fetch(descriptor), !broken.isEmpty else { return }
        AppLog.sync.info("Repairing \(broken.count, privacy: .public) location(s) with no coordinates")
        for row in broken {
            guard let serverID = row.serverID else { continue }
            guard
                let page = try? await api.messages(
                    chatID: row.chatID, beforeID: serverID + 1, limit: 1),
                let dto = page.first(where: { $0.id == serverID })
            else { continue }
            upsert(dto, bumpUnread: false)
        }
    }

    /// Most broken locations repaired per resync — see the note above.
    private static let repairBatch = 25

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
            // Both arrays, one store: the tombstones are what let a stored
            // message still name its sender. Only `members` is a roster
            // (protocol.md, "Deleting an account").
            upsertMembers(mine.members, formerMembers: mine.formerMembers)
            // The assistant is NOT a member and is not upserted as one —
            // it belongs to no family, so it appears in no roster. It is
            // kept aside purely so the family chat can put a name on its
            // messages and the composer knows whether to offer `@ai`.
            AppSettings.assistantUserID = mine.assistant?.userID
            AppSettings.assistantName = mine.assistant?.displayName
        }

        // 3. Chat list: server unread wins; direct chats the server
        // dropped go — with their messages and everything else keyed by
        // their id (see `dropChats`, and `deleteChat` under it).
        //
        // "Wins" with one correction. `unread_count` is computed on the
        // server BEFORE the response is sent, so a live `message` frame
        // applied while it was in flight may not be in it — and assigning
        // the response flat dropped that message from the count for good,
        // because the catch-up in step 4 never bumps. Add back exactly the
        // live messages the response shows the server had not counted.
        //
        // The response also carries this caller's OWN read marker per chat
        // (protocol.md, resync step 2), and a marker that moves FORWARD is
        // the only thing that ever tells this device the chat was read on
        // another one: the live `read` frame is relayed to other members
        // only, so a reader's own devices never see their own read go past.
        // The count corrects itself from `unread_count` either way; what
        // needs the marker is Notification Center, which no resync has ever
        // touched and which would otherwise still be showing banners for
        // messages this person read on their phone an hour ago.
        guard let chatList = try? await api.chats() else { return }
        var readElsewhere: [Int64: Int64] = [:]
        for item in chatList.chats {
            if let marker = upsertChat(item, uncountedLiveMessages: uncountedBumps(in: item)) {
                readElsewhere[item.chat.id] = marker
            }
        }
        dropChats(absentFrom: chatList.chats)
        saveContext()
        // After the save, so the icon is already showing the server's number
        // when the banners go.
        lastResyncReadElsewhere = readElsewhere
        ChatNotifier.dismissDelivered(readMarkers: readElsewhere)

        // 4. Per-chat catch-up: after_id loops until a short page.
        let cursors = chatList.chats.map {
            SyncPlan.ChatCursor(
                chatID: $0.chat.id,
                serverLatestMessageID: $0.lastMessage?.id,
                serverMaxReactionSeq: $0.maxReactionSeq ?? 0,
                serverMaxEditSeq: $0.maxEditSeq ?? 0,
                serverMaxPollSeq: $0.maxPollSeq ?? 0)
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

        // 6a. Poll catch-up, on the fourth cursor and for the same reason
        // the other two exist: `after_id` is WHERE id > cursor and can
        // never see a vote cast on an older message. Gated on the chat's
        // max_poll_seq from step 2, so a family that has never run a poll
        // costs no request at all.
        for step in SyncPlan.makePollSteps(chats: cursors, localCursors: localPollCursors()) {
            await runPollCatchUp(step)
        }

        // 7. Board catch-up, on the third cursor. The family read already
        // told us the server's max, so a board nothing has happened on
        // costs no request at all.
        if let mine = try? await api.myFamily() {
            await catchUpBoard(serverMaxSeq: mine.maxBoardSeq ?? 0)
        }

        // 6. Outbox sweep: anything still pending after 30 s gets re-sent
        // (same client_msg_id — the server dedups).
        await sweepOutbox()

        // 6a. Repair any location that was stored without its coordinates.
        await repairLocationsMissingCoordinates()

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

    /// Report everything currently held for `chatID` as read.
    ///
    /// Reached from exactly two places, both of which have already
    /// established that the person is LOOKING at the newest message with
    /// the app in front of them: `updatePresence`, and the live-message
    /// path for a chat they are reading. Never from appearing, selecting,
    /// foregrounding or resyncing — see ChatPresence for why the bar is
    /// that high.
    ///
    /// The local marker advances only once the server has actually taken
    /// the read. It used to advance first and swallow both failures, so a
    /// report that never reached the server left the marker permanently
    /// ahead of it: the next GET /chats re-inflated the badge, and the
    /// monotonic guard here then refused to send it again — a badge no
    /// amount of opening the chat could clear. The local count is zeroed
    /// immediately either way, because that is display, not truth, and must
    /// never be held hostage by the wire.
    func markRead(chatID: Int64) {
        guard let chat = fetchChat(chatID) else { return }
        let target = chat.maxServerMessageID
        let advances = target > chat.myLastReadID
        // A no-op read is a real case, not a defensive one: the view
        // re-publishes its presence on every scroll settle and every new
        // message, and most of those have nothing left to read.
        guard chat.unreadCount > 0 || advances else { return }
        chat.unreadCount = 0
        saveContext()
        // The banners are the same statement as the badge; leaving them in
        // Notification Center contradicts an icon that now says nothing.
        ChatNotifier.dismissDelivered(chatID: chatID)
        guard advances else { return }
        // Remember it instead of losing it: the report on the wire is for an
        // older message, and when it settles this is what gets sent.
        guard !readPostsInFlight.contains(chatID) else {
            pendingReadTargets[chatID] = max(pendingReadTargets[chatID] ?? 0, target)
            return
        }
        readPostsInFlight.insert(chatID)
        let socket = self.socket
        let api = self.api
        pendingReadPost = Task { [weak self] in
            var delivered = true
            do {
                try await socket.send(.read(chatID: chatID, lastReadMessageID: target))
            } catch {
                // REST is the fallback for a socket that is down, and its
                // answer is the one that decides — the old code discarded
                // it and assumed success.
                do {
                    try await api.markRead(chatID: chatID, lastReadMessageID: target)
                } catch {
                    delivered = false
                    AppLog.sync.info("Read report for chat \(chatID, privacy: .public) failed: \(String(describing: error))")
                }
            }
            guard let self else { return }
            self.readPostsInFlight.remove(chatID)
            // Only now may the throttle move: a failed report leaves the
            // marker where it was, so the next presence update sends it
            // again instead of the read being lost for good.
            if delivered, let chat = self.fetchChat(chatID), target > chat.myLastReadID {
                chat.myLastReadID = target
                self.saveContext()
            }
            // A newer message became due while this one was in flight. Send
            // it now — nothing else will ask: the reader is still at the
            // bottom, so no geometry moves, and the message that would have
            // re-asked has already arrived. `markRead` re-reads the chat, so
            // this posts whatever is newest rather than the remembered
            // number, and it is a no-op if the reader has since left.
            guard let queued = self.pendingReadTargets.removeValue(forKey: chatID),
                queued > (self.fetchChat(chatID)?.myLastReadID ?? 0)
            else { return }
            self.markRead(chatID: chatID)
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

    /// How many of this chat's live bumps the response in `item` cannot
    /// have counted — and forget the ones it can.
    ///
    /// The test is IDENTITY, not arithmetic: `last_message` and
    /// `unread_count` come from the same query, so a message the server had
    /// already stored is at or below `last_message.id` and is therefore
    /// already in the count. Anything above it reached this client before
    /// it reached that query, and is the message the correction exists for.
    /// Re-applying the same response can only ever produce the same answer,
    /// which a counter difference could not promise.
    ///
    /// Bumps at or below the line are dropped here: every later response
    /// carries a `last_message` at least as high, so they can never need
    /// adding back again, and a session that never resyncs is not a reason
    /// to remember every message id it ever received.
    private func uncountedBumps(in item: ChatListItemDTO) -> Int {
        guard let bumped = liveBumpedMessageIDs[item.chat.id] else { return 0 }
        let counted = item.lastMessage?.id ?? 0
        let uncounted = bumped.filter { $0 > counted }
        liveBumpedMessageIDs[item.chat.id] = uncounted.isEmpty ? nil : uncounted
        return uncounted.count
    }

    /// `uncountedLiveMessages`: live messages this client counted that the
    /// server's `unread_count` demonstrably does not include. See resync
    /// step 3 and `uncountedBumps`.
    ///
    /// Returns the chat's `last_read_message_id` if it moved this device's
    /// stored marker FORWARD, and nil otherwise — which is the resync's
    /// only evidence that the chat was read somewhere else. A marker that
    /// did not move proves nothing either way and must cost nothing: it is
    /// what this device already believed.
    @discardableResult
    private func upsertChat(
        _ item: ChatListItemDTO,
        resetWhenNoLastMessage: Bool = true,
        uncountedLiveMessages: Int = 0
    ) -> Int64? {
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
        chat.unreadCount = item.unreadCount + max(0, uncountedLiveMessages) // server-authoritative
        // The caller's OWN read marker, off the same row of the same query
        // as `unread_count` (protocol.md, GET /chats) — so the count and
        // the marker always describe one instant, and the count stays the
        // authority on the number while the marker says how far the person
        // has read.
        //
        // MONOTONICALLY, for the reason the server applies it that way
        // too: this response was built before it was sent, and one still in
        // flight while its owner keeps reading would otherwise walk the
        // local marker BACKWARDS — re-arming `markRead`'s throttle to
        // re-report a read the server already has, and telling the banner
        // teardown below that a chat it just settled is unread again. An
        // absent field (a server older than it) lands in the same place as
        // the `0` that means "has never reported reading anything here":
        // on the stored value, unchanged.
        var readMarkerAdvanced: Int64?
        if let marker = item.lastReadMessageID, marker > chat.myLastReadID {
            chat.myLastReadID = marker
            readMarkerAdvanced = marker
        }
        if let last = item.lastMessage {
            chat.lastMessagePreview = Self.preview(
                body: last.body, attachments: last.attachmentList,
                call: last.call, isMine: last.senderID == currentUserID)
            chat.lastMessageDate = last.createdAt
            chat.lastMessageSenderID = last.senderID
        } else if resetWhenNoLastMessage {
            chat.lastMessagePreview = nil
            chat.lastMessageDate = nil
            chat.lastMessageSenderID = nil
            // An empty chat has no history to page for.
            chat.hasFullHistory = true
        }
        return readMarkerAdvanced
    }

    /// THE local delete-by-chat, and the only one: everything this device
    /// holds that is keyed by a chat id goes here, in one place, so the two
    /// callers below cannot prune half of it each.
    ///
    /// Nothing in this protocol needed it until account deletion. A member
    /// leaving does NOT take a chat away — that history is retained and
    /// resurfaces if they rejoin — so the only chat that can genuinely
    /// vanish is a direct one whose peer deleted their account, and it
    /// vanishes in BOTH halves (docs/protocol.md, "Deleting an account").
    ///
    /// The messages go first and by hand: nothing here is a SwiftData
    /// relationship, so deleting the chat row alone would leave every one
    /// of its messages orphaned in the store — invisible, unreachable and
    /// still counted by anything that fetches messages without a chat
    /// predicate. The in-memory state keyed by the same id goes too, or
    /// the id comes back the moment somebody reuses it.
    private func deleteChat(_ chat: ChatEntity) {
        let chatID = chat.chatID
        let messages = (try? modelContext.fetch(
            FetchDescriptor<MessageEntity>(predicate: #Predicate { $0.chatID == chatID }))) ?? []
        // The cached bytes are keyed by attachment id, not chat id, so
        // they can only be found through the rows that are about to go.
        attachmentStore?.forget(attachmentIDs: messages.compactMap(\.attachmentID))
        for message in messages { modelContext.delete(message) }
        modelContext.delete(chat)
        // Everything else this object keys by chat id. The read markers
        // and the sync cursors are columns ON the row, so they go with it.
        releasePresence(chatID: chatID)
        typingByChat[chatID] = nil
        lastTypingSentAt[chatID] = nil
        liveBumpedMessageIDs[chatID] = nil
        readPostsInFlight.remove(chatID)
        pendingReadTargets[chatID] = nil
    }

    /// The GENERAL REPAIR: prune the direct chats a full `GET /chats` did
    /// not list. This is what heals a device that was asleep when a peer
    /// deleted their account — the frame that says so was never delivered
    /// (the socket is a live wire, not a queue), and nothing else would
    /// ever tell it.
    ///
    /// ONLY DIRECT CHATS, and that is deliberate rather than defensive: a
    /// direct chat is the only kind that can disappear. The family chat is
    /// in every response of every member of a family, the assistant thread
    /// is returned to its owner, and a client that pruned either because a
    /// response looked thin would be throwing away history the server still
    /// holds. An unknown future kind is left alone for the same reason.
    ///
    /// The caller runs this ONLY on a response that actually arrived and
    /// was complete — `GET /chats` is unpaginated, so "complete" is the
    /// same thing as "succeeded". On a network failure resync returns
    /// before reaching here, because pruning on a flaky connection would
    /// wipe somebody's history.
    private func dropChats(absentFrom items: [ChatListItemDTO]) {
        let serverIDs = Set(items.map { $0.chat.id })
        guard let locals = try? modelContext.fetch(FetchDescriptor<ChatEntity>()) else { return }
        for chat in locals where chat.kind == "direct" && !serverIDs.contains(chat.chatID) {
            deleteChat(chat)
        }
    }

    /// The IMMEDIATE half: a peer deleted their account, so the direct
    /// chat with them is already gone on the server — every request from
    /// inside it now answers 404 — and the member watching their chat list
    /// should see it go now rather than at the next resync.
    ///
    /// Keyed on the peer, because that is all `member_deleted` carries: a
    /// direct chat can outlive the family that created it, so the frame
    /// reaches people who share no family with the deleted account and
    /// carries a `family_id` that means nothing to them (protocol.md,
    /// "Server → client").
    private func dropDirectChat(peerUserID: Int64) {
        let peer: Int64? = peerUserID
        let descriptor = FetchDescriptor<ChatEntity>(
            predicate: #Predicate { $0.kind == "direct" && $0.peerUserID == peer })
        guard let chats = try? modelContext.fetch(descriptor), !chats.isEmpty else { return }
        for chat in chats { deleteChat(chat) }
        saveContext()
    }

    private func upsertMembers(_ members: [MemberDTO], formerMembers: [MemberDTO] = []) {
        let present = Set(members.map(\.id))
        for member in members {
            upsertMember(
                userID: member.id,
                username: member.username,
                displayName: member.displayName,
                // Every LIVE member carries a role; only a tombstone
                // does not, and tombstones do not come through here.
                role: member.role ?? "member",
                avatarVersion: member.avatarVersion,
                birthday: .some(member.birthday))
        }
        // Anyone we know locally but the roster omits has left; keep the
        // row (name resolution on old bubbles) but flag it.
        if let locals = try? modelContext.fetch(FetchDescriptor<MemberEntity>()) {
            for local in locals where !present.contains(local.userID) {
                local.hasLeft = true
            }
        }
        // The tombstones last. The sweep above has just flagged every
        // local row the roster omits as having left, and a former member
        // is one of those — but a tombstone says a great deal more than
        // `hasLeft`, so it is written over the top rather than under it.
        for former in formerMembers {
            applyMemberTombstone(former)
        }
        saveContext()
    }

    /// Write a deleted account's tombstone, DELIBERATELY.
    ///
    /// Not `upsertMember`, and that is the whole point: that path exists
    /// so an absent field can never clear a stored one, and this is the
    /// one place in the protocol where fields must be cleared — the
    /// picture, the birthday, the role and the name all go, and what is
    /// left is a row that can still put "Deleted account" against a
    /// message somebody still has (protocol.md, "Deleting an account").
    ///
    /// The row is INSERTED when this device never knew the person: their
    /// messages may well be in the family chat regardless, and a missing
    /// row draws them as "Someone".
    ///
    /// `displayName` is stored exactly as the server sent it (the English
    /// placeholder); every screen draws `resolvedDisplayName`, which is
    /// the translated one.
    func applyMemberTombstone(_ dto: MemberDTO) {
        let member = fetchMember(dto.id) ?? {
            let inserted = MemberEntity(
                userID: dto.id,
                username: dto.username,
                displayName: dto.displayName,
                role: "",
                isCurrentUser: false)
            modelContext.insert(inserted)
            return inserted
        }()
        member.username = dto.username
        member.displayName = dto.displayName
        member.accountDeleted = true
        // Not a member of anything any more: no role, and out of every
        // roster, picker and count.
        member.role = ""
        member.hasLeft = true
        member.isCurrentUser = false
        // Nothing to fetch — the server reports 0 and answers 404 for the
        // bytes, so a request would only cache a miss.
        member.avatarVersion = 0
        member.birthdayMonth = nil
        member.birthdayDay = nil
        saveContext()
    }

    /// Everyone this account has blocked, as the rest of the app reads it.
    ///
    /// Published rather than fetched at each call site because it is
    /// consulted on nearly every row that draws a person, and a
    /// `FetchDescriptor` per bubble is not a thing to do in a list.
    private(set) var blockedUserIDs: Set<Int64> = []

    /// Replace the whole set from a `blocked_user_ids` array.
    ///
    /// WHOLESALE, because the wire field is a complete state-set and never
    /// a delta — which is also why the server always sends it, `[]`
    /// included. Merging instead would leave an unblock made on another
    /// device in force here for ever (protocol.md, "Blocking a member").
    func replaceBlocks(with ids: [Int64]) {
        let incoming = Set(ids)
        guard let rows = try? modelContext.fetch(FetchDescriptor<BlockEntity>()) else { return }
        var held: Set<Int64> = []
        for row in rows {
            if incoming.contains(row.userID) {
                held.insert(row.userID)
            } else {
                modelContext.delete(row)
            }
        }
        for id in incoming.subtracting(held) {
            modelContext.insert(BlockEntity(userID: id))
            // A block made on ANOTHER device arrives here, never through
            // `applyBlock` — so the prune has to happen on this path too or
            // a cold start leaves the stale chat standing. Newly-arrived
            // ids only: re-dropping `held` would delete the chat again on
            // every `GET /me`.
            dropDirectChat(peerUserID: id)
        }
        blockedUserIDs = incoming
        saveContext()
    }

    /// Block a member: the request FIRST, then the local write.
    ///
    /// Never optimistic, and that ordering is the point. An optimistic
    /// block that then failed would hide rows the reader does not know are
    /// hidden, in a feature with no error surface and no badge to notice —
    /// they would simply stop seeing somebody and never learn why.
    ///
    /// Returns false when the request threw, having applied nothing.
    @discardableResult
    func block(userID: Int64) async -> Bool {
        do {
            try await api.blockMember(userID: userID)
        } catch {
            return false
        }
        applyBlock(userID: userID, blocked: true)
        return true
    }

    /// Unblock, then RESYNC — because the direct chat this block pruned
    /// locally comes back from `GET /chats`, not from anything held here.
    /// That is the protocol's own recovery path: "unblocking puts the chat
    /// back in `GET /chats` with its true `unread_count` and
    /// `last_read_message_id`".
    @discardableResult
    func unblock(userID: Int64) async -> Bool {
        do {
            try await api.unblockMember(userID: userID)
        } catch {
            return false
        }
        applyBlock(userID: userID, blocked: false)
        await resync()
        return true
    }

    /// File a report. Nothing local changes: a report is a write to the
    /// owner's inbox, and this reader's own view of the family does not
    /// move because of it — blocking and reporting are independent.
    @discardableResult
    func report(reportedUserID: Int64, reason: String, messageID: Int64?) async -> Bool {
        do {
            _ = try await api.createReport(
                reportedUserID: reportedUserID, reason: reason, messageID: messageID)
            return true
        } catch {
            return false
        }
    }

    /// Apply one block or unblock — the `member_blocked` frame, and the
    /// optimistic write behind the Block button.
    func applyBlock(userID: Int64, blocked: Bool) {
        let existing = try? modelContext.fetch(
            FetchDescriptor<BlockEntity>(predicate: #Predicate { $0.userID == userID })
        ).first
        if blocked {
            if existing == nil { modelContext.insert(BlockEntity(userID: userID)) }
            blockedUserIDs.insert(userID)
            // "For the blocker that chat is not merely unlisted; it is gone
            // from every path they can reach it by" (protocol.md, "Blocking
            // a member"). The server already refuses every request in it
            // with `blocked`, which nothing in this app maps to a message —
            // so a chat left in the store is one the reader can open and
            // then watch fail silently.
            //
            // Only on the way IN. Unblocking does not put it back from
            // here: `GET /chats` does, which is why `unblock` resyncs.
            dropDirectChat(peerUserID: userID)
        } else {
            if let existing { modelContext.delete(existing) }
            blockedUserIDs.remove(userID)
        }
        saveContext()
    }

    /// Load the set from the store at launch, before the first sync — so a
    /// cold start offline draws hidden rows hidden rather than briefly
    /// showing everything.
    func loadBlocksFromStore() {
        guard let rows = try? modelContext.fetch(FetchDescriptor<BlockEntity>()) else { return }
        blockedUserIDs = Set(rows.map(\.userID))
    }

    /// Apply a `family_owner` frame: the named user is the family's owner
    /// from now on.
    ///
    /// Both halves matter. The roster row is what the members list draws
    /// its "Owner" badge from, and the session's `role` is what unlocks
    /// the owner-only screens — a client that has just been handed the
    /// family would otherwise wait for its next `GET /me` to find out.
    private func applyFamilyOwner(familyID: Int64, userID: Int64) {
        // A frame about a family this device is not in is not ours to
        // apply; ignore it rather than rewriting a roster with it.
        if let known = session?.family?.id, known != familyID { return }
        if let locals = try? modelContext.fetch(FetchDescriptor<MemberEntity>()) {
            for local in locals where !local.accountDeleted {
                if local.userID == userID {
                    local.role = "owner"
                } else if local.role == "owner" {
                    local.role = "member"
                }
            }
        }
        saveContext()
        session?.applyFamilyOwner(userID: userID)
    }

    private func upsertMember(
        userID: Int64,
        username: String,
        displayName: String,
        role: String,
        avatarVersion: Int64,
        /// Doubly wrapped, and not for fun: `.none` means the caller was
        /// never told anything about a birthday and the stored one must
        /// survive, while `.some(nil)` means the roster says there is
        /// none. The `member_joined` frame carries no birthday at all
        /// (protocol.md, "Server → client"), so a member who leaves and
        /// rejoins would otherwise lose theirs to a frame that never
        /// mentioned it — the same "absent fields never wipe" rule the
        /// reaction upsert follows.
        birthday: BirthdayDTO?? = .none
    ) {
        if let existing = fetchMember(userID) {
            // Deletion is one-way and ids are never reused, so a live
            // roster entry for a tombstoned row can only be a response
            // that was already in flight when the account went. Applying
            // it would put the person's real name and picture back on a
            // row whose whole job is that they are gone.
            guard !existing.accountDeleted else { return }
            existing.username = username
            existing.displayName = displayName
            existing.role = role
            existing.isCurrentUser = userID == currentUserID
            existing.hasLeft = false
            existing.avatarVersion = avatarVersion
            if let birthday {
                existing.birthdayMonth = birthday?.month
                existing.birthdayDay = birthday?.day
            }
        } else {
            modelContext.insert(MemberEntity(
                userID: userID,
                username: username,
                displayName: displayName,
                role: role,
                isCurrentUser: userID == currentUserID,
                avatarVersion: avatarVersion,
                birthday: birthday ?? nil))
        }
        saveContext()
    }

    /// Write a birthday onto a roster row after this device edited one.
    ///
    /// There is no `member_updated` frame and no push for a birthday
    /// (protocol.md, "Birthdays") — every other device learns it on its
    /// next resync — so the device that made the change is the only one
    /// that can show it now, and it has the new value in hand.
    func applyMemberBirthday(userID: Int64, birthday: BirthdayDTO?) {
        guard let member = fetchMember(userID) else { return }
        member.birthdayMonth = birthday?.month
        member.birthdayDay = birthday?.day
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

    /// chatID → stored maxPollSeq, for planning the poll catch-up.
    private func localPollCursors() -> [Int64: Int64] {
        let chats = (try? modelContext.fetch(FetchDescriptor<ChatEntity>())) ?? []
        return Dictionary(uniqueKeysWithValues: chats.map { ($0.chatID, $0.maxPollSeq) })
    }

    private func saveContext() {
        do {
            try modelContext.save()
        } catch {
            AppLog.sync.error("ModelContext save failed: \(String(describing: error))")
        }
        refreshUnreadBadge()
    }

    /// The number the icon should be showing, according to the store alone.
    ///
    /// Needs no network and no push — which is the point: it is the only
    /// answer that exists on a device that cannot reach the server, and the
    /// answer the app takes over with the moment it starts running
    /// (UnreadBadge's header has the split).
    func storedUnreadTotal() -> Int {
        let chats = (try? modelContext.fetch(FetchDescriptor<ChatEntity>())) ?? []
        return UnreadBadge.total(unreadCounts: chats.map(\.unreadCount))
    }

    /// Push the total unread onto the app icon.
    ///
    /// Hung off `saveContext` rather than off each of the three places that
    /// write `unreadCount` — the live +1, `markRead`'s reset, and the
    /// server-authoritative overwrite on resync — because every one of them
    /// has to persist, so this is the one seam none of them can skip. A
    /// family has a handful of chats, so summing them is cheaper than
    /// keeping a running total correct across those three paths.
    ///
    /// Internal rather than private for exactly one other caller: RootView
    /// calls it once at launch, because until something saves, this app has
    /// not touched the icon at all and it is still showing whatever the last
    /// push left there — or, on the Mac, nothing.
    func refreshUnreadBadge() {
        UnreadBadge.show(storedUnreadTotal())
    }

}

// MARK: - Call signalling

extension ChatSyncCoordinator: CallSignaling {
    /// The socket is private on purpose — nothing else in the app sends a
    /// frame — so the call machine goes through this one door. Throws when
    /// the socket is not connected: a call cannot be placed over REST, and
    /// the manager shows that as a failure rather than waiting.
    func sendCallFrame(_ frame: ClientFrame) async throws {
        try await socket.send(frame)
    }
}
