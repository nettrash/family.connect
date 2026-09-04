//
//  ChatSocket.swift
//  FamilyConnect
//
//  The WebSocket wire: owns the URLSessionWebSocketTask, the reconnect
//  loop, and the application-level heartbeat, and surfaces everything to
//  the coordinator as one AsyncStream of events. Deliberately DUMB about
//  content: frames pass through undecoded-then-decoded, nothing here
//  touches SwiftData or knows what an "ack" means — that separation is
//  what lets the coordinator's dedup matrix be tested without a network.
//
//  Lifecycle contract:
//    - `start(url:token:)` returns the event stream and begins connecting.
//      Each successful upgrade yields `.connected`; every drop yields
//      `.disconnected` and the loop sleeps a full-jitter backoff before
//      trying again (protocol.md: the socket is a live wire, not a queue —
//      the coordinator resyncs over REST on every `.connected`).
//    - `suspend()` (app → background) tears the connection down but keeps
//      the stream alive; `resume()` restarts the connect loop on the same
//      stream. `stop()` finishes the stream for good.
//    - A single undecodable frame is logged and skipped — one garbled
//      message must not kill a healthy connection.
//
//  Heartbeat: the server pings at the WS protocol level every 30 s, but
//  URLSession surfaces no protocol pings to us, so per protocol.md we send
//  the application-level {"type":"ping"} every 25 s and treat a missing
//  pong (no traffic for 75 s, the server's own idle-drop horizon) as a
//  dead connection — cancel the task and let the reconnect loop take over.
//
//  An actor: all mutable state (task, continuation, backoff, suspended)
//  is confined here. Nested types are `nonisolated` — under this target's
//  SWIFT_DEFAULT_ACTOR_ISOLATION=MainActor they would otherwise be
//  captured by the main actor, and they are pure values.
//

import Foundation
import os

nonisolated enum SocketEvent: Sendable {
    case connected
    case disconnected
    /// The session this socket authenticates with is GONE — the upgrade
    /// was refused with `401`, or the server closed the connection with
    /// `4401` (protocol.md, "WebSocket protocol"). Reconnecting cannot
    /// help: the token is dead, and the same token is all this socket has.
    /// The coordinator turns it into the ordinary 401 handling, which is
    /// what returns the app to the sign-in screen — the case that makes
    /// this exist is an account deleted from another device, where a Mac
    /// holding its socket open all day would otherwise sit at
    /// "Connecting…" forever, never making a REST call to be refused.
    case unauthorized
    case frame(ServerFrame)
}

nonisolated enum SocketError: Error, Equatable {
    /// `send` was called with no live, handshaken connection. Callers
    /// (the send pipeline) treat this as "fall back to REST now".
    case notConnected
}

