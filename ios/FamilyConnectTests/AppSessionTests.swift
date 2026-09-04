//
//  AppSessionTests.swift
//  FamilyConnectTests
//
//  The phase machine: the pure /me routing and purge-scope tables in
//  SessionLogic, plus the stateful transitions of AppSession itself with
//  spy closures standing in for the SwiftData store. Serialized because
//  the transition tests exercise the real AppSettings (UserDefaults) and
//  the keychain, which are process-global.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

// MARK: - Pure tables

@Suite("SessionLogic tables")
struct SessionLogicTests {

    private static let user = UserDTO(id: 7, username: "anna", displayName: "Anna", createdAt: nil)
    private static let family = FamilyDTO(
        id: 3, name: "The Smiths", joinPolicy: "open", createdAt: nil, inviteCode: nil,
        aiVision: false, aiHistoryPhotos: false, maxMembers: nil)
    private static let pending = PendingJoinRequestDTO(familyID: 3, familyName: "The Smiths", createdAt: nil)

    @Test("family present → active (regardless of prior waiting)")
    func resolveActive() {
        let me = MeResponse(user: Self.user, family: Self.family, role: "member", pendingJoinRequest: nil)
        #expect(SessionLogic.resolve(me: me, wasAwaitingApproval: false) == .active)
        #expect(SessionLogic.resolve(me: me, wasAwaitingApproval: true) == .active)
    }

    @Test("pending request → pendingApproval")
    func resolvePending() {
        let me = MeResponse(user: Self.user, family: nil, role: nil, pendingJoinRequest: Self.pending)
        #expect(SessionLogic.resolve(me: me, wasAwaitingApproval: false) == .pendingApproval)
    }

    @Test("neither, not waiting → needsFamily, not declined")
    func resolveNeedsFamily() {
        let me = MeResponse(user: Self.user, family: nil, role: nil, pendingJoinRequest: nil)
        #expect(SessionLogic.resolve(me: me, wasAwaitingApproval: false) == .needsFamily(declined: false))
    }

    @Test("neither, but WAS waiting → declined (the protocol's only rejection signal)")
    func resolveDeclined() {
        let me = MeResponse(user: Self.user, family: nil, role: nil, pendingJoinRequest: nil)
        #expect(SessionLogic.resolve(me: me, wasAwaitingApproval: true) == .needsFamily(declined: true))
    }

    @Test("purge scopes: logout keeps the server URL, wipes the rest")
    func purgeLogout() {
        let scope = SessionLogic.purgeScope(for: .logout)
        #expect(scope == .init(wipesToken: true, wipesServerURL: false, wipesChatData: true, wipesDefaults: true))
    }

    @Test("purge scopes: server change wipes everything")
    func purgeServerChange() {
        let scope = SessionLogic.purgeScope(for: .serverChange)
        #expect(scope == .init(wipesToken: true, wipesServerURL: true, wipesChatData: true, wipesDefaults: true))
    }

    @Test("purge scopes: unauthorized behaves like logout")
    func purgeUnauthorized() {
        let scope = SessionLogic.purgeScope(for: .unauthorized)
        #expect(scope == .init(wipesToken: true, wipesServerURL: false, wipesChatData: true, wipesDefaults: true))
    }

    @Test("purge scopes: a deleted account purges exactly like a logout")
    func purgeAccountDeleted() {
        // Same scope, different reason — and the reason is what says out
        // loud that there is no session left to revoke on the server
        // (protocol.md, "Deleting an account").
        let scope = SessionLogic.purgeScope(for: .accountDeleted)
        #expect(scope == SessionLogic.purgeScope(for: .logout))
        #expect(scope == .init(wipesToken: true, wipesServerURL: false, wipesChatData: true, wipesDefaults: true))
    }

    @Test("purge scopes: kicked/left keep token and server, wipe chat data only")
    func purgeKickedAndLeft() {
        for reason in [SessionLogic.PurgeReason.kicked, .leftFamily] {
            let scope = SessionLogic.purgeScope(for: reason)
            #expect(scope == .init(wipesToken: false, wipesServerURL: false, wipesChatData: true, wipesDefaults: false))
        }
    }
}

