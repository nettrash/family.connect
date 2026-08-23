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
    func send(_ frame: ClientFrame) async throws {
        guard isConnected, let task else { throw SocketError.notConnected }
        try await task.send(.string(frame.encodedString()))
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
                backoff.reset()
                continuation.yield(.connected)
                AppLog.socket.info("Socket connected")
                startHeartbeat(task)
                try await receiveLoop(task, continuation: continuation)
            } catch {
                AppLog.socket.info("Socket dropped: \(String(describing: error))")
            }

            stopHeartbeat()
            task.cancel(with: .goingAway, reason: nil)
            self.task = nil
            let wasConnected = isConnected
            isConnected = false
            if wasConnected { continuation.yield(.disconnected) }

            guard !Task.isCancelled && !suspended else { break }
            let delay = backoff.nextDelay()
            AppLog.socket.info("Reconnecting in \(delay, format: .fixed(precision: 1), privacy: .public)s")
            try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
        }
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