actor ChatSocket {

    /// Application-level ping cadence (protocol.md recommends ~25 s).
    private let pingInterval: TimeInterval = 25
    /// No pong/traffic for this long ⇒ the connection is dead.
    private let pongTimeout: TimeInterval = 75

    private let session: URLSession
    private var url: URL?
    private var token: String?

    private var task: URLSessionWebSocketTask?
    private var runner: Task<Void, Never>?
    private var heartbeat: Task<Void, Never>?
    private var continuation: AsyncStream<SocketEvent>.Continuation?
    private var backoff: ReconnectBackoff
    private var suspended = false
    private var isConnected = false
    /// When the current connection's handshake completed, or nil when there
    /// is none. Read once at teardown to decide whether it earned a reset.
    private var connectedAt: Date?

    /// How long a connection must last before its next drop is treated as
    /// bad luck rather than a broken endpoint.
    ///
    /// Ten seconds: long enough that an accept-then-close cannot reach it
    /// (those return in milliseconds), short enough that a genuine
    /// connection on a slow network still earns its reset well inside one
    /// heartbeat interval.
    static let durableAfter: TimeInterval = 10
    /// Last time the connection proved alive (pong or any inbound frame).
    private var lastAliveAt = Date.distantPast

    /// `session` injected as the only test seam, like APIClient.
    init(session: URLSession = .shared, backoff: ReconnectBackoff = ReconnectBackoff()) {
        self.session = session
        self.backoff = backoff
    }

    // MARK: - Lifecycle

    /// Begin connecting and return the event stream. Calling again tears
    /// down any previous stream first (server change, re-login).
    func start(url: URL, token: String) -> AsyncStream<SocketEvent> {
        stop()
        self.url = url
        self.token = token
        suspended = false
        backoff.reset()
        let (stream, continuation) = AsyncStream<SocketEvent>.makeStream()
        self.continuation = continuation
        runner = Task { await self.runLoop() }
        return stream
    }

    /// Tear down the connection AND the stream. After this, `start` again.
    func stop() {
        runner?.cancel()
        runner = nil
        teardownConnection()
        continuation?.finish()
        continuation = nil
    }

    /// Background: drop the connection (iOS will kill it anyway) but keep
    /// the stream so the coordinator's consumer task survives.
    func suspend() {
        guard !suspended else { return }
        suspended = true
        runner?.cancel()
        runner = nil
        teardownConnection()
    }

    /// What `resume()` found when the app came back to the foreground.
    ///
    /// The caller needs this to decide what to show. Claiming "Connecting…"
    /// for a socket that was never suspended is the bug this exists to
    /// prevent: `resume()` correctly does nothing in that case, so nothing
    /// would ever have set the banner back to connected.
    enum ResumeOutcome {
        /// A connect loop is now running; the banner should say connecting.
        case reconnecting
        /// The socket never went down — it is live right now.
        case alreadyLive
        /// Never started, or already torn down for good.
        case notStarted
    }

    /// Foreground: restart the connect loop on the surviving stream.
    @discardableResult
    func resume() -> ResumeOutcome {
        guard continuation != nil, url != nil else { return .notStarted }
        guard suspended else {
            // NOT an error, and the `guard` above must stay: resuming a live
            // socket would fork a second runLoop onto the same stream and
            // double-consume it. It just means there is nothing to do.
            return isConnected ? .alreadyLive : .reconnecting
        }
        suspended = false
        backoff.reset()
        runner = Task { await self.runLoop() }
        return .reconnecting
    }

    /// Send one frame. Throws `SocketError.notConnected` when there is no
    /// live handshaken connection — the caller falls back to REST.
    ///
    /// The write has a DEADLINE, and it needs one: a socket whose TCP
    /// connection has gone away without a FIN absorbs writes into a buffer
    /// that will never drain, and the only thing that eventually notices is
    /// the 75 s pong horizon. That is far longer than a person will watch a
    /// spinner, and the REST leg waiting behind this call would have
    /// delivered the message in a second (docs/protocol.md, "Sending on an
    /// unreliable network": the frame write gets the ack deadline too).
    func send(_ frame: ClientFrame) async throws {
        guard isConnected, let task else { throw SocketError.notConnected }
        let text = try frame.encodedString()
        try await withThrowingTaskGroup(of: Void.self) { group in
            group.addTask { try await task.send(.string(text)) }
            group.addTask {
                try await Task.sleep(nanoseconds: UInt64(Self.writeTimeout * 1_000_000_000))
                throw SocketError.notConnected
            }
            // Whichever finishes first decides; the other is cancelled.
            defer { group.cancelAll() }
            _ = try await group.next()
        }
    }

    /// How long a frame write may take before the caller falls to REST.
    /// Matches the coordinator's ack deadline: the protocol gives the
    /// write and the answer the same 10 s.
    private static let writeTimeout: TimeInterval = 10

    /// The network came back — dial NOW rather than finishing a sleep that
    /// was sized for a network that no longer exists.
    ///
    /// A full-jitter ceiling of 30 s is right for a server that went away
    /// and wrong for a phone that has just left a tunnel: the route is
    /// back, and the app would otherwise sit at "Connecting…" for another
    /// half minute with messages queued behind it. Cancelling the runner
    /// interrupts its `Task.sleep`; the loop is written to re-dial, so
    /// restarting it is the whole kick.
    func kick() {
        guard continuation != nil, url != nil, !suspended else { return }
        runner?.cancel()
        backoff.reset()
        runner = Task { await self.runLoop() }
    }

    // MARK: - Connect / reconnect loop

    private func runLoop() async {
        while !Task.isCancelled && !suspended {
            guard let url, let token, let continuation else { return }

            var request = URLRequest(url: url, timeoutInterval: 15)
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
            let task = session.webSocketTask(with: request)
            self.task = task
            task.resume()

            do {
                // Handshake probe: the first send only succeeds once the
                // upgrade completed, so a successful ping IS "connected".
                try await task.send(.string(ClientFrame.ping.encodedString()))
                isConnected = true
                lastAliveAt = Date()
                // NOT `backoff.reset()`. A completed handshake proves the
                // upgrade happened, not that the connection is usable: a
                // proxy — or this app's own server, which kicks a connection
                // whose send queue overflows with code 1001 — can accept and
                // drop immediately. Resetting here made every such cycle
                // start again from random(0…1)s, so the ceiling never grew
                // and the socket reconnected about twice a second forever,
                // each one firing a full resync. The backoff is forgiven at
                // teardown instead, and only if the connection LASTED.
                connectedAt = Date()
                continuation.yield(.connected)
                AppLog.socket.info("Socket connected")
                startHeartbeat(task)
                try await receiveLoop(task, continuation: continuation)
            } catch {
                AppLog.socket.info("Socket dropped: \(String(describing: error))")
            }

            stopHeartbeat()
            // Read BEFORE cancelling: cancel(with:) overwrites closeCode.
            let sessionIsGone = Self.isUnauthorized(task)
            task.cancel(with: .goingAway, reason: nil)
            self.task = nil
            let wasConnected = isConnected
            isConnected = false
            // Durability, judged only now that the connection is over. A
            // connection that carried traffic for a while was real, so the
            // next drop starts cheap; one that died on arrival leaves the
            // ceiling where it was, and repeated arrivals-and-deaths climb
            // it towards the 30s cap the way an unreachable server does.
            if ReconnectBackoff.earnsReset(
                connectedAt: connectedAt, durableAfter: Self.durableAfter) {
                backoff.reset()
            }
            connectedAt = nil
            if wasConnected { continuation.yield(.disconnected) }

            if sessionIsGone {
                AppLog.socket.info("Socket refused: the session is gone")
                continuation.yield(.unauthorized)
                // No backoff and no retry: every attempt would carry the
                // same dead token. The coordinator tears this socket down
                // as part of returning to the sign-in screen.
                break
            }

            guard !Task.isCancelled && !suspended else { break }
            let delay = backoff.nextDelay()
            AppLog.socket.info("Reconnecting in \(delay, format: .fixed(precision: 1), privacy: .public)s")
            try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
        }
    }

    /// Whether the connection ended because the SESSION is gone rather
    /// than because the network is.
    ///
    /// Two shapes of the same fact: a socket that was already open is
    /// closed by the server with `4401`, and a socket that has yet to open
    /// has its upgrade refused with HTTP `401`. Both are checked because
    /// both happen in the one case this matters for — a deleted account or
    /// a revoked session closes the live socket, and the reconnect that
    /// follows is the attempt that gets the 401.
    private static func isUnauthorized(_ task: URLSessionWebSocketTask) -> Bool {
        if let http = task.response as? HTTPURLResponse, http.statusCode == 401 { return true }
        return task.closeCode.rawValue == 4401
    }

    private func receiveLoop(
        _ task: URLSessionWebSocketTask,
        continuation: AsyncStream<SocketEvent>.Continuation
    ) async throws {
        while !Task.isCancelled {
            let message = try await task.receive()
            let data: Data?
            switch message {
            case .string(let text): data = text.data(using: .utf8)
            case .data(let raw): data = raw
            @unknown default: data = nil
            }
            guard let data else { continue }

            do {
                let frame = try APICoding.decoder().decode(ServerFrame.self, from: data)
                // Any decodable inbound frame proves the wire is alive; the
                // pong just happens to be the one we solicit.
                lastAliveAt = Date()
                continuation.yield(.frame(frame))
            } catch {
                // One garbled frame must not kill the connection.
                AppLog.socket.error("Undecodable frame skipped: \(String(describing: error))")
            }
        }
    }

    // MARK: - Heartbeat

    private func startHeartbeat(_ task: URLSessionWebSocketTask) {
        heartbeat = Task { await self.heartbeatLoop(task) }
    }

    private func stopHeartbeat() {
        heartbeat?.cancel()
        heartbeat = nil
    }

    private func heartbeatLoop(_ task: URLSessionWebSocketTask) async {
        while !Task.isCancelled {
            try? await Task.sleep(nanoseconds: UInt64(pingInterval * 1_000_000_000))
            guard !Task.isCancelled else { break }
            try? await task.send(.string((try? ClientFrame.ping.encodedString()) ?? #"{"type":"ping"}"#))
            if Date().timeIntervalSince(lastAliveAt) > pongTimeout {
                AppLog.socket.info("No pong within \(self.pongTimeout, privacy: .public)s — declaring the connection dead")
                // Cancelling makes receive() throw, which trips the
                // reconnect loop; nothing else to clean up here.
                task.cancel(with: .goingAway, reason: nil)
                break
            }
        }
    }

    private func teardownConnection() {
        stopHeartbeat()
        task?.cancel(with: .goingAway, reason: nil)
        task = nil
        let wasConnected = isConnected
        isConnected = false
        if wasConnected { continuation?.yield(.disconnected) }
    }
}
