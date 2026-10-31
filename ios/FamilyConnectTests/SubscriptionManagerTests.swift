//
//  SubscriptionManagerTests.swift
//  FamilyConnectTests
//
//  Selling the family subscription (docs/protocol.md, "Paid families",
//  2026-10-30), against a FAKE App Store: when the manager calls StoreKit at
//  all, what it hands `PUT /me/subscription`, and what each answer does to
//  the transaction — finished once the server took it or refused it for
//  good, left for StoreKit to deliver again otherwise.
//
//  The real store is StoreKit 2 (`StoreKitSubscriptionStore`). Its
//  purchase-to-PUT path is not run here under `SKTestSession`: this tree's
//  simulator could not run tests at all when it was written (see the #89
//  report), and an SKTestSession test nobody has seen pass is a CI failure
//  waiting to happen. The decisions it feeds are all here.
//

import Foundation
import Testing
@testable import FamilyConnect

/// The App Store, as a recording. Thread-safe because the manager's update
/// listener and the test both reach it.
private final class FakeStore: SubscriptionStore, @unchecked Sendable {
    private let lock = NSLock()
    private var _entitlements: [StoreEntitlement] = []
    private var _calls: [String] = []
    private var _finished: [UInt64] = []
    private var _syncFails = false
    private var continuation: AsyncStream<StoreEntitlement>.Continuation?
    private var _streamEnded = false

    var entitlements: [StoreEntitlement] {
        get { lock.withLock { _entitlements } }
        set { lock.withLock { _entitlements = newValue } }
    }
    var syncFails: Bool {
        get { lock.withLock { _syncFails } }
        set { lock.withLock { _syncFails = newValue } }
    }
    /// Every StoreKit call, in order: "newest", "updates", "sync", "finish".
    var calls: [String] { lock.withLock { _calls } }
    var finished: [UInt64] { lock.withLock { _finished } }
    var streamEnded: Bool { lock.withLock { _streamEnded } }

    func newestEntitlement(among productIDs: Set<String>) async -> StoreEntitlement? {
        lock.withLock {
            _calls.append("newest")
            return _entitlements.filter { productIDs.contains($0.productID) }
                .max { $0.purchaseDate < $1.purchaseDate }
        }
    }

    func updates() -> AsyncStream<StoreEntitlement> {
        lock.withLock { _calls.append("updates") }
        return AsyncStream { continuation in
            self.lock.withLock { self.continuation = continuation }
            continuation.onTermination = { [weak self] _ in
                self?.lock.withLock { self?._streamEnded = true }
            }
        }
    }

    func sync() async throws {
        let fails = lock.withLock {
            _calls.append("sync")
            return _syncFails
        }
        if fails { throw CancellationError() }
    }

    func finish(transactionID: UInt64) async {
        lock.withLock {
            _calls.append("finish")
            _finished.append(transactionID)
        }
    }

    /// `Transaction.updates` delivering one.
    func emit(_ entitlement: StoreEntitlement) {
        let continuation = lock.withLock { self.continuation }
        continuation?.yield(entitlement)
    }
}

/// What the server is asked and answers.
@MainActor
private final class Server {
    var answer: Result<SubscriptionDTO?, Error> = .success(nil)
    var sent: [(transaction: String, renewal: String?)] = []
    var applied: [SubscriptionDTO?] = []
    var refreshes = 0
    var unauthorized = 0
    var remembered: [[String]?] = []
}

@MainActor
@Suite("Paid families: the subscription manager")
struct SubscriptionManagerTests {

    nonisolated private static let monthly = "me.nettrash.FamilyConnect.family.monthly"
    nonisolated private static let yearly = "me.nettrash.FamilyConnect.family.yearly"
    nonisolated private static let token = "6F1C2D3E-4A5B-4C6D-8E7F-A1B2C3D4E5F6"
    nonisolated private static let active = SubscriptionDTO(
        productID: yearly, environment: "Sandbox",
        expiresAt: Date(timeIntervalSince1970: 1_800_000_000), active: true, renews: true)

    private static func entitlement(
        _ id: UInt64, product: String = monthly, renewal: String? = "renewal-jws", at seconds: TimeInterval = 0
    ) -> StoreEntitlement {
        StoreEntitlement(
            transactionID: id, productID: product, transaction: "transaction-jws-\(id)",
            renewal: renewal, purchaseDate: Date(timeIntervalSince1970: 1_700_000_000 + seconds))
    }

