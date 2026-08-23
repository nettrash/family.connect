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