// MARK: - Stateful transitions

@MainActor
@Suite("AppSession transitions", .serialized)
struct AppSessionTransitionTests {

    private static let user = UserDTO(id: 7, username: "anna", displayName: "Anna", createdAt: nil)
    private static let family = FamilyDTO(
        id: 3, name: "The Smiths", joinPolicy: "open", createdAt: nil, inviteCode: nil,
        aiVision: false, aiHistoryPhotos: false, maxMembers: nil)

    /// Spy-instrumented session against a never-hit API client.
    @MainActor
    private final class Spies {
        var chatStoreCleared = 0
        var cachedChats = false
        var avatarCacheCleared = 0
        /// Deleting an account must NOT call this: the server has already
        /// destroyed the device rows along with the sessions.
        var deregistered = 0
    }

    /// `defaultServerURL` is the compiled-in-default seam; tests default
    /// it to "no default" so they stay deterministic regardless of which
    /// build configuration hosts them (a Release-nettrash host would
    /// otherwise leak its real default into every bootstrap test).
    private func makeSession(
        api: APIClient? = nil,
        defaultServerURL: @escaping () -> URL? = { nil }
    ) -> (AppSession, Spies) {
        let spies = Spies()
        let session = AppSession(api: api ?? APIClient(serverURL: nil), defaultServerURL: defaultServerURL)
        session.clearChatStore = { spies.chatStoreCleared += 1 }
        session.hasCachedChats = { spies.cachedChats }
        session.clearAvatarCache = { spies.avatarCacheCleared += 1 }
        session.deregisterDevice = { spies.deregistered += 1 }
        return (session, spies)
    }

    // MARK: - Leaving a family

    /// The ordering that makes the hand-off message possible at all.
    ///
    /// `purge(.leftFamily)` wipes the roster the successor's id has to be
    /// looked up in, so resolving after it would always name nobody. The
    /// protocol states the order — the leaving owner "resolves
    /// `new_owner_user_id` against the roster it still holds, tells the
    /// user who inherited, and only then tears its family state down"
    /// (docs/protocol.md, `POST /families/leave`) — and nothing about the
    /// code makes it obvious, which is why it is pinned here.
    @Test("the successor is resolved BEFORE the family state is torn down")
    func leaveResolvesBeforeTearingDown() async throws {
        let host = "session-leave-owner.test"
        StubURLProtocol.register(host: host) { _ in
            .json(200, #"{"new_owner_user_id": 11}"#)
        }
        defer { StubURLProtocol.unregister(host: host) }
        let (session, spies) = makeSession(api: APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession()))

        var clearedWhenResolved: Int?
        let name = try await session.leaveFamily { id in
            clearedWhenResolved = spies.chatStoreCleared
            return id == 11 ? "Kid" : nil
        }

        #expect(name == "Kid")
        #expect(clearedWhenResolved == 0, "the roster must still be standing when the id is resolved")
        #expect(spies.chatStoreCleared == 1, "and torn down once the name is in hand")
        #expect(session.phase == .needsFamily)
    }

    /// An ordinary member leaving gets `204` and no body. Nothing is
    /// named, and the teardown happens all the same.
    @Test("a member leaving names nobody, and still leaves")
    func leaveWithoutSuccessor() async throws {
        let host = "session-leave-member.test"
        StubURLProtocol.register(host: host) { _ in .empty(204) }
        defer { StubURLProtocol.unregister(host: host) }
        let (session, spies) = makeSession(api: APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession()))

        var resolverCalls = 0
        let name = try await session.leaveFamily { _ in
            resolverCalls += 1
            return "Should never be asked"
        }

