//
//  AttachmentRetryTests.swift
//  FamilyConnectTests
//
//  A photo whose fetch happened to land during an outage used to stay a
//  spinner for the rest of the process: the store folded a 404 and a
//  timeout into the same "missing", and the gate that reads it sits in
//  front of the disk cache as well as the fetch, so nothing short of a
//  logout brought the picture back.
//
//  These pin the distinction. The failing direction is the one that
//  matters — a transport failure must leave the key retryable — so the
//  first test here fails against the old implementation.
//

import Foundation
import Testing
@testable import FamilyConnect

@MainActor
struct AttachmentRetryTests {

    private func makeStore(host: String, session: URLSession) throws -> AttachmentStore {
        // No token is set: the stub answers on the URL alone, and the
        // store's behaviour under test is about errors, not auth.
        let api = APIClient(serverURL: URL(string: "https://\(host)"), session: session)
        let directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("attachment-retry-\(UUID().uuidString)")
        return AttachmentStore(api: api, directory: directory)
    }

    private func waitUntil(_ condition: @MainActor () -> Bool, timeout: TimeInterval = 5) async {
        let deadline = Date().addingTimeInterval(timeout)
        while !condition() && Date() < deadline {
            try? await Task.sleep(for: .milliseconds(20))
        }
    }

    /// THE REGRESSION. One 5xx while the phone is on a bad connection, and
    /// the photo must still arrive once the server answers.
    @Test("a transport failure leaves the photo retryable")
    func transportFailureDoesNotSettle() async throws {
        let host = "attachment-retry-transport.test"
        StubURLProtocol.register(host: host) { _ in
            // APIClient retries a transient GET once inside `perform`, so a
            // single 503 never reaches the store at all — the first FETCH is
            // two requests. Fail both, then answer, or this tests the
            // client's retry instead of the store's bookkeeping.
            let earlier = StubURLProtocol.requests(host: host).count
            if earlier <= 2 {
                return StubResponse(status: 503, headers: ["Retry-After": "0"], body: Data())
            }
            return StubResponse(status: 200, headers: ["Content-Type": "image/jpeg"],
                                body: TestImages.photograph(width: 32, height: 32))
        }
        defer { StubURLProtocol.unregister(host: host) }

        let store = try makeStore(host: host, session: StubURLProtocol.makeSession())

        // First look: nothing yet, and a fetch that is about to fail.
        #expect(store.image(id: 1, preview: true) == nil)
        // Both attempts of the first fetch must land before we judge it.
        await waitUntil { StubURLProtocol.requests(host: host).count >= 2 }
        await waitUntil { store.image(id: 1, preview: true) == nil }

        // Second look: the key must NOT have been written off, so this
        // starts another fetch rather than short-circuiting forever.
        _ = store.image(id: 1, preview: true)
        await waitUntil { StubURLProtocol.requests(host: host).count >= 3 }
        await waitUntil { store.image(id: 1, preview: true) != nil }

        #expect(store.image(id: 1, preview: true) != nil,
                "a photo that failed once on transport never came back")
    }

    /// The other half: a genuine 404 IS final, or every render pass would
    /// re-ask the server for a preview that does not exist.
    @Test("a 404 settles the key and is not re-requested")
    func notFoundSettles() async throws {
        let host = "attachment-retry-404.test"
        StubURLProtocol.register(host: host) { _ in .empty(404) }
        defer { StubURLProtocol.unregister(host: host) }

        let store = try makeStore(host: host, session: StubURLProtocol.makeSession())

        #expect(store.image(id: 2, preview: true) == nil)
        await waitUntil { StubURLProtocol.requests(host: host).count >= 1 }
        let afterFirst = StubURLProtocol.requests(host: host).count

        // Several more render passes must not produce more requests.
        for _ in 0..<5 { _ = store.image(id: 2, preview: true) }
        try? await Task.sleep(for: .milliseconds(100))
        #expect(StubURLProtocol.requests(host: host).count == afterFirst,
                "a settled 404 was re-requested on every render pass")
    }

    /// Reconnecting is what un-sticks a key: the store has no network
    /// monitor of its own, so the socket coming back is the signal.
    @Test("a reconnect forgets earlier misses and lets views ask again")
    func reconnectClearsMisses() async throws {
        let host = "attachment-retry-reconnect.test"
        StubURLProtocol.register(host: host) { _ in
            let earlier = StubURLProtocol.requests(host: host).count
            if earlier <= 1 { return .empty(404) }
            return StubResponse(status: 200, headers: ["Content-Type": "image/jpeg"],
                                body: TestImages.photograph(width: 32, height: 32))
        }
        defer { StubURLProtocol.unregister(host: host) }

        let store = try makeStore(host: host, session: StubURLProtocol.makeSession())

        #expect(store.image(id: 3, preview: true) == nil)
        await waitUntil { StubURLProtocol.requests(host: host).count >= 1 }
        // Settled: further looks ask nothing.
        for _ in 0..<3 { _ = store.image(id: 3, preview: true) }
        let beforeReconnect = StubURLProtocol.requests(host: host).count

        let generationBefore = store.generation
        store.retryAfterReconnect()
        #expect(store.generation != generationBefore,
                "views were never told to redraw, so nothing would re-ask")

        _ = store.image(id: 3, preview: true)
        await waitUntil { StubURLProtocol.requests(host: host).count > beforeReconnect }
        await waitUntil { store.image(id: 3, preview: true) != nil }
        #expect(store.image(id: 3, preview: true) != nil)
    }

    /// A no-op reconnect must not churn `generation`: every bump redraws
    /// every view holding a photo.
    @Test("a reconnect with nothing to forget does not redraw")
    func reconnectWithNothingMissingIsQuiet() async throws {
        let host = "attachment-retry-quiet.test"
        StubURLProtocol.register(host: host) { _ in .empty(404) }
        defer { StubURLProtocol.unregister(host: host) }

        let store = try makeStore(host: host, session: StubURLProtocol.makeSession())
        let before = store.generation
        store.retryAfterReconnect()
        #expect(store.generation == before)
    }
}
