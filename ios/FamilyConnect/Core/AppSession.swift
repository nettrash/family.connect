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
//                      Only for the LAST family: leaving one of several
//                      drops that family's data and nothing else
//                      (`removeFamily`, docs/protocol.md "Several families").
//
//  SEVERAL FAMILIES (2026-10-22): `families` is every family the account is
//  in, from `GET /me`; one of them is CURRENT, persisted per account on the
//  device, and `family`/`role` are that one's — so every screen that read
//  "the" family reads the current one. The decisions are `FamilyRules`.
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

    /// From the family LIST (docs/protocol.md, "Several families"): any
    /// family at all is the chat list; an account in none with a request
    /// still waiting — to any family — is the waiting screen; otherwise the
    /// gate, exactly as before.
    static func resolve(me: MeResponse, wasAwaitingApproval: Bool) -> ResolvedPhase {
        if !FamilyRules.families(from: me).isEmpty { return .active }
        if !FamilyRules.pendingRequests(from: me).isEmpty { return .pendingApproval }
        return .needsFamily(declined: wasAwaitingApproval)
    }

    /// Where an account goes the moment it is out of its LAST family — it
    /// left, it was removed, the family was deleted under it — with no
    /// `/me` in hand to ask: `resolve(me:)`'s rule, read off what the
    /// session already holds. Since several families an account may be in
    /// a family AND have join requests pending elsewhere (docs/protocol.md,
    /// "Several families"), so losing the last family with one still
    /// waiting is the waiting screen, not the gate — the gate would offer
    /// to create or join, and re-entering that code answers
    /// `join_request_pending`. Android (`FamilyList.status`), Windows
    /// (`Gate.Pending`) and the web route it the same way.
    static func afterLastFamily(pendingRequests: [PendingJoinRequestDTO]) -> ResolvedPhase {
        pendingRequests.isEmpty ? .needsFamily(declined: false) : .pendingApproval
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
    /// Every family this account is in, home family first, each wearing
    /// the account's own `role` in it (docs/protocol.md, "Several
    /// families"). Empty for an account in none — the gate.
    private(set) var families: [FamilyDTO] = []

    /// The family the app is showing. Persisted per account
    /// (`AppSettings.currentFamilyID(forUser:)`); falls back to the first of
    /// `families` when the stored one is gone (`FamilyRules.currentFamilyID`).
    private(set) var currentFamilyID: Int64?

    /// Whether this server predates several families (`/me` without
    /// `families`): it is then spoken to about the home family alone, by
    /// the paths it knows (`apiFamilyID`).
    private(set) var isLegacyServer = false

    /// Join requests still waiting, to any family — rows in the switcher
    /// while the account is in a family, the waiting screen while it is in
    /// none.
    private(set) var pendingJoinRequests: [PendingJoinRequestDTO] = []

    /// Invitations waiting on this account: the banner over the chat list
    /// and rows in the switcher. Kept live by the `invitation` and
    /// `invitation_removed` frames.
    private(set) var invitations: [InvitationDTO] = []

    /// `[families] member_families`: may a member start another family?
    private(set) var memberFamiliesEnabled = false

    /// The most families one account may be in (1 on an older server).
    private(set) var maxFamiliesPerAccount = 1

    /// Bumped by every invitation frame, so an owner's "Pending
    /// Invitations" list re-reads instead of going stale under them.
    private(set) var invitationsGeneration = 0

    /// The current family — what every screen that said "the family" means.
    var family: FamilyDTO? {
        families.first { $0.id == currentFamilyID }
    }

    /// The HOME family: the first of `families`, which `GET /me` always
    /// lists first (and `addFamily` only ever appends to). A chat that names
    /// no family — cached before several families, or from a server that
    /// predates them — is this one's (`FamilyRules.isListed`). Nil while no
    /// list is held, an offline launch before the first `/me` included.
    var homeFamilyID: Int64? { families.first?.id }

    /// "owner" | "member" | nil — the account's role in the CURRENT family.
    var role: String? { family?.role }

    /// The id a family request names in its path for the current family:
    /// nil — the home family's old form — on a server that predates several
    /// families (`FamilyRules.pathFamilyID`).
    var apiFamilyID: Int64? {
        FamilyRules.pathFamilyID(currentFamilyID, legacy: isLegacyServer)
    }

    /// The same for any family this device holds.
    func apiFamilyID(for familyID: Int64?) -> Int64? {
        FamilyRules.pathFamilyID(familyID, legacy: isLegacyServer)
    }

    /// Whether the switcher exists at all: two or more families, or
    /// something pending to show.
    var showsFamilySwitcher: Bool {
        FamilyRules.showsSwitcher(
            familyCount: families.count,
            pendingRequests: pendingJoinRequests.count,
            invitations: invitations.count)
    }

    var offersNewFamily: Bool {
        FamilyRules.offersNewFamily(
            memberFamiliesEnabled: memberFamiliesEnabled, families: families,
            maxFamilies: maxFamiliesPerAccount)
    }

    var offersJoinAnotherFamily: Bool {
        FamilyRules.offersJoinAnother(families: families, maxFamilies: maxFamiliesPerAccount)
    }

    /// Replace one cached family after its owner changes a setting on it —
    /// the twin of `applyProfile`, and narrow for the same reason. PATCH
    /// answers with the whole family, and nothing else re-reads it until
    /// the next resync, so a settings screen that did not write back here
    /// would keep showing the old value. The family's id says which entry;
    /// the account's role in it is kept, because a PATCH answer carries none.
    func applyFamily(_ family: FamilyDTO) {
        guard let index = families.firstIndex(where: { $0.id == family.id }) else { return }
        families[index] = family.withRole(family.role ?? families[index].role)
    }

    /// Show another family. Everything family-level follows: the chat list
    /// filters by it, the roster re-projects onto it (`onCurrentFamilyChanged`),
    /// and the choice is remembered for this account on this device.
    func switchFamily(to familyID: Int64) {
        guard families.contains(where: { $0.id == familyID }), familyID != currentFamilyID else { return }
        setCurrentFamily(familyID)
    }

    private func setCurrentFamily(_ familyID: Int64?) {
        currentFamilyID = familyID
        if let user = currentUser?.id ?? AppSettings.currentUserID {
            AppSettings.setCurrentFamilyID(familyID, forUser: user)
        }
        onCurrentFamilyChanged(familyID)
    }
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
    /// When this member agreed that their words may go to the model, nil
    /// until they have (docs/protocol.md, "Consenting to the assistant").
    ///
    /// The SERVER's answer and not this device's: it is what actually
    /// calls the model, a reinstall must not quietly re-ask and re-send,
    /// and somebody who agreed on their phone has agreed rather than
    /// agreed-on-that-phone. Replaced on every `/me`, which is step 1 of
    /// the resync, so a withdrawal from another device arrives here.
    private(set) var assistantConsentAt: Date?
    /// When this member agreed that the assistant may send a query or place
    /// name it wrote from their question to the lookup providers, nil until
    /// they have (docs/protocol.md, "Consenting to the assistant", amended
    /// 2026-10-03). The server's answer, replaced on every `/me`, for the
    /// reasons `assistantConsentAt` is.
    private(set) var assistantLookupConsentAt: Date?
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

    /// The family a parked push names, waiting for `/me` to say whether
    /// this account is in it (a cold-start tap lands before bootstrap).
    private(set) var pendingPushFamilyID: Int64?

    /// A tapped notification: open the app in ITS family (docs/protocol.md,
    /// "Several families" — Pushes), then route. A push naming a family
    /// opens there; a direct chat's names none and opens in the family on
    /// screen when that lists it, else in the first that does.
    func routePush(_ route: PushRoute, familyID: Int64?) {
        pendingPushFamilyID = familyID
        pendingPushRoute = route
        if phase == .active { resolvePendingPushFamily() }
    }

    private func resolvePendingPushFamily() {
        guard pendingPushRoute != nil || pendingPushFamilyID != nil else { return }
        var chatFamilies: [Int64] = []
        if case .chat(let chatID) = pendingPushRoute { chatFamilies = chatFamilyIDs(chatID) ?? [] }
        let target = FamilyRules.pushFamily(
            pushFamilyID: pendingPushFamilyID, chatFamilyIDs: chatFamilies,
            current: currentFamilyID, held: families.map(\.id))
        pendingPushFamilyID = nil
        if let target { switchFamily(to: target) }
    }

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
    /// Set by the app: deletes every voice recording waiting as "not sent",
    /// and throws away one still running — "everything recorded and not sent
    /// is deleted" at sign-out (#79, S2.8, S4). A closure for the reason
    /// `clearMediaOutbox` is one: a test of this phase machine must never
    /// reach the real store in Application Support.
    var clearParkedRecordings: (() -> Void)?
    /// Set by the app: forgets which video messages this device has played,
    /// and stops whatever plays (#79, S5.2: the unplayed dot is "kept per
    /// account, never sent, wiped at sign-out"). A closure for the reason
    /// the two above are.
    var clearRoundVideoPlays: (() -> Void)?
    /// The family on screen changed: the coordinator re-points the roster
    /// at it. A closure for the reason `clearChatStore` is one.
    var onCurrentFamilyChanged: (Int64?) -> Void = { _ in }
    /// Drop everything this device holds that is ONE family's: its chats
    /// (a direct chat while another held family still lists it excepted),
    /// its board, its pack, its roster memberships and their cursors.
    var dropFamilyData: (Int64) -> Void = { _ in }
    /// Rows a build from before several families cached were the home
    /// family's (it read nothing else): hand them to it, once.
    var adoptUnattributedFamilyData: (Int64) -> Void = { _ in }
    /// The id of every family a held chat is listed in, or nil when this
    /// device does not hold the chat — for a push that names no family.
    var chatFamilyIDs: (Int64) -> [Int64]? = { _ in nil }
    /// Run a resync: a family was added and its roster, board, pack and
    /// chats are to be fetched now, not at the next reconnect.
    var requestResync: () -> Void = {}

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
                // The family this account was last shown, so the list is
                // that family's chats until `/me` can say more.
                currentFamilyID = AppSettings.currentUserID.flatMap(AppSettings.currentFamilyID(forUser:))
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
        assistantConsentAt = me.assistantConsentAt
        assistantLookupConsentAt = me.assistantLookupConsentAt
        familyRegistrationEnabled = me.familyRegistrationEnabled
        familylessAccountTTLDays = me.familylessAccountTTLDays
        // The operator's half of the daily greeting. Stored rather than held
        // on this object because the settings screen draws the family's
        // switch before the first /me of a session answers (protocol.md,
        // "The daily greeting").
        AppSettings.greetingsEnabled = me.greetingsEnabled
        // Replaced wholesale on every /me, which is step 1 of the
        // documented resync — so the block list is a step-1 fact and the
        // `member_blocked` frame is a latency optimisation rather than the
        // only delivery path (protocol.md, "Blocking a member").
        applyBlockedIDs(me.blockedUserIDs)
        let wasActive = phase == .active
        let wasAwaiting = phase == .pendingApproval || AppSettings.joinPending
        // Whether this device held a family — and with it, family data —
        // before this answer: the condition under which a `/me` that lists
        // none is a loss to purge for, on either of the two family-less
        // branches below. The list as well as the phase, as Android reads
        // it (`wasInFamily || before.families.isNotEmpty()`).
        let heldFamilies = wasActive || !families.isEmpty
        // Several families (docs/protocol.md, "Several families"): the list,
        // whatever this server is — one that predates them describes its
        // one family the old way, and `FamilyRules` reads both.
        let listed = FamilyRules.families(from: me)
        isLegacyServer = FamilyRules.isLegacy(me)
        pendingJoinRequests = FamilyRules.pendingRequests(from: me)
        invitations = me.invitations
        memberFamiliesEnabled = me.memberFamiliesEnabled
        maxFamiliesPerAccount = max(1, me.maxFamiliesPerAccount)

        switch SessionLogic.resolve(me: me, wasAwaitingApproval: wasAwaiting) {
        case .active:
            // A family this device held that `/me` no longer lists is gone
            // for this account — left on another device, or removed while
            // the socket was down — and its data goes with it, alone.
            let gone = Set(families.map(\.id)).subtracting(listed.map(\.id))
            families = listed
            if let home = listed.first {
                AppSettings.adoptLegacyFamilyState(into: home.id)
                adoptUnattributedFamilyData(home.id)
            }
            for familyID in gone { forgetFamily(familyID) }
            let stored = currentFamilyID
                ?? (currentUser?.id).flatMap(AppSettings.currentFamilyID(forUser:))
            let chosen = FamilyRules.currentFamilyID(stored: stored, families: listed)
            if chosen != currentFamilyID { setCurrentFamily(chosen) }
            AppSettings.joinPending = false
            joinDeclined = false
            phase = .active
            resolvePendingPushFamily()
        case .pendingApproval:
            // No family, and a join request still waiting somewhere. Before
            // several families only an account that had never been in a
            // family could get here; now an ACTIVE one can — in A with a
            // request pending to B, removed from A while the socket was
            // down, so no `member_left` ever reached this device — and what
            // it held of A is not its to keep. Purged exactly as the gate's
            // branch below purges a kick (Android and Windows do the same).
            // Skipped here, nothing else would: A's chats, board, pack and
            // cursors sat under the waiting screen, and when B then declined,
            // the gate's branch skipped its purge too, the phase being
            // `.pendingApproval` rather than `.active` by then.
            families = []
            currentFamilyID = nil
            AppSettings.joinPending = true
            if heldFamilies { purge(.kicked) }
            phase = .pendingApproval
        case .needsFamily(let declined):
            families = []
            currentFamilyID = nil
            AppSettings.joinPending = false
            if declined { joinDeclined = true }
            if heldFamilies { purge(.kicked) }
            phase = .needsFamily
        }
    }

    /// Answer the assistant question, and hold what the server answers
    /// back (docs/protocol.md, "Consenting to the assistant").
    ///
    /// Here rather than in each screen because the state is one fact about
    /// this member and three surfaces read it — the phone's composer and
    /// settings, and the Mac's. The server's own stamp is stored rather
    /// than `Date.now`: granting twice keeps the first one, and a client
    /// that invented its own would show a date the server would not.
    func setAssistantConsent(_ granted: Bool) async throws {
        assistantConsentAt = try await api.setAssistantConsent(granted)
        // Withdrawing the assistant consent withdraws the lookup consent
        // with it, on the server (protocol.md) — mirrored here so Settings
        // does not keep showing "Stop Lookups" until the next /me.
        if assistantConsentAt == nil { assistantLookupConsentAt = nil }
    }

    /// The second consent, on its own: Settings' "Stop Lookups", and the
    /// lookup screen of somebody who already agreed to the assistant.
    func setAssistantLookupConsent(_ granted: Bool) async throws {
        assistantLookupConsentAt = try await api.setAssistantLookupConsent(granted)
    }

    /// Record what a member chose on the consent screen, one server write
    /// per step in the order `AssistantConsent.steps` gives — the
    /// assistant's first, because the server refuses the lookup consent
    /// without it. Throws on the first write that fails, leaving whatever
    /// the earlier ones stored: granting is idempotent, so pressing the
    /// same button again finishes the job.
    ///
    /// One exception. When "Agree With Lookups" finds the server has no
    /// lookup source any more (404 — the operator took it away since the
    /// screen was drawn), the assistant consent the member gave stands and
    /// nothing is thrown: refusing to send their message over a lookup that
    /// cannot happen would be punishing them for the server's change.
    func agreeToAssistant(_ answer: AssistantConsent.Answer) async throws {
        for step in AssistantConsent.steps(for: answer) {
            switch step {
            case .assistant:
                try await setAssistantConsent(true)
            case .lookups:
                do {
                    try await setAssistantLookupConsent(true)
                } catch APIError.notFound where answer == .assistantAndLookups {
                    assistantLookupConsentAt = nil
                    AppSettings.assistantLookups = nil
                }
            }
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

    /// Create a family — the account's first, from the gate, or ANOTHER
    /// one from "New Family…" (docs/protocol.md, "Several families"). The
    /// creator owns it, and the app switches to it at once.
    func createFamily(name: String) async throws {
        let created = try await api.createFamily(name: name)
        let wasActive = phase == .active
        addFamily(created.withRole("owner"))
        joinDeclined = false
        phase = .active
        // Already in the connected world: the new family's roster, chat and
        // marks are fetched now (the gate's way in starts a resync of its
        // own when the phase turns active).
        if wasActive { requestResync() }
    }

    /// Hold a family the account has just entered, and show it.
    private func addFamily(_ family: FamilyDTO) {
        if let index = families.firstIndex(where: { $0.id == family.id }) {
            families[index] = family
        } else {
            families.append(family)
        }
        pendingJoinRequests.removeAll { $0.familyID == family.id }
        invitations.removeAll { $0.familyID == family.id }
        setCurrentFamily(family.id)
    }

    enum JoinOutcome: Equatable {
        case joined
        case pending
    }

    /// Join by invite code — from the gate, or "Join Another Family…" from
    /// inside the app. An account already in a family whose request now
    /// waits on an owner stays where it is, with a pending row in the
    /// switcher; one that got straight in is switched to the new family.
    @discardableResult
    func join(code: String) async throws -> JoinOutcome {
        let response = try await api.joinFamily(inviteCode: code)
        joinDeclined = false
        let wasActive = phase == .active
        let held = Set(families.map(\.id))
        if response.status == "pending" {
            if wasActive {
                // The new request is a switcher row: /me lists it.
                if let me = try? await api.me() { apply(me: me) }
                return .pending
            }
            AppSettings.joinPending = true
            phase = .pendingApproval
            return .pending
        }
        // "joined" — fetch /me for the family object + role.
        do {
            apply(me: try await api.me())
            // The family the answer names; an older server names none, and
            // the family /me lists that this device did not hold is it.
            if let joined = response.familyID ?? families.first(where: { !held.contains($0.id) })?.id {
                switchFamily(to: joined)
            }
        } catch {
            phase = .active // joined for sure; details refresh on resync
        }
        if wasActive { requestResync() }
        return .joined
    }

    // MARK: - Invitations (docs/protocol.md, "Several families")

    /// Accept: membership is immediate, and the app switches to the family.
    /// `invitation_not_pending` — accepted, declined or withdrawn meanwhile —
    /// takes the stale row away before the error reaches the screen, which
    /// says "This invitation is no longer available."
    func acceptInvitation(_ invitation: InvitationDTO) async throws {
        do {
            let family = try await api.acceptInvitation(id: invitation.id)
            invitations.removeAll { $0.id == invitation.id }
            let wasActive = phase == .active
            addFamily(family.withRole(family.role ?? "member"))
            phase = .active
            if wasActive { requestResync() }
        } catch APIError.conflict(let code, let message) where code == "invitation_not_pending" {
            invitations.removeAll { $0.id == invitation.id }
            throw APIError.conflict(code: code, message: message)
        }
    }

    /// Decline. Idempotent on the server; gone here either way.
    func declineInvitation(_ invitation: InvitationDTO) async throws {
        do {
            try await api.declineInvitation(id: invitation.id)
        } catch APIError.conflict(let code, _) where code == "invitation_not_pending" {
            // Nothing left to decline: the outcome the person asked for.
        }
        invitations.removeAll { $0.id == invitation.id }
    }

    /// The `invitation` frame: to the invited person (a row to answer) and to
    /// the family's owner (their pending list moved).
    func applyInvitation(_ invitation: InvitationDTO) {
        let me = currentUser?.id ?? AppSettings.currentUserID
        if invitation.user.id == me, !families.contains(where: { $0.id == invitation.familyID }) {
            if let index = invitations.firstIndex(where: { $0.id == invitation.id }) {
                invitations[index] = invitation
            } else {
                invitations.append(invitation)
            }
        }
        invitationsGeneration &+= 1
    }

    /// The `invitation_removed` frame: accepted, declined, withdrawn or
    /// lapsed — gone from both lists.
    func removeInvitation(id: Int64) {
        invitations.removeAll { $0.id == id }
        invitationsGeneration &+= 1
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
    ///
    /// Leaves the CURRENT family only (docs/protocol.md, "Several
    /// families"). With others left, that family's data goes and the app
    /// switches to the first remaining one; leaving the last is the old
    /// path to the gate — or to the waiting screen, while a join request
    /// is pending elsewhere (`SessionLogic.afterLastFamily`).
    ///
    /// The decision must not depend on whether this device's own
    /// `member_left` got here first. The server sends it to EVERY connection
    /// of the leaver, this one included — for an ordinary departure as it
    /// always has, and also when the leaver was the last member and the
    /// family was deleted — and it can land while this call is still
    /// waiting on its answer (the server deletes the family's attachment
    /// files before it answers). The coordinator's handler has then already
    /// removed the family (`removeFamily`). Reading `families.count > 1`
    /// after the await saw the list that echo had shrunk, so an account in
    /// two families leaving one took the "left my LAST family" branch:
    /// purged the whole store, emptied the list and went to the gate while
    /// still in the other family. So what is decided before the request is
    /// only whether the family being left is one this session lists, and
    /// what is decided after it is only what remains OTHER than that family
    /// — which is the same answer whichever of the two arrived first.
    func leaveFamily(resolveName: (Int64) -> String?) async throws -> String? {
        let leaving = currentFamilyID
        let listed = leaving.map { id in families.contains { $0.id == id } } ?? false
        let successor = try await api.leaveFamily(family: apiFamilyID)
        let name = successor.flatMap(resolveName)
        if let leaving, listed {
            // Last or not is `removeFamily`'s question, asked of what is left
            // now; a family the echo already removed is a leave already
            // carried out, and `removeFamily` does nothing a second time.
            removeFamily(leaving, reason: .leftFamily)
            return name
        }
        // No family list to consult — a session no `/me` has described
        // since several families: the one family there is, left. The old
        // path, unchanged.
        purge(.leftFamily)
        families = []
        currentFamilyID = nil
        enterFamilyless()
        return name
    }

    /// The account is out of one family — it left, or a `member_left` naming
    /// it arrived. THAT family's data goes and only that family's; when it
    /// was the one on screen, the first remaining family is shown, and when
    /// it was the last, everything goes (`purge(reason)`) and the app is
    /// routed as a family-less account is (`enterFamilyless`). A family this
    /// session does not hold — already removed — changes nothing.
    func removeFamily(_ familyID: Int64, reason: SessionLogic.PurgeReason = .kicked) {
        guard families.contains(where: { $0.id == familyID }) else { return }
        families.removeAll { $0.id == familyID }
        guard !families.isEmpty else {
            currentFamilyID = nil
            purge(reason)
            enterFamilyless()
            return
        }
        forgetFamily(familyID)
        if currentFamilyID == familyID { setCurrentFamily(families.first?.id) }
    }

    /// Out of the last family, with no `/me` in hand: the waiting screen
    /// while a join request is still pending (`SessionLogic.afterLastFamily`),
    /// else the gate. The waiting screen's own `/me` poll is what routes an
    /// approval in — the socket closes outside `.active`, so on the gate
    /// nothing would until a relaunch. The waiting bit goes up with it, as
    /// `apply(me:)` and `join(code:)` raise it: it is how a relaunch knows
    /// this account was waiting, so that a `/me` then showing neither a
    /// family nor the request is read as the decline it is.
    private func enterFamilyless() {
        switch SessionLogic.afterLastFamily(pendingRequests: pendingJoinRequests) {
        case .pendingApproval:
            AppSettings.joinPending = true
            phase = .pendingApproval
        case .needsFamily, .active:
            phase = .needsFamily
        }
    }

    /// One family's local data and marks, gone.
    private func forgetFamily(_ familyID: Int64) {
        dropFamilyData(familyID)
        AppSettings.forgetFamily(familyID)
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
        families = []
        currentFamilyID = nil
        isLegacyServer = false
        pendingJoinRequests = []
        invitations = []
        memberFamiliesEnabled = false
        maxFamiliesPerAccount = 1
        // What the server said about itself goes too: the next sign-in may
        // be to a different server, and a gate that quoted the old one's
        // grace or door until /me answered would be quoting the wrong server.
        callsEnabled = false
        videoCallsEnabled = false
        maxFamilyMembers = nil
        supportContact = nil
        assistantConsentAt = nil
        assistantLookupConsentAt = nil
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

    /// `familyID` is the family the frame names; nil is the current one
    /// (a server that predates several families names only its own). A
    /// frame about a family this account is not in changes nothing.
    func applyFamilyOwner(familyID: Int64? = nil, userID: Int64) {
        guard let target = familyID ?? currentFamilyID,
              let index = families.firstIndex(where: { $0.id == target })
        else { return }
        let me = currentUser?.id ?? AppSettings.currentUserID
        families[index] = families[index].withRole((userID == me) ? "owner" : "member")
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
            // So does a voice message that was never sent: its chat is the
            // family's, and its reply quotes the family's messages.
            clearParkedRecordings?()
            // And what this device knows it has played among them.
            clearRoundVideoPlays?()
            // Member ↔ contact links name members of the family that just
            // went — family-scoped like the roster, not defaults-scoped,
            // so they go with it on a kick or a leave as well.
            ContactLinks.shared.removeAll()
            // And every family's board and pack marks. They live in
            // UserDefaults and the rows in the store, and a kick or a leave
            // wipes the store WITHOUT wiping the defaults — so a cursor
            // that outlived its board said "caught up" over an empty one
            // and the next family's board was never read.
            AppSettings.forgetFamilies(except: [])
        }
        if scope.wipesDefaults {
            AppSettings.wipe(keepServerURL: !scope.wipesServerURL)
        }
        pendingPushRoute = nil
        pendingPushFamilyID = nil
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
