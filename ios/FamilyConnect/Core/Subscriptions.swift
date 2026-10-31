//
//  Subscriptions.swift
//  FamilyConnect
//
//  Selling the family subscription and telling the server about it
//  (docs/protocol.md, "Paid families", 2026-10-30). The App Store sells; the
//  server believes only what Apple signed, so this side's whole job is to
//  hand it the signed values — `PUT /me/subscription {transaction, renewal}`:
//
//    - after a purchase or a restore;
//    - on every launch and login while the server charges, for the newest
//      entitlement among its products;
//    - for every `Transaction.updates` item (a renewal, a refund, an Ask to
//      Buy approved, a purchase on another device).
//
//  …and then to read `GET /me` again. A transaction is FINISHED once the
//  server has accepted it, or has refused it for good (`subscription_invalid`,
//  `subscription_taken`); on a transient failure it is left unfinished, so
//  StoreKit delivers it again.
//
//  StoreKit sits behind `SubscriptionStore`, so every one of those decisions
//  is tested with a fake; `StoreKitSubscriptionStore` is the real one. A
//  server whose `GET /me` does not say `subscriptions_enabled` gets NO
//  StoreKit call at all: nothing here starts until a `/me` — or, at launch,
//  the last `/me` from this server — says it charges.
//
//  Logs name outcomes and codes, never a JWS.
//

import Foundation
import Observation
import os
import StoreKit

/// One App Store entitlement, as the server is to be shown it.
nonisolated struct StoreEntitlement: Equatable, Sendable {
    /// `Transaction.id` — what finishing it names.
    let transactionID: UInt64
    let productID: String
    /// `Transaction.jwsRepresentation`.
    let transaction: String
    /// The subscription status's `RenewalInfo.jwsRepresentation`, when
    /// StoreKit has one.
    let renewal: String?
    /// When this period was bought or renewed — what "newest" goes by.
    let purchaseDate: Date
}

/// The App Store, as far as paid families need it.
nonisolated protocol SubscriptionStore: Sendable {
    /// The newest of `Transaction.currentEntitlements` whose product is one
    /// of `productIDs`, or nil when there is none.
    func newestEntitlement(among productIDs: Set<String>) async -> StoreEntitlement?
    /// `Transaction.updates`, for as long as the stream is iterated.
    func updates() -> AsyncStream<StoreEntitlement>
    /// `AppStore.sync()`: Restore Purchases.
    func sync() async throws
    /// Finish the transaction with this id, if it is still unfinished.
    func finish(transactionID: UInt64) async
}

