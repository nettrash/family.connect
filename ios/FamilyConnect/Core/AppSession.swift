//
//  AppSession.swift
//  FamilyConnect
//
//  The app's phase machine: which of the six top-level subtrees RootView
//  shows, and every transition between them. All the *decisions* live in
//  the pure `SessionLogic` enum below (unit-testable as tables); this
//  class is the stateful shell that executes them against the keychain,
//  UserDefaults, the API client and the SwiftData store.
//
//  Purge discipline (the part that's easy to get subtly wrong):
//    - logout        → wipe token + chat data + defaults, KEEP server URL
//                      (the family's server rarely changes; retyping it is
//                      pure friction).
//    - unauthorized  → same as logout (the session is gone either way).
//    - server change → wipe EVERYTHING including the old server URL; chats
//                      from server A are meaningless — and misleading —
//                      against server B's ids.
//    - kicked/left   → wipe chat data only; the account (token) and server
//                      are still valid, the user just has no family now.
//
//  The store-touching side effects (`hasCachedChats`, `clearChatStore`)
//  are injected closures rather than a ModelContainer dependency, so the
//  phase machine tests run without SwiftData.
//

import Foundation
import Observation
import os

// MARK: - Pure decision tables

nonisolated enum SessionLogic {

    /// Where a GET /me response routes the app.
    enum ResolvedPhase: Equatable, Sendable {
        case active
        case pendingApproval
        /// `declined` is true when we *were* waiting on a join request and
        /// /me now shows neither a family nor a pending request — the only
        /// way the protocol lets a client learn its request was rejected.
        case needsFamily(declined: Bool)
    }

    static func resolve(me: MeResponse, wasAwaitingApproval: Bool) -> ResolvedPhase {
        if me.family != nil { return .active }
        if me.pendingJoinRequest != nil { return .pendingApproval }
        return .needsFamily(declined: wasAwaitingApproval)
    }

    /// Why local state is being purged; drives what survives.
    enum PurgeReason: Equatable, Sendable {
        case logout
        case serverChange
        case unauthorized
        case kicked
        case leftFamily
    }

    struct PurgeScope: Equatable, Sendable {
        let wipesToken: Bool
        let wipesServerURL: Bool
        let wipesChatData: Bool
        /// currentUserID / joinPending / push-registration defaults.
        let wipesDefaults: Bool
    }

    static func purgeScope(for reason: PurgeReason) -> PurgeScope {
        switch reason {
        case .logout:
            PurgeScope(wipesToken: true, wipesServerURL: false, wipesChatData: true, wipesDefaults: true)
        case .serverChange:
            PurgeScope(wipesToken: true, wipesServerURL: true, wipesChatData: true, wipesDefaults: true)
        case .unauthorized:
            PurgeScope(wipesToken: true, wipesServerURL: false, wipesChatData: true, wipesDefaults: true)
        case .kicked, .leftFamily:
            // The session survives; only family-scoped data goes.
            PurgeScope(wipesToken: false, wipesServerURL: false, wipesChatData: true, wipesDefaults: false)
        }
    }
}

// MARK: - The phase machine

@MainActor @Observable
final class AppSession {

    enum Phase: Equatable {
        /// Bootstrapping (or bootstrapping failed — see `bootError`).
        case booting
        case needsServer
        case needsAuth
        case needsFamily
        case pendingApproval
        case active
    }

    private(set) var phase: Phase = .booting
    private(set) var currentUser: UserDTO?
    private(set) var family: FamilyDTO?
    /// "owner" | "member" | nil — from the last /me or family call.
    private(set) var role: String?
    /// Set when a pending join request silently disappeared from /me:
    /// FamilyGateView surfaces "your request was declined" once.
    var joinDeclined = false
    /// Non-nil when bootstrap hit a transport error with no cached chats
    /// to fall back on; BootingView shows it with a Retry button.
    private(set) var bootError: String?
    /// A notification tap waiting to be acted on. Set by AppDelegate's
    /// didReceive — possibly before bootstrap finishes (a cold-start
    /// tap) — and consumed by ChatListView once the phase is .active.
    /// Cleared by every purge: a route into a family we just left (or a
    /// session we just dropped) must not navigate the next user.
    var pendingPushRoute: PushRoute?

    var isOwner: Bool { role == "owner" }

    private let api: APIClient

