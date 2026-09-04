//
//  ConnectionStateTests.swift
//  FamilyConnectTests
//
//  The banner's state machine, which had no coverage at all until it
//  produced a user-visible bug: "Connecting…" showing indefinitely while
//  every message went through.
//
//  The cause was an assumption rather than a question. `resumeForeground()`
//  set `.connecting` unconditionally, but `ChatSocket.resume()` is a no-op on
//  a socket that was never suspended — and nothing else re-emits
//  `.connected`, because the server sends no hello frame: a successful HTTP
//  upgrade is the only "you are connected" signal that exists. So the state
//  had no way back.
//
//  Two rules are pinned here, and the second is what makes the whole class of
//  bug self-healing: ask the socket what actually happened, and treat any
//  inbound frame as proof the wire is alive.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Connection state")
struct ConnectionStateTests {

    @MainActor
    private struct Harness {
        let coordinator: ChatSyncCoordinator
        let host: String

        func tearDown() { StubURLProtocol.unregister(host: host) }
    }

    private func makeHarness(host: String) throws -> Harness {
        StubURLProtocol.register(host: host, handler: { _ in .empty(204) })
        let configuration = ModelConfiguration(isStoredInMemoryOnly: true)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self, NoteEntity.self,
            PendingMediaItemEntity.self,
            configurations: configuration)
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        return Harness(coordinator: coordinator, host: host)
    }

    @Test("a frame arriving promotes connecting to connected")
    func frameProvesTheConnection() throws {
        let harness = try makeHarness(host: "frame-promotes.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        coordinator.overrideConnectionState(.connecting)
        // A pong is the emptiest frame there is; even that is proof.
        coordinator.handle(event: .frame(.pong))

        #expect(coordinator.connectionState == .connected)
    }

    @Test("a deliberate suspension is not undone by a straggler frame")
    func offlineSurvivesALateFrame() throws {
        let harness = try makeHarness(host: "offline-stays.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        // `.offline` means the app put the socket down on purpose. A frame
        // decoded before teardown can still be sitting in the stream's
        // (unbounded) buffer, and must not resurrect the banner.
        coordinator.overrideConnectionState(.offline)
        coordinator.handle(event: .frame(.pong))

        #expect(coordinator.connectionState == .offline)
    }

    @Test("connected stays connected")
    func connectedIsIdempotent() throws {
        let harness = try makeHarness(host: "connected-idempotent.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        coordinator.overrideConnectionState(.connected)
        coordinator.handle(event: .frame(.pong))

        #expect(coordinator.connectionState == .connected)
    }

    @Test("a socket that was never suspended reports itself already live")
    func resumeOnALiveSocketIsNotAReconnect() async {
        let socket = ChatSocket()
        // Never started: there is nothing to resume and nothing to claim.
        #expect(await socket.resume() == .notStarted)
    }

    @Test("disconnect while connected shows connecting, not offline")
    func dropGoesToConnecting() throws {
        let harness = try makeHarness(host: "drop-shows-connecting.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        coordinator.overrideConnectionState(.connected)
        coordinator.handle(event: .disconnected)

        // The socket's own loop is retrying — "offline" is reserved for a
        // deliberate suspension, so the banner must say connecting.
        #expect(coordinator.connectionState == .connecting)
    }
}

@MainActor
@Suite("Network reachability")
struct NetworkReachabilityTests {

    /// The one signal this app did not have: a route appearing where
    /// there was none. It fires on the TRANSITION only — a path that was
    /// already satisfied is not news, and treating it as news would make
    /// every interface update a reconnect.
    @Test("restored fires only on the unsatisfied → satisfied edge")
    func firesOnTheEdge() {
        let reachability = NetworkReachability()
        var restored = 0
        reachability.onRestored = { restored += 1 }

        reachability.apply(satisfied: true)
        #expect(restored == 0, "already online is not an event")

        reachability.apply(satisfied: false)
        #expect(restored == 0)

        let comeBack = Date()
        reachability.apply(satisfied: true, now: comeBack)
        #expect(restored == 1)
        #expect(reachability.isOnline)
    }

    /// Switching from cellular to Wi-Fi fires several satisfied paths in
    /// a second; undebounced, that is its own little reconnect storm.
    @Test("bursts of path updates are debounced into one restore")
    func debouncesBursts() {
        let reachability = NetworkReachability(debounce: 2)
        var restored = 0
        reachability.onRestored = { restored += 1 }
        let start = Date()

        reachability.apply(satisfied: false, now: start)
        reachability.apply(satisfied: true, now: start)
        reachability.apply(satisfied: false, now: start.addingTimeInterval(0.2))
        reachability.apply(satisfied: true, now: start.addingTimeInterval(0.4))
        #expect(restored == 1, "the second flap is inside the debounce window")

        reachability.apply(satisfied: false, now: start.addingTimeInterval(5))
        reachability.apply(satisfied: true, now: start.addingTimeInterval(6))
        #expect(restored == 2, "a genuinely later restore still counts")
    }
}