/// The real store: StoreKit 2.
nonisolated struct StoreKitSubscriptionStore: SubscriptionStore {

    func newestEntitlement(among productIDs: Set<String>) async -> StoreEntitlement? {
        guard !productIDs.isEmpty else { return nil }
        var newest: VerificationResult<Transaction>?
        for await result in Transaction.currentEntitlements {
            // The server is the one that verifies — offline, against
            // Apple's root — so an entitlement StoreKit could not verify is
            // still sent, and the server's verdict decides it.
            let transaction = result.unsafePayloadValue
            guard productIDs.contains(transaction.productID), transaction.revocationDate == nil else { continue }
            if let held = newest, held.unsafePayloadValue.purchaseDate >= transaction.purchaseDate { continue }
            newest = result
        }
        guard let newest else { return nil }
        return await Self.entitlement(from: newest)
    }

    func updates() -> AsyncStream<StoreEntitlement> {
        AsyncStream { continuation in
            let task = Task.detached {
                for await result in Transaction.updates {
                    continuation.yield(await Self.entitlement(from: result))
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    func sync() async throws {
        try await AppStore.sync()
    }

    func finish(transactionID: UInt64) async {
        for await result in Transaction.unfinished where result.unsafePayloadValue.id == transactionID {
            await result.unsafePayloadValue.finish()
        }
    }

    /// A transaction StoreKit handed over — a purchase's, an update's, an
    /// entitlement's — with the renewal info of its subscription, which
    /// tells the server whether it will renew.
    static func entitlement(from result: VerificationResult<Transaction>) async -> StoreEntitlement {
        let transaction = result.unsafePayloadValue
        return StoreEntitlement(
            transactionID: transaction.id,
            productID: transaction.productID,
            transaction: result.jwsRepresentation,
            renewal: await renewalJWS(for: transaction),
            purchaseDate: transaction.purchaseDate)
    }

    /// `product.subscription?.status`'s renewal info for this transaction's
    /// subscription — the status whose transaction shares its original id,
    /// else the first. Nil when StoreKit has none to give (offline, say):
    /// the protocol makes `renewal` optional for exactly that.
    private static func renewalJWS(for transaction: Transaction) async -> String? {
        guard let product = try? await Product.products(for: [transaction.productID]).first,
              let statuses = try? await product.subscription?.status
        else { return nil }
        let status = statuses.first {
            $0.transaction.unsafePayloadValue.originalID == transaction.originalID
        } ?? statuses.first
        return status?.renewalInfo.jwsRepresentation
    }
}

/// The subscription's side of the app: the update listener, the launch
/// send, the purchase and Restore paths, and what each answer does to the
/// transaction. One per process, beside the session.
@MainActor @Observable
final class SubscriptionManager {

    /// What handing one entitlement to the server came to.
    enum Outcome: Equatable, Sendable {
        /// The server took it. The caller's resulting subscription — not
        /// necessarily the one just sent.
        case accepted(SubscriptionDTO?)
        /// `subscription_invalid` or `subscription_taken`: the server's
        /// verdict, terminal. The transaction is finished.
        case refused(code: String)
        /// No answer about it — no network, a 5xx, a throttle. Left
        /// unfinished, so StoreKit delivers it again.
        case transient
        /// The server refused something else (a server that stopped
        /// charging, a session that is gone). Left unfinished too.
        case failed
        /// Nothing to send: no entitlement among the server's products.
        case nothing
    }

    /// The two codes after which a transaction is finished although the
    /// server did not take it: asking again gets the same answer.
    nonisolated static let terminalCodes: Set<String> = ["subscription_invalid", "subscription_taken"]

    private let store: any SubscriptionStore

    /// `PUT /me/subscription`. Closures rather than the API client, for the
    /// reason `AppSession`'s store hooks are closures: the decisions here
    /// are tested without a network.
    var send: (_ transaction: String, _ renewal: String?) async throws -> SubscriptionDTO? = { _, _ in nil }
    /// The server's answer, applied to the session at once.
    var apply: (SubscriptionDTO?) -> Void = { _ in }
    /// `GET /me`, applied — what follows every accepted send.
    var refresh: () async -> Void = {}
    /// A `401`: the session is gone.
    var onUnauthorized: () -> Void = {}
    /// The products the last `/me` from this server sold, kept across
    /// launches so the update listener can start before the first `/me`
    /// answers; nil when that `/me` said the server does not charge.
    var remember: ([String]?) -> Void = { _ in }
    var remembered: () -> [String]? = { nil }

    /// Whether the server charges, as its last `/me` said.
    private(set) var isEnabled = false
    /// The products it sells, in its order.
    private(set) var productIDs: [String] = []
    /// The account's `app_account_token`, which a purchase carries.
    private(set) var appAccountToken: UUID?

    private var listener: Task<Void, Never>?
    /// Whether this launch, or this sign-in, has sent the newest
    /// entitlement yet. Every `/me` reaches `configure`, a resync's
    /// included, and the send is once per launch and per login.
    private var sentThisSession = false
    /// The launch send in flight — what a test waits on.
    private(set) var launchSend: Task<Outcome, Never>?

    init(store: any SubscriptionStore) {
        self.store = store
    }

    /// Whether the update listener is running.
    var isListening: Bool { listener != nil }

    // MARK: - What GET /me says

    /// Adopt what a `/me` says about subscriptions: start the listener and
    /// send the newest entitlement (once per session) when the server
    /// charges; stop everything when it does not.
    func configure(enabled: Bool, productIDs: [String], appAccountToken: String?) {
        isEnabled = enabled
        self.productIDs = enabled ? productIDs : []
        self.appAccountToken = enabled ? appAccountToken.flatMap(UUID.init(uuidString:)) : nil
        remember(enabled ? productIDs : nil)
        guard enabled, !productIDs.isEmpty else {
            stopListening()
            return
        }
        startListening()
        guard !sentThisSession else { return }
        sentThisSession = true
        launchSend = Task { await self.sendNewestEntitlement() }
    }

    /// At launch, once there is a session to send with and before the first
    /// `/me` answers: listen at once when this server charged last time, as
    /// Apple asks — an update delivered before anybody listens is delivered
    /// again, but only to a listener. A server that did not charge last time
    /// gets no StoreKit call until a `/me` says it does.
    func startRemembered() {
        guard let products = remembered(), !products.isEmpty else { return }
        productIDs = products
        isEnabled = true
        startListening()
    }

    /// A sign-out, a lost session or a deleted account: stop, and let the
    /// next sign-in send again.
    func reset() {
        launchSend?.cancel()
        stopListening()
        isEnabled = false
        productIDs = []
        appAccountToken = nil
        sentThisSession = false
        launchSend = nil
    }

    // MARK: - Sending

    /// The newest entitlement among the server's products, to the server.
    @discardableResult
    func sendNewestEntitlement() async -> Outcome {
        guard let entitlement = await store.newestEntitlement(among: Set(productIDs)) else { return .nothing }
        return await present(entitlement)
    }

    /// Hand one entitlement to the server, finish it or leave it by the
    /// answer, and read `/me` again after an accepted one.
    @discardableResult
    func present(_ entitlement: StoreEntitlement) async -> Outcome {
        do {
            let subscription = try await send(entitlement.transaction, entitlement.renewal)
            apply(subscription)
            await store.finish(transactionID: entitlement.transactionID)
            AppLog.app.info("Subscription accepted by the server")
            await refresh()
            return .accepted(subscription)
        } catch APIError.conflict(let code?, _) where Self.terminalCodes.contains(code) {
            await store.finish(transactionID: entitlement.transactionID)
            AppLog.app.info("Subscription refused by the server: \(code, privacy: .public)")
            return .refused(code: code)
        } catch APIError.unauthorized {
            onUnauthorized()
            return .failed
        } catch let error as APIError where error.isTransient {
            AppLog.app.info("Subscription not delivered; StoreKit will offer it again")
            return .transient
        } catch {
            AppLog.app.info("Subscription not accepted: \(Self.describe(error), privacy: .public)")
            return .failed
        }
    }

    /// Restore Purchases: `AppStore.sync()`, then the newest entitlement to
    /// the server. A sync that fails — the person dismissed the App Store's
    /// sign-in, say — still sends what this device already holds.
    func restore() async -> Outcome {
        do {
            try await store.sync()
        } catch {
            AppLog.app.info("App Store sync did not complete")
        }
        return await sendNewestEntitlement()
    }

    /// A purchase made on the subscription screen, as StoreKit reported it.
    func completePurchase(_ result: VerificationResult<Transaction>) async -> Outcome {
        await present(await StoreKitSubscriptionStore.entitlement(from: result))
    }

    // MARK: - Transaction.updates

    private func startListening() {
        guard listener == nil else { return }
        let updates = store.updates()
        listener = Task { [weak self] in
            for await entitlement in updates {
                guard let self, !Task.isCancelled else { return }
                // Only the server's own products. Left unfinished otherwise:
                // it is not this server's to settle.
                guard self.productIDs.contains(entitlement.productID) else { continue }
                await self.present(entitlement)
            }
        }
    }

    private func stopListening() {
        listener?.cancel()
        listener = nil
    }

    /// An error's kind for the log — the code, never a body.
    private static func describe(_ error: Error) -> String {
        switch error {
        case APIError.forbidden(let code): "403 \(code ?? "-")"
        case APIError.notFound(let code): "404 \(code ?? "-")"
        case APIError.conflict(let code, _): "4xx \(code ?? "-")"
        case APIError.decoding: "undecodable answer"
        default: "other"
        }
    }
}