    private func make(_ store: FakeStore = FakeStore()) -> (SubscriptionManager, FakeStore, Server) {
        let server = Server()
        let manager = SubscriptionManager(store: store)
        manager.send = { transaction, renewal in
            server.sent.append((transaction, renewal))
            return try server.answer.get()
        }
        manager.apply = { server.applied.append($0) }
        manager.refresh = { server.refreshes += 1 }
        manager.onUnauthorized = { server.unauthorized += 1 }
        manager.remember = { server.remembered.append($0) }
        return (manager, store, server)
    }

    /// Wait for the listener's work, which runs on its own task.
    private func eventually(_ condition: () -> Bool) async {
        for _ in 0..<200 where !condition() {
            try? await Task.sleep(for: .milliseconds(10))
        }
    }

    // MARK: - A server that does not charge

    @Test("subscriptions off: not one StoreKit call, nothing sent, nothing remembered")
    func offMeansNoStoreKit() async {
        let (manager, store, server) = make()
        store.entitlements = [Self.entitlement(1)]
        manager.configure(enabled: false, productIDs: [Self.monthly], appAccountToken: Self.token)
        _ = await manager.launchSend?.value
        #expect(store.calls.isEmpty)
        #expect(server.sent.isEmpty)
        #expect(!manager.isListening)
        #expect(!manager.isEnabled)
        #expect(manager.appAccountToken == nil)
        #expect(server.remembered.count == 1 && server.remembered[0] == nil)
    }

    @Test("a launch with nothing remembered makes no StoreKit call either")
    func nothingRememberedNoStoreKit() {
        let (manager, store, _) = make()
        manager.remembered = { nil }
        manager.startRemembered()
        #expect(store.calls.isEmpty)
        #expect(!manager.isListening)
    }

    // MARK: - Launch and login

    @Test("subscriptions on: listens, and sends the newest entitlement once, finishing it")
    func launchSend() async {
        let (manager, store, server) = make()
        store.entitlements = [
            Self.entitlement(1, at: 0), Self.entitlement(2, product: Self.yearly, at: 100),
            Self.entitlement(3, product: "some.other.app", at: 200),
        ]
        server.answer = .success(Self.active)
        manager.configure(enabled: true, productIDs: [Self.monthly, Self.yearly], appAccountToken: Self.token)
        let outcome = await manager.launchSend?.value
        #expect(outcome == .accepted(Self.active))
        #expect(manager.isListening)
        #expect(manager.appAccountToken == UUID(uuidString: Self.token))
        #expect(server.sent.count == 1)
        #expect(server.sent.first?.transaction == "transaction-jws-2", "the newest among the server's products")
        #expect(server.sent.first?.renewal == "renewal-jws")
        #expect(server.applied == [Self.active])
        #expect(store.finished == [2])
        #expect(server.refreshes == 1, "an accepted send is followed by GET /me")
        #expect(server.remembered.last == [Self.monthly, Self.yearly])

        // Every /me reaches configure — a resync's, the refresh's — and the
        // send is once per session.
        manager.configure(enabled: true, productIDs: [Self.monthly, Self.yearly], appAccountToken: Self.token)
        _ = await manager.launchSend?.value
        #expect(server.sent.count == 1)
        #expect(store.calls.filter { $0 == "updates" }.count == 1, "one listener, not one per /me")
    }

    @Test("a sign-out stops the listener, and the next sign-in sends again")
    func resetSendsAgain() async {
        let (manager, store, server) = make()
        store.entitlements = [Self.entitlement(1)]
        server.answer = .success(Self.active)
        manager.configure(enabled: true, productIDs: [Self.monthly], appAccountToken: Self.token)
        _ = await manager.launchSend?.value
        manager.reset()
        #expect(!manager.isListening)
        #expect(!manager.isEnabled)
        await eventually { store.streamEnded }
        #expect(store.streamEnded)

        manager.configure(enabled: true, productIDs: [Self.monthly], appAccountToken: Self.token)
        _ = await manager.launchSend?.value
        #expect(server.sent.count == 2)
    }

    @Test("no entitlement: nothing is sent and nothing finished")
    func noEntitlement() async {
        let (manager, store, server) = make()
        manager.configure(enabled: true, productIDs: [Self.monthly], appAccountToken: nil)
        #expect(await manager.launchSend?.value == .nothing)
        #expect(server.sent.isEmpty)
        #expect(store.finished.isEmpty)
    }