    /// The build's compiled-in default server, if any — a seam over
    /// `AppSettings.defaultServerURL` so tests can drive both the
    /// "store build with a default" and "generic build without one"
    /// bootstrap branches without touching the real bundle.
    private let defaultServerURL: () -> URL?

    // Store side effects, injected at wiring time (see file header).
    var hasCachedChats: () -> Bool = { false }
    var clearChatStore: () -> Void = {}
    /// Best-effort push deregistration (PushRegistrar.deregister), also
    /// injected at wiring time so the phase machine stays UIKit-free.
    /// logout() awaits it BEFORE /auth/logout, because DELETE /devices
    /// authenticates with the very token logout revokes.
    var deregisterDevice: () async -> Void = {}

    init(api: APIClient, defaultServerURL: @escaping () -> URL? = { AppSettings.defaultServerURL }) {
        self.api = api
        self.defaultServerURL = defaultServerURL
    }

    // MARK: - Bootstrap

    /// Cold-start routing, exactly the design's decision ladder:
    /// no server URL → adopt the build's compiled-in default when there
    /// is one (store builds — see `AppSettings.defaultServerURL`), else
    /// needsServer; no token → needsAuth; then GET /me routes (family →
    /// active, pending → pendingApproval, neither → needsFamily). 401
    /// wipes the token and lands on needsAuth. A transport error opens
    /// cached chats offline when we have any, otherwise parks on booting
    /// with a retry affordance.
    ///
    /// Adopting the default deliberately skips `setServer`'s network
    /// probe: boot must not block on the network, and the URL is already
    /// normalized by the accessor, so it is persisted directly — exactly
    /// the state `setServer` would have left — and any connectivity
    /// problem surfaces on the auth screen instead. A stored URL always
    /// wins over the default, and "Change server" later overrides it
    /// through the normal `setServer` path.
    func bootstrap() async {
        guard phase == .booting else { return }
        bootError = nil

        guard let serverURL = AppSettings.serverURL else {
            if let compiledDefault = defaultServerURL() {
                AppLog.app.info("Adopting compiled-in default server")
                AppSettings.serverURL = compiledDefault
                await api.configure(serverURL: compiledDefault, token: nil)
                phase = .needsAuth
            } else {
                phase = .needsServer
            }
            return
        }
        guard let token = try? KeychainStore.getString(account: KeychainStore.tokenAccount) else {
            await api.configure(serverURL: serverURL, token: nil)
            phase = .needsAuth
            return
        }
        await api.configure(serverURL: serverURL, token: token)

        do {
            apply(me: try await api.me())
        } catch APIError.unauthorized {
            handleUnauthorized()
        } catch {
            if hasCachedChats() {
                // Offline but with history: open the chat list read-only;
                // the coordinator keeps retrying the socket + resync.
                AppLog.app.info("Bootstrap offline; opening cached chats")
                phase = .active
            } else {
                bootError = Self.describeTransport(error)
            }
        }
    }

    func retryBootstrap() async {
        await bootstrap()
    }

    // MARK: - /me reconciliation (shared by bootstrap, polling, resync)

    /// Route from a fresh /me. Also detects the "kicked while active"
    /// case: we were in a family and /me no longer shows one.
    func apply(me: MeResponse) {
        currentUser = me.user
        AppSettings.currentUserID = me.user.id
        let wasActive = phase == .active
        let wasAwaiting = phase == .pendingApproval || AppSettings.joinPending

        switch SessionLogic.resolve(me: me, wasAwaitingApproval: wasAwaiting) {
        case .active:
            family = me.family
            role = me.role
            AppSettings.joinPending = false
            joinDeclined = false
            phase = .active
        case .pendingApproval:
            family = nil
            role = nil
            AppSettings.joinPending = true
            phase = .pendingApproval
        case .needsFamily(let declined):
            family = nil
            role = nil
            AppSettings.joinPending = false
            if declined { joinDeclined = true }
            if wasActive { purge(.kicked) }
            phase = .needsFamily
        }
    }

    // MARK: - Transitions

    /// Validate + adopt a server URL. Changing to a *different* server
    /// wipes everything first — chats and ids from the old server are
    /// meaningless against the new one.
    func setServer(_ url: URL) async throws {
        guard await api.probe(serverURL: url) else {
            throw SetServerError.probeFailed
        }
        if let existing = AppSettings.serverURL, existing != url {
            purge(.serverChange)
        }
        AppSettings.serverURL = url
        await api.configure(serverURL: url, token: nil)
        phase = .needsAuth
    }