        #expect(name == nil)
        #expect(resolverCalls == 0, "there is no id to resolve, so nothing to ask about")
        #expect(spies.chatStoreCleared == 1)
    }

    private func resetGlobals() {
        AppSettings.wipe(keepServerURL: false)
        try? KeychainStore.delete(account: KeychainStore.tokenAccount)
    }

    /// Whether this environment can write the keychain at all. An
    /// unsigned test host (CODE_SIGNING_ALLOWED=NO) cannot on some
    /// simulator runtimes; tests that *require* a stored token gate on
    /// this instead of failing on infrastructure.
    private func keychainWritable() -> Bool {
        let probe = "probe-\(UUID().uuidString)"
        defer { try? KeychainStore.delete(account: probe) }
        return (try? KeychainStore.setString("x", account: probe)) != nil
    }

    @Test("apply: family → active, records user and role")
    func applyActive() {
        resetGlobals()
        defer { resetGlobals() }
        let (session, _) = makeSession()

        session.apply(me: MeResponse(user: Self.user, family: Self.family, role: "owner", pendingJoinRequest: nil))

        #expect(session.phase == .active)
        #expect(session.currentUser?.id == 7)
        #expect(session.isOwner)
        #expect(AppSettings.currentUserID == 7)
        #expect(!AppSettings.joinPending)
    }

    @Test("apply: what the server said about its door and its grace reaches the gate")
    func applyCarriesTheGateFacts() {
        resetGlobals()
        defer { resetGlobals() }
        let (session, _) = makeSession()
        #expect(session.familyRegistrationEnabled)
        #expect(session.familylessAccountTTLDays == 0)

        session.apply(me: MeResponse(
            user: Self.user, family: nil, role: nil, pendingJoinRequest: nil,
            familyRegistrationEnabled: false, familylessAccountTTLDays: 7
        ))

        #expect(session.phase == .needsFamily)
        #expect(!session.familyRegistrationEnabled)
        #expect(session.familylessAccountTTLDays == 7)

        // And back, on a server that takes families and never sweeps.
        session.apply(me: MeResponse(user: Self.user, family: nil, role: nil, pendingJoinRequest: nil))
        #expect(session.familyRegistrationEnabled)
        #expect(session.familylessAccountTTLDays == 0)
    }

    @Test("apply: pending request → pendingApproval, persists the waiting bit")
    func applyPending() {
        resetGlobals()
        defer { resetGlobals() }
        let (session, _) = makeSession()

        session.apply(me: MeResponse(
            user: Self.user, family: nil, role: nil,
            pendingJoinRequest: PendingJoinRequestDTO(familyID: 3, familyName: "The Smiths", createdAt: nil)))

        #expect(session.phase == .pendingApproval)
        #expect(AppSettings.joinPending)
    }

    @Test("apply: was waiting (persisted), now neither → needsFamily + declined flag")
    func applyDeclinedAcrossLaunch() {
        resetGlobals()
        defer { resetGlobals() }
        AppSettings.joinPending = true // as a previous launch left it
        let (session, _) = makeSession()

        session.apply(me: MeResponse(user: Self.user, family: nil, role: nil, pendingJoinRequest: nil))

        #expect(session.phase == .needsFamily)
        #expect(session.joinDeclined)
        #expect(!AppSettings.joinPending)
    }

    @Test("apply: kicked while active → needsFamily, chat data purged, token kept")
    func applyKicked() throws {
        resetGlobals()
        defer { resetGlobals() }
        let canUseKeychain = keychainWritable()
        if canUseKeychain {
            try KeychainStore.setString("tok", account: KeychainStore.tokenAccount)
        }
        AppSettings.serverURL = URL(string: "https://family.example")!
        let (session, spies) = makeSession()
        session.apply(me: MeResponse(user: Self.user, family: Self.family, role: "member", pendingJoinRequest: nil))
        #expect(session.phase == .active)

        session.apply(me: MeResponse(user: Self.user, family: nil, role: nil, pendingJoinRequest: nil))

        #expect(session.phase == .needsFamily)
        #expect(spies.chatStoreCleared == 1)
        // Kicked keeps the session and the server.
        if canUseKeychain {
            #expect(try KeychainStore.getString(account: KeychainStore.tokenAccount) == "tok")
        }
        #expect(AppSettings.serverURL != nil)
    }

    @Test("handleUnauthorized: token gone, server kept, chat data purged, → needsAuth")
    func unauthorizedTransition() throws {
        resetGlobals()
        defer { resetGlobals() }
        let canUseKeychain = keychainWritable()
        if canUseKeychain {
            try KeychainStore.setString("tok", account: KeychainStore.tokenAccount)
        }
        AppSettings.serverURL = URL(string: "https://family.example")!
        AppSettings.currentUserID = 7
        let (session, spies) = makeSession()

        session.handleUnauthorized()

        #expect(session.phase == .needsAuth)
        #expect(spies.chatStoreCleared == 1)
        if canUseKeychain {
            #expect(try KeychainStore.getString(account: KeychainStore.tokenAccount) == nil)
        }
        #expect(AppSettings.serverURL != nil)       // logout-style: server survives
        #expect(AppSettings.currentUserID == nil)   // defaults wiped
    }

    @Test("bootstrap: no server URL, no compiled-in default → needsServer without any I/O")
    func bootstrapNeedsServer() async {
        resetGlobals()
        defer { resetGlobals() }
        let (session, _) = makeSession(defaultServerURL: { nil })

        await session.bootstrap()

        #expect(session.phase == .needsServer)
    }

    @Test("bootstrap: no server URL but a compiled-in default → adopts it, persists it, → needsAuth")
    func bootstrapAdoptsCompiledDefault() async {
        resetGlobals()
        defer { resetGlobals() }
        let defaultURL = URL(string: "https://fc.example")!
        let (session, _) = makeSession(defaultServerURL: { defaultURL })

        await session.bootstrap()

        // Straight to Register/Login — no server-setup screen and no
        // network probe on the boot path.
        #expect(session.phase == .needsAuth)
        // Persisted exactly like a user-entered URL, so the auth screen's
        // footer and "Change server…" prefill see it.
        #expect(AppSettings.serverURL == defaultURL)
    }

    @Test("bootstrap: a stored server URL wins over the compiled-in default")
    func bootstrapStoredURLWinsOverDefault() async {
        resetGlobals()
        defer { resetGlobals() }
        let stored = URL(string: "https://family.example")!
        AppSettings.serverURL = stored
        let (session, _) = makeSession(defaultServerURL: { URL(string: "https://fc.example")! })

        await session.bootstrap()

        #expect(session.phase == .needsAuth)
        #expect(AppSettings.serverURL == stored)
    }

    @Test("setServer after an adopted default: the user's choice overrides it persistently")
    func changeServerOverridesAdoptedDefault() async throws {
        resetGlobals()
        defer {
            resetGlobals()
            StubURLProtocol.unregister(host: "family-b.test")
        }
        // Boot a "store build": the default gets adopted.
        let api = APIClient(serverURL: nil, session: StubURLProtocol.makeSession())
        let (session, _) = makeSession(api: api, defaultServerURL: { URL(string: "https://fc.example")! })
        await session.bootstrap()
        #expect(AppSettings.serverURL == URL(string: "https://fc.example"))

        // "Change server…" → the normal setServer path (probe included).
        StubURLProtocol.register(host: "family-b.test") { _ in
            .json(401, #"{"error": {"code": "unauthorized", "message": "auth required"}}"#)
        }
        session.requestServerChange()
        #expect(session.phase == .needsServer)
        try await session.setServer(URL(string: "https://family-b.test")!)

        #expect(session.phase == .needsAuth)
        #expect(AppSettings.serverURL == URL(string: "https://family-b.test"))
    }

    @Test("this test host carries no compiled-in default (Debug/Release leave FC_DEFAULT_SERVER_URL empty)")
    func hostBundleHasNoDefault() {
        // Guards the committed builds' behavior: only Release-nettrash
        // sets the build setting, so the generic app must report nil
        // (Info.plist carries the key, but empty ⇒ nil).
        #expect(AppSettings.defaultServerURL == nil)
    }

    @Test("bootstrap: server but no token → needsAuth")
    func bootstrapNeedsAuth() async {
        resetGlobals()
        defer { resetGlobals() }
        AppSettings.serverURL = URL(string: "https://family.example")!
        let (session, _) = makeSession()

        await session.bootstrap()

        #expect(session.phase == .needsAuth)
    }

    @Test("bootstrap: transport error + cached chats → active (offline)")
    func bootstrapOfflineWithCache() async throws {
        resetGlobals()
        defer {
            resetGlobals()
            StubURLProtocol.unregister(host: "boot-offline.test")
        }
        // The flow under test needs a stored token to get past needsAuth.
        guard keychainWritable() else { return }
        AppSettings.serverURL = URL(string: "https://boot-offline.test")!
        try KeychainStore.setString("tok", account: KeychainStore.tokenAccount)
        // No handler registered for the host and no protocol match ⇒ the
        // ephemeral session fails DNS on the fake TLD → transport error.
        let spies = Spies()
        spies.cachedChats = true
        let session = AppSession(api: APIClient(serverURL: nil, session: StubURLProtocol.makeSession()))
        session.hasCachedChats = { spies.cachedChats }
        session.clearChatStore = { spies.chatStoreCleared += 1 }

        await session.bootstrap()

        #expect(session.phase == .active)
    }

    @Test("bootstrap: transport error, no cache → stays booting with an error")
    func bootstrapErrorWithoutCache() async throws {
        resetGlobals()
        defer { resetGlobals() }
        // The flow under test needs a stored token to get past needsAuth.
        guard keychainWritable() else { return }
        AppSettings.serverURL = URL(string: "https://boot-dead.test")!
        try KeychainStore.setString("tok", account: KeychainStore.tokenAccount)
        let (session, _) = makeSession() // hasCachedChats = false

        await session.bootstrap()

        #expect(session.phase == .booting)
        #expect(session.bootError != nil)
    }

    // MARK: - Deleting the account (protocol.md, "Deleting an account")

    @Test("deleteAccount: one POST /me/delete, then everything local goes")
    func deleteAccountTearsDownWithoutLoggingOut() async throws {
        resetGlobals()
        let host = "delete-account-session.test"
        defer {
            resetGlobals()
            StubURLProtocol.unregister(host: host)
        }
        StubURLProtocol.register(host: host) { _ in .empty(204) }
        let canUseKeychain = keychainWritable()
        if canUseKeychain {
            try KeychainStore.setString("tok", account: KeychainStore.tokenAccount)
        }
        AppSettings.serverURL = URL(string: "https://\(host)")!
        AppSettings.currentUserID = 7
        AppSettings.pushDeviceID = 12
        // A cursor from the account that is about to stop existing. The
        // board is family-scoped and its seqs mean nothing to the next
        // account signed in on this device.
        AppSettings.boardCursor = 88
        AppSettings.boardSeenNoteID = 91
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let (session, spies) = makeSession(api: api)
        session.apply(me: MeResponse(user: Self.user, family: Self.family, role: "owner", pendingJoinRequest: nil))

        try await session.deleteAccount(password: "correct horse")

        // Exactly one call, and it is the documented one — NOT the logout
        // path, whose two calls would both 401 against a session the
        // server has already destroyed.
        let requests = StubURLProtocol.requests(host: host)
        #expect(requests.count == 1)
        #expect(requests.first?.method == "POST")
        #expect(requests.first?.url.path() == "/api/v1/me/delete")
        #expect(requests.first?.bodyJSON()?["password"] as? String == "correct horse")
        #expect(spies.deregistered == 0)

        #expect(session.phase == .needsAuth)
        #expect(session.currentUser == nil)
        #expect(session.family == nil)
        #expect(!session.isOwner)
        // The whole cache, not just the token: chats (and with them every
        // per-chat cursor), members, notes, faces and files.
        #expect(spies.chatStoreCleared == 1)
        #expect(spies.avatarCacheCleared == 1)
        if canUseKeychain {
            #expect(try KeychainStore.getString(account: KeychainStore.tokenAccount) == nil)
        }
        #expect(AppSettings.currentUserID == nil)
        #expect(AppSettings.pushDeviceID == nil)
        #expect(AppSettings.boardCursor == 0)
        #expect(AppSettings.boardSeenNoteID == 0)
        // The server URL is the family's, not the account's.
        #expect(AppSettings.serverURL != nil)
    }

    @Test("deleteAccount: a wrong password is a 401 that must NOT sign anybody out")
    func deleteAccountWrongPasswordKeepsTheSession() async throws {
        resetGlobals()
        let host = "delete-account-wrong.test"
        defer {
            resetGlobals()
            StubURLProtocol.unregister(host: host)
        }
        StubURLProtocol.register(host: host) { _ in
            .json(401, #"{"error": {"code": "invalid_credentials", "message": "wrong password"}}"#)
        }
        AppSettings.serverURL = URL(string: "https://\(host)")!
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let (session, spies) = makeSession(api: api)
        session.apply(me: MeResponse(user: Self.user, family: Self.family, role: "member", pendingJoinRequest: nil))

        await #expect(throws: APIError.unauthorized) {
            try await session.deleteAccount(password: "nope")
        }

        // Still signed in, still in the family, nothing wiped: the
        // account is very much alive and the user made a typo.
        #expect(session.phase == .active)
        #expect(session.currentUser?.id == 7)
        #expect(spies.chatStoreCleared == 0)
        #expect(AppSettings.currentUserID == 7)
    }

    @Test("family_owner: the client it names gains the owner's screens at once")
    func familyOwnerFrameGrantsOwnership() {
        resetGlobals()
        defer { resetGlobals() }
        let (session, _) = makeSession()
        session.apply(me: MeResponse(user: Self.user, family: Self.family, role: "member", pendingJoinRequest: nil))
        #expect(!session.isOwner)

        session.applyFamilyOwner(userID: 7)
        #expect(session.isOwner)

        // Somebody else got it: back to being an ordinary member.
        session.applyFamilyOwner(userID: 9)
        #expect(!session.isOwner)
        #expect(session.role == "member")
    }

    /// The case account deletion exposes: the socket of a device that is
    /// signed in elsewhere is closed with `4401`, and every reconnect is
    /// then refused with `401`. A Mac holds its socket open all day and
    /// makes no REST call of its own, so without this it would sit at
    /// "Connecting…" forever against an account that no longer exists.
    @Test("a socket refused as unauthorized returns this device to sign-in")
    func socketUnauthorizedSignsThisDeviceOut() throws {
        resetGlobals()
        defer { resetGlobals() }
        AppSettings.serverURL = URL(string: "https://socket-unauthorized.test")!
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self, NoteEntity.self,
            PendingMediaItemEntity.self,
            configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        let (session, spies) = makeSession()
        let coordinator = ChatSyncCoordinator(modelContainer: container)
        coordinator.bind(session: session)
        session.apply(me: MeResponse(user: Self.user, family: Self.family, role: "member", pendingJoinRequest: nil))
        #expect(session.phase == .active)

        coordinator.handle(event: .unauthorized)

        #expect(session.phase == .needsAuth)
        #expect(spies.chatStoreCleared == 1)
        #expect(AppSettings.currentUserID == nil)
    }

    @Test("family_owner outside a family is ignored")
    func familyOwnerFrameWithoutFamilyIsIgnored() {
        resetGlobals()
        defer { resetGlobals() }
        let (session, _) = makeSession()
        session.apply(me: MeResponse(user: Self.user, family: nil, role: nil, pendingJoinRequest: nil))

        session.applyFamilyOwner(userID: 7)

        #expect(session.role == nil)
        #expect(session.phase == .needsFamily)
    }
}