    @Test("a server that stops charging stops the listener")
    func turnedOff() async {
        let (manager, store, _) = make()
        manager.configure(enabled: true, productIDs: [Self.monthly], appAccountToken: nil)
        _ = await manager.launchSend?.value
        #expect(manager.isListening)
        manager.configure(enabled: false, productIDs: [], appAccountToken: nil)
        #expect(!manager.isListening)
        await eventually { store.streamEnded }
        #expect(store.streamEnded)
    }

    @Test("a launch where this server charged last time listens before /me answers, and sends nothing yet")
    func rememberedListens() {
        let (manager, store, server) = make()
        manager.remembered = { [Self.monthly] }
        manager.startRemembered()
        #expect(manager.isListening)
        #expect(store.calls == ["updates"])
        #expect(server.sent.isEmpty)
    }

    // MARK: - What each answer does to the transaction

    @Test("subscription_invalid and subscription_taken are terminal: finished, not sent again")
    func terminalRefusals() async {
        for code in ["subscription_invalid", "subscription_taken"] {
            let (manager, store, server) = make()
            server.answer = .failure(APIError.conflict(code: code, message: nil))
            let outcome = await manager.present(Self.entitlement(7))
            #expect(outcome == .refused(code: code))
            #expect(store.finished == [7])
            #expect(server.refreshes == 0)
            #expect(server.applied.isEmpty)
        }
    }

    @Test("a transient failure leaves the transaction for StoreKit to deliver again")
    func transientLeavesUnfinished() async {
        let failures: [APIError] = [
            .transport(URLError(.notConnectedToInternet)), .server(status: 503, message: nil),
            .throttled(retryAfter: 2),
        ]
        for failure in failures {
            let (manager, store, server) = make()
            server.answer = .failure(failure)
            #expect(await manager.present(Self.entitlement(8)) == .transient)
            #expect(store.finished.isEmpty)
            #expect(server.refreshes == 0)
            #expect(server.applied.isEmpty)
        }
    }

    @Test("any other refusal leaves it unfinished too; a 401 ends the session")
    func otherRefusals() async {
        let (manager, store, server) = make()
        server.answer = .failure(APIError.forbidden(code: "subscriptions_disabled"))
        #expect(await manager.present(Self.entitlement(9)) == .failed)
        #expect(store.finished.isEmpty)

        server.answer = .failure(APIError.unauthorized)
        #expect(await manager.present(Self.entitlement(9)) == .failed)
        #expect(store.finished.isEmpty)
        #expect(server.unauthorized == 1)
    }

    @Test("a renewal JWS StoreKit could not give is left out, not invented")
    func noRenewal() async {
        let (manager, _, server) = make()
        server.answer = .success(Self.active)
        await manager.present(Self.entitlement(10, renewal: nil))
        #expect(server.sent.count == 1)
        #expect(server.sent.first?.renewal == nil)
    }

    // MARK: - Transaction.updates

    @Test("every update for the server's products is sent; others are left alone")
    func updatesAreSent() async {
        let (manager, store, server) = make()
        server.answer = .success(Self.active)
        manager.configure(enabled: true, productIDs: [Self.monthly, Self.yearly], appAccountToken: nil)
        _ = await manager.launchSend?.value

        store.emit(Self.entitlement(20, product: "some.other.app"))
        store.emit(Self.entitlement(21, product: Self.yearly))
        await eventually { store.finished.contains(21) }
        #expect(server.sent.map(\.transaction) == ["transaction-jws-21"])
        #expect(store.finished == [21])
        #expect(server.refreshes == 1)
    }

    // MARK: - Restore Purchases

    @Test("Restore syncs with the App Store, then sends the newest entitlement")
    func restore() async {
        let (manager, store, server) = make()
        manager.configure(enabled: true, productIDs: [Self.monthly], appAccountToken: nil)
        _ = await manager.launchSend?.value
        store.entitlements = [Self.entitlement(30)]
        server.answer = .success(Self.active)
        #expect(await manager.restore() == .accepted(Self.active))
        #expect(Array(store.calls.suffix(3)) == ["sync", "newest", "finish"])
        #expect(server.sent.map(\.transaction) == ["transaction-jws-30"])
    }

    @Test("a sync that fails still sends what the device holds; nothing held is said")
    func restoreAfterFailedSync() async {
        let (manager, store, server) = make()
        manager.configure(enabled: true, productIDs: [Self.monthly], appAccountToken: nil)
        _ = await manager.launchSend?.value
        store.syncFails = true
        #expect(await manager.restore() == .nothing)
        store.entitlements = [Self.entitlement(31)]
        server.answer = .success(Self.active)
        #expect(await manager.restore() == .accepted(Self.active))
        #expect(store.finished == [31])
    }
}