    enum SetServerError: Error {
        /// GET /me didn't answer 401-with-error-body: not our server.
        case probeFailed
    }

    /// "Change server" affordance from the auth screen. No purge yet —
    /// nothing is wiped until a *different* server is actually confirmed.
    func requestServerChange() {
        phase = .needsServer
    }

    func register(username: String, displayName: String, password: String) async throws {
        let response = try await api.register(username: username, displayName: displayName, password: password)
        try await adopt(session: response)
        // A brand-new account can't be in a family or have a pending
        // request; skip the /me round-trip.
        phase = .needsFamily
    }

    func login(username: String, password: String) async throws {
        let response = try await api.login(username: username, password: password)
        try await adopt(session: response)
        do {
            apply(me: try await api.me())
        } catch {
            // Login succeeded but /me flaked: needsFamily is the safe
            // default (bootstrap-on-next-launch or the family screens'
            // own errors will correct it).
            AppLog.app.error("Post-login /me failed: \(String(describing: error))")
            phase = .needsFamily
        }
    }

    private func adopt(session response: AuthResponse) async throws {
        try KeychainStore.setString(response.token, account: KeychainStore.tokenAccount)
        await api.setToken(response.token)
        currentUser = response.user
        AppSettings.currentUserID = response.user.id
    }

    func createFamily(name: String) async throws {
        let created = try await api.createFamily(name: name)
        family = created
        role = "owner"
        joinDeclined = false
        phase = .active
    }

    enum JoinOutcome: Equatable {
        case joined
        case pending
    }

    @discardableResult
    func join(code: String) async throws -> JoinOutcome {
        let response = try await api.joinFamily(inviteCode: code)
        joinDeclined = false
        if response.status == "pending" {
            AppSettings.joinPending = true
            phase = .pendingApproval
            return .pending
        }
        // "joined" — fetch /me for the family object + role.
        do {
            apply(me: try await api.me())
        } catch {
            phase = .active // joined for sure; details refresh on resync
        }
        return .joined
    }

    /// One poll tick while waiting for approval; also the pull-to-refresh
    /// action. Transport errors are ignored — the next tick retries.
    func pollPending() async {
        guard phase == .pendingApproval else { return }
        do {
            apply(me: try await api.me())
        } catch APIError.unauthorized {
            handleUnauthorized()
        } catch {
            // stay pending; poll again next tick
        }
    }

    /// Throws `.conflict(code: "owner_cannot_leave", …)` for a non-sole
    /// owner — SettingsView shows the explanatory alert.
    func leaveFamily() async throws {
        try await api.leaveFamily()
        purge(.leftFamily)
        family = nil
        role = nil
        phase = .needsFamily
    }

    func logout() async {
        // Best-effort device removal first (see deregisterDevice), then
        // best-effort revoke; local state goes regardless.
        await deregisterDevice()
        try? await api.logout()
        purge(.logout)
        currentUser = nil
        family = nil
        role = nil
        phase = .needsAuth
    }

    /// Any 401 outside login: the session is gone server-side.
    func handleUnauthorized() {
        purge(.unauthorized)
        currentUser = nil
        family = nil
        role = nil
        phase = .needsAuth
    }

    // MARK: - Purge

    /// Execute a purge scope. Kept `internal` (not private) so transition
    /// tests can drive reasons directly.
    func purge(_ reason: SessionLogic.PurgeReason) {
        let scope = SessionLogic.purgeScope(for: reason)
        if scope.wipesToken {
            try? KeychainStore.delete(account: KeychainStore.tokenAccount)
            let keepURL: URL? = scope.wipesServerURL ? nil : AppSettings.serverURL
            Task { await api.configure(serverURL: keepURL, token: nil) }
        }
        if scope.wipesChatData {
            clearChatStore()
        }
        if scope.wipesDefaults {
            AppSettings.wipe(keepServerURL: !scope.wipesServerURL)
        }
        pendingPushRoute = nil
        AppLog.app.info("Purged local state (\(String(describing: reason), privacy: .public))")
    }

    // MARK: - Helpers

    private static func describeTransport(_ error: Error) -> String {
        if case APIError.transport(let urlError) = error {
            return urlError.localizedDescription
        }
        return "The server could not be reached."
    }
}
