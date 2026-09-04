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
//    - accountDeleted→ same as logout, and deliberately NOT logout(): the
//                      server has already destroyed the session, so
//                      /auth/logout and DELETE /devices would both answer
//                      401 against it.
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
        /// The account itself is gone (POST /me/delete succeeded). The
        /// same scope as a logout — there is simply nothing left on the
        /// server to keep a token for.
        case accountDeleted
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
        case .unauthorized, .accountDeleted:
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

    /// Replace the cached profile after the user changes it themselves —
    /// today only the avatar. Everything else about `currentUser` still
    /// arrives from /me, so this is a narrow door rather than a setter.
    func applyProfile(_ user: UserDTO) {
        currentUser = user
    }
    private(set) var family: FamilyDTO?

    /// Replace the cached family after the owner changes a setting on it
    /// — the twin of `applyProfile`, and narrow for the same reason.
    /// PATCH /families/mine answers with the whole family, and nothing
    /// else re-reads it until the next resync, so a settings screen that
    /// did not write back here would keep showing the old value.
    func applyFamily(_ family: FamilyDTO) {
        self.family = family
    }
    /// "owner" | "member" | nil — from the last /me or family call.
    private(set) var role: String?
    /// Whether the server signals voice calls (docs/protocol.md, "Voice
    /// calls"). The call button hides behind it rather than discovering
    /// `calls_disabled` at the moment somebody wants to talk.
    private(set) var callsEnabled = false
    /// Whether the server accepts VIDEO calls (docs/protocol.md, "Video").
    /// Gates the video-call button alone — voice calls sit behind
    /// `callsEnabled` exactly as before.
    private(set) var videoCallsEnabled = false
    /// The operator's ceiling on a family's size, from `GET /me`. The range
    /// an owner's cap picker draws itself from, rather than discovering a
    /// `validation` error at the moment somebody saves. Nil on a server
    /// that predates the cap.
    private(set) var maxFamilyMembers: Int?
    /// Whether this server takes NEW families (docs/protocol.md, "Starting
    /// a family"). The family gate replaces "Create a family" with
    /// directions to run one's own server when it is false; true until a
    /// `/me` says otherwise, which is also the answer on an older server.
    private(set) var familyRegistrationEnabled = true
    /// Days an account may go without a family before this server removes
    /// it; 0 when it never does (docs/protocol.md, "Accounts without a
    /// family"). The family gate says so under its two doors.
    private(set) var familylessAccountTTLDays = 0
    /// How to reach the operator, when they published anything. Shown on
    /// the report screen: it is the escalation path for when the family's
    /// own moderator is the problem.
    private(set) var supportContact: String?
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

    /// A call the SYSTEM asked for — a Recents row, a contact card's
    /// "Family" button, Siri (CallIntents) — waiting for the chat list to
    /// route it once the phase is `.active`. The pendingPushRoute idiom:
    /// parked here so a request that launched the app survives bootstrap,
    /// and cleared by every purge.
    var pendingCallRequest: CallRequest?

    /// Files shared INTO the app, waiting for the user to choose a chat.
    ///
    /// Set by `handleShareURL` (the share extension's hand-off), consumed
    /// by the chat pickers (ChatListView / MacChatView). The pendingPushRoute
    /// idiom: parked here so a share that launched the app survives
    /// bootstrap, and cleared by every purge for the same reason.
    var pendingShareImport: [URL]?

    /// The chosen destination: the shared files, addressed to one chat.
    /// Set by `chooseShareTarget`, consumed exactly once by that chat's
    /// conversation view, which STAGES the files in its composer — the
    /// user still presses Send there; nothing auto-sends.
    var shareImportTarget: ShareImportTarget?

    struct ShareImportTarget: Equatable {
        let chatID: Int64
        let urls: [URL]
    }

    /// The share extension opened `familyconnect://share?ids=…`: move the
    /// staged files out of the App Group inbox into this process's temp
    /// directory and park them for the chat picker. Ids are validated as
    /// UUIDs (ShareHandoff) — anything else never becomes a path.
    ///
    /// `container` is injectable so the parsing-and-moving shape can be
    /// exercised against a plain directory; the app passes nothing and
    /// gets the real group container.
    func handleShareURL(
        _ url: URL,
        container: URL? = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: ShareHandoff.appGroup)
    ) {
        guard let ids = ShareHandoff.ids(from: url), let container else { return }
        var moved: [URL] = []
        for id in ids {
            guard let inbox = ShareHandoff.stagingDirectory(container: container, id: id),
                  let name = (try? FileManager.default.contentsOfDirectory(atPath: inbox.path))?.first
            else { continue }
            let source = inbox.appendingPathComponent(name)
            let destinationDirectory = FileManager.default.temporaryDirectory
                .appendingPathComponent("fc-shared-\(id)", isDirectory: true)
            let destination = destinationDirectory.appendingPathComponent(name)
            do {
                try? FileManager.default.removeItem(at: destinationDirectory)
                try FileManager.default.createDirectory(
                    at: destinationDirectory, withIntermediateDirectories: true)
                try FileManager.default.moveItem(at: source, to: destination)
                moved.append(destination)
            } catch {
                AppLog.app.info("Share import could not move a staged file: \(String(describing: error))")
            }
            try? FileManager.default.removeItem(at: inbox)
        }
        guard !moved.isEmpty else { return }
        // A second share before the first was placed replaces it — the
        // picker is about to show THESE files, and the old ones are gone
        // from the composer nobody chose for them.
        discardPendingShareImport()
        pendingShareImport = moved
    }

    /// The picker chose: address the parked files to one chat.
    func chooseShareTarget(chatID: Int64) {
        guard let urls = pendingShareImport else { return }
        pendingShareImport = nil
        shareImportTarget = ShareImportTarget(chatID: chatID, urls: urls)
    }

    /// The addressed files, handed over exactly once — and only to the
    /// chat they were addressed to, so a conversation that merely appears
    /// while a share is pending cannot swallow it.
    func takeShareImport(for chatID: Int64) -> [URL]? {
        guard let target = shareImportTarget, target.chatID == chatID else { return nil }
        shareImportTarget = nil
        return target.urls
    }

    /// The picker was dismissed without choosing: the files are nobody's,
    /// so they go. A no-op after `chooseShareTarget`, which is what lets
    /// the sheet's onDismiss call this unconditionally.
    func discardPendingShareImport() {
        guard let urls = pendingShareImport else { return }
        pendingShareImport = nil
        deleteShareImportFiles(urls)
    }

    /// Drop BOTH share-import holders and their bytes: the files still
    /// parked for the picker AND the ones `chooseShareTarget` already
    /// addressed to a chat but no conversation has consumed yet. The
    /// purge path — `discardPendingShareImport` alone is a no-op once
    /// the URLs have moved into `shareImportTarget`, which would clear
    /// the state while leaving the moved files on disk.
    func discardShareImports() {
        let urls = (pendingShareImport ?? []) + (shareImportTarget?.urls ?? [])
        pendingShareImport = nil
        shareImportTarget = nil
        deleteShareImportFiles(urls)
    }

    /// Each shared file lives alone in its per-import `fc-shared-<id>`
    /// directory (see `handleShareURL`); deleting the DIRECTORY is what
    /// keeps a discard from leaving empty husks behind.
    private func deleteShareImportFiles(_ urls: [URL]) {
        for url in urls {
            try? FileManager.default.removeItem(at: url.deletingLastPathComponent())
        }
    }

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
    /// Hand the block list to the store, wholesale. A closure for the
    /// reason `clearChatStore` is one: the session owns the phase machine
    /// and knows nothing about SwiftData.
    var applyBlockedIDs: ([Int64]) -> Void = { _ in }

    /// Drops cached profile pictures. Wired alongside `clearChatStore`
    /// so a logout does not leave the previous account's faces in memory
    /// for the next one.
    var clearAvatarCache: () -> Void = {}
    /// Set by the app: drops in-flight and failed media sends, files included.
    var clearMediaOutbox: (() -> Void)?
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
        callsEnabled = me.callsEnabled
        videoCallsEnabled = me.videoCallsEnabled
        maxFamilyMembers = me.maxFamilyMembers
        supportContact = me.supportContact
        familyRegistrationEnabled = me.familyRegistrationEnabled
        familylessAccountTTLDays = me.familylessAccountTTLDays
        // Replaced wholesale on every /me, which is step 1 of the
        // documented resync — so the block list is a step-1 fact and the
        // `member_blocked` frame is a latency optimisation rather than the
        // only delivery path (protocol.md, "Blocking a member").
        applyBlockedIDs(me.blockedUserIDs)
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
        // request — but the gate it lands on is drawn from what /me says
        // about THIS server: whether it takes new families at all, and how
        // long an account may wait to join one. The person who just
        // registered is exactly who those two facts are for, so the
        // round-trip is not skipped (docs/protocol.md, "Starting a family"
        // and "Accounts without a family").
        do {
            apply(me: try await api.me())
        } catch {
            AppLog.app.error("Post-register /me failed: \(String(describing: error))")
            phase = .needsFamily
        }
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
    /// Leave the family, and answer with the NAME of whoever inherited it
    /// — or nil when nobody did.
    ///
    /// `resolveName` runs BEFORE the purge, and that ordering is the whole
    /// point of taking a closure instead of returning the raw id:
    /// `purge(.leftFamily)` wipes the roster the id has to be looked up
    /// in, so a caller resolving afterwards would always name nobody. The
    /// protocol says it in the same order — the leaving owner "resolves
    /// `new_owner_user_id` against the roster it still holds, tells the
    /// user who inherited, and only then tears its family state down"
    /// (docs/protocol.md, `POST /families/leave`).
    ///
    /// An owner is never refused. `owner_cannot_leave` is retired.
    func leaveFamily(resolveName: (Int64) -> String?) async throws -> String? {
        let successor = try await api.leaveFamily()
        let name = successor.flatMap(resolveName)
        purge(.leftFamily)
        family = nil
        role = nil
        phase = .needsFamily
        return name
    }

    func logout() async {
        // Best-effort device removal first (see deregisterDevice), then
        // best-effort revoke; local state goes regardless.
        await deregisterDevice()
        try? await api.logout()
        resetToSignIn(.logout)
    }

    /// Delete this account for good (protocol.md, "Deleting an account").
    ///
    /// Deliberately NOT the logout path. By the time this returns, the
    /// server has destroyed every session the account had — this one
    /// included — and the device rows with them, so `DELETE /devices/{id}`
    /// and `POST /auth/logout` would each answer 401 against a session
    /// that no longer exists. There is nothing to deregister and nothing
    /// to revoke; what is left is local, and all of it goes: the token,
    /// the chats and their per-chat cursors, the members, the board and
    /// its cursor, the cached faces and files, and every default this app
    /// owns except the server URL (which is the family's, not the
    /// account's, and retyping it is pure friction).
    ///
    /// Throws whatever the call threw — `.unauthorized` for a WRONG
    /// PASSWORD, which the caller must show as such rather than treating
    /// as a dead session.
    func deleteAccount(password: String) async throws {
        try await api.deleteAccount(password: password)
        AppLog.app.info("Account deleted; returning to sign-in")
        resetToSignIn(.accountDeleted)
    }

    /// Any 401 outside login: the session is gone server-side.
    func handleUnauthorized() {
        resetToSignIn(.unauthorized)
    }

    /// Wipe to the given scope and land on the sign-in screen. The one
    /// primitive behind logout, a 401 and a deleted account — they differ
    /// only in what they had to do on the server first.
    private func resetToSignIn(_ reason: SessionLogic.PurgeReason) {
        purge(reason)
        currentUser = nil
        family = nil
        role = nil
        // What the server said about itself goes too: the next sign-in may
        // be to a different server, and a gate that quoted the old one's
        // grace or door until /me answered would be quoting the wrong server.
        callsEnabled = false
        videoCallsEnabled = false
        maxFamilyMembers = nil
        supportContact = nil
        familyRegistrationEnabled = true
        familylessAccountTTLDays = 0
        phase = .needsAuth
    }

    /// A `family_owner` frame: the family has a new owner.
    ///
    /// Sent to every member when an owner deletes their account and
    /// ownership passes to the longest-standing remaining member. The
    /// client that has just become the owner gains the owner-only screens
    /// immediately instead of at its next `GET /me`; everybody else's
    /// `role` is corrected to "member" for the same reason.
    /// Bumped every time a `family_owner` frame lands. `role` is not
    /// enough on its own: ownership moving between two OTHER members
    /// leaves it untouched, and a leave dialog open at that moment is
    /// still naming a successor that has just changed under it
    /// (docs/protocol.md, `POST /families/leave`).
    private(set) var familyOwnerGeneration = 0

    func applyFamilyOwner(userID: Int64) {
        guard family != nil else { return }
        let me = currentUser?.id ?? AppSettings.currentUserID
        role = (userID == me) ? "owner" : "member"
        familyOwnerGeneration &+= 1
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
            clearAvatarCache()
            // A media send still in the outbox belongs to the family that
            // just went — the same scope as the roster and the chat store.
            clearMediaOutbox?()
            // Member ↔ contact links name members of the family that just
            // went — family-scoped like the roster, not defaults-scoped,
            // so they go with it on a kick or a leave as well.
            ContactLinks.shared.removeAll()
        }
        if scope.wipesDefaults {
            AppSettings.wipe(keepServerURL: !scope.wipesServerURL)
        }
        pendingPushRoute = nil
        pendingCallRequest = nil
        // Shared files waiting for a chat are family-scoped too: a share
        // parked before a logout must not land in the next session —
        // whether it is still waiting for the picker or was already
        // addressed to a chat. Both holders, bytes included.
        discardShareImports()
        AppLog.app.info("Purged local state (\(String(describing: reason), privacy: .public))")
    }

    // MARK: - Helpers

    private static func describeTransport(_ error: Error) -> String {
        if case APIError.transport(let urlError) = error {
            return urlError.localizedDescription
        }
        return String(localized: "The server could not be reached.")
    }
}
